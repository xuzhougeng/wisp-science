//! Where a packaged build finds the files that ship beside it.
//!
//! Skills, the Python and R workers, the browser extension and the example
//! conversations are folders on disk, read through `wisp-paths`. Run from a
//! checkout they are the repository's own. A release package carries a copy
//! (`scripts/package_gpui.py`), and this points `wisp-paths` at it.

use std::path::{Path, PathBuf};

/// The folder a packaged executable's `skills/`, `python/`, `r/`, `seed/` and
/// `browser-extension/` are in, or `None` when nothing was packed beside it.
///
/// - macOS: `<name>.app/Contents/MacOS/wisp-gpui` reads `Contents/Resources`.
/// - Windows and Linux: `resources` in the executable's own folder.
///
/// A folder counts only when it holds `skills/`, so an executable built in a
/// checkout keeps reading the repository.
pub fn bundled_root(executable: &Path, macos: bool) -> Option<PathBuf> {
    let folder = executable.parent()?;
    let bundle = macos
        .then(|| folder.parent().map(|contents| contents.join("Resources")))
        .flatten();
    bundle
        .into_iter()
        .chain([folder.join("resources")])
        .find(|root| root.join("skills").is_dir())
}

/// Point `wisp-paths` at this executable's packaged resources, if it has any.
/// Call once, before the host starts.
pub fn install() {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    // A link to the executable must still find the package it points into.
    // Windows is left as given: resolving there yields `\\?\` paths, which
    // the interpreters these folders are handed to do not all accept.
    let executable = if cfg!(target_os = "windows") {
        executable
    } else {
        std::fs::canonicalize(&executable).unwrap_or(executable)
    };
    if let Some(root) = bundled_root(&executable, cfg!(target_os = "macos")) {
        wisp_paths::set_resource_root(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("wisp-gpui-resources-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn a_macos_bundle_reads_contents_resources() {
        let root = scratch("bundle");
        let app = root.join("wisp-science-gpui.app/Contents");
        std::fs::create_dir_all(app.join("MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Resources/skills")).unwrap();
        let executable = app.join("MacOS/wisp-gpui");
        assert_eq!(bundled_root(&executable, true), Some(app.join("Resources")));
        // The bundle layout means nothing on the other systems.
        assert_eq!(bundled_root(&executable, false), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unpacked_archive_reads_resources_beside_the_executable() {
        let root = scratch("archive");
        std::fs::create_dir_all(root.join("resources/skills")).unwrap();
        for (name, macos) in [
            ("wisp-gpui.exe", false),
            ("wisp-gpui", false),
            ("wisp-gpui", true),
        ] {
            assert_eq!(
                bundled_root(&root.join(name), macos),
                Some(root.join("resources")),
                "{name}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_checkout_build_keeps_the_repository() {
        let root = scratch("checkout");
        // `resources` without skills is not a package: target/release may
        // hold any folder.
        std::fs::create_dir_all(root.join("target/release/resources")).unwrap();
        std::fs::create_dir_all(root.join("target/Resources")).unwrap();
        let executable = root.join("target/release/wisp-gpui");
        assert_eq!(bundled_root(&executable, true), None);
        assert_eq!(bundled_root(&executable, false), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
