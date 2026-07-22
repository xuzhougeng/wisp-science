//! Resolve bundled asset directories in dev (repo root) and release (Tauri resources).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static RESOURCE_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Set the install resource root (Tauri `resource_dir` in release builds).
pub fn set_resource_root(root: PathBuf) {
    let _ = RESOURCE_ROOT.set(normalize_resource_root(root));
}

/// Prefer the resource layout that actually contains bundled assets.
///
/// Tauri map-form `resources` (current `tauri.conf.json`) place `skills/` at the
/// resource root. Older list-form `../` entries landed under `_up_/`. Some
/// Windows upgrades leave a stale `_up_/` beside a newer top-level tree; always
/// prefer the top-level catalog when it exists so skills like `local-env-setup`
/// are not hidden behind an outdated `_up_/skills`.
pub fn normalize_resource_root(root: PathBuf) -> PathBuf {
    if root.join("skills").is_dir() {
        return root;
    }
    let up = root.join("_up_");
    if up.join("skills").is_dir() {
        up
    } else {
        root
    }
}

fn dev_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Root directory containing bundled `skills/`, `python/`, `r/`, etc.
pub fn resource_root() -> PathBuf {
    RESOURCE_ROOT.get().cloned().unwrap_or_else(dev_repo_root)
}

fn existing_dir(base: &Path, rel: &str) -> Option<PathBuf> {
    let p = base.join(rel);
    p.is_dir().then_some(p)
}

pub fn skills_dir() -> Option<PathBuf> {
    existing_dir(&resource_root(), "skills")
}

pub fn python_dir() -> Option<PathBuf> {
    existing_dir(&resource_root(), "python")
}

pub fn r_dir() -> Option<PathBuf> {
    existing_dir(&resource_root(), "r")
}

pub fn bio_tools_dir() -> Option<PathBuf> {
    existing_dir(&resource_root(), "mcp-servers/bio-tools")
}

pub fn seed_dir() -> Option<PathBuf> {
    existing_dir(&resource_root(), "seed")
}

pub fn browser_extension_dir() -> Option<PathBuf> {
    existing_dir(&resource_root(), "browser-extension")
}

pub fn kernel_worker_path() -> Option<PathBuf> {
    python_dir()
        .map(|d| d.join("kernel_worker.py"))
        .filter(|p| p.is_file())
}

pub fn r_kernel_worker_path() -> Option<PathBuf> {
    r_dir()
        .map(|d| d.join("kernel_worker.R"))
        .filter(|p| p.is_file())
}

pub fn mcp_requirements_path() -> Option<PathBuf> {
    python_dir()
        .map(|d| d.join("requirements-mcp.txt"))
        .filter(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_tree_has_bundled_assets() {
        assert!(skills_dir().is_some());
        assert!(python_dir().is_some());
        assert!(r_dir().is_some());
        assert!(r_kernel_worker_path().is_some());
        assert!(bio_tools_dir().is_some());
        assert!(seed_dir().is_some());
        assert!(browser_extension_dir().is_some());
    }

    #[test]
    fn normalize_up_resource_root() {
        let tmp = std::env::temp_dir().join(format!("wisp-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        // Legacy list-form: only `_up_/skills`.
        std::fs::create_dir_all(tmp.join("_up_/skills")).unwrap();
        assert_eq!(normalize_resource_root(tmp.clone()), tmp.join("_up_"));
        // Map-form: top-level `skills/` wins even if a stale `_up_/` remains.
        std::fs::create_dir_all(tmp.join("skills")).unwrap();
        assert_eq!(normalize_resource_root(tmp.clone()), tmp);
        let flat = tmp.join("flat");
        std::fs::create_dir_all(flat.join("skills")).unwrap();
        assert_eq!(normalize_resource_root(flat.clone()), flat);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
