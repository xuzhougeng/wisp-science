//! uv-managed Python environment provisioning.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A uv-created virtualenv that hosts the Wisp kernel worker.
pub struct PythonEnv {
    pub venv: PathBuf,
}

impl PythonEnv {
    /// Locate `uv` on PATH (or via `UV_PATH` env).
    pub fn find_uv() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("UV_PATH") {
            return Some(PathBuf::from(p));
        }
        which::which("uv").ok()
    }

    /// Locate `node` on PATH.
    pub fn find_node() -> Option<PathBuf> {
        which::which("node").ok()
    }

    /// Locate `npm` on PATH.
    pub fn find_npm() -> Option<PathBuf> {
        which::which("npm").ok()
    }

    /// Locate `sci` (scimaster-cli) on PATH.
    pub fn find_sci() -> Option<PathBuf> {
        which::which("sci").ok()
    }

    /// Locate `pixi` on PATH (or via `PIXI_PATH` env).
    pub fn find_pixi() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("PIXI_PATH") {
            return Some(PathBuf::from(p));
        }
        which::which("pixi").ok()
    }

    /// Python interpreter inside the venv (`Scripts\python.exe` on Windows).
    pub fn python(&self) -> PathBuf {
        if cfg!(target_os = "windows") {
            self.venv.join("Scripts").join("python.exe")
        } else {
            self.venv.join("bin").join("python")
        }
    }

    /// Ensure a venv exists under `app_data/python/.venv`, create with `uv venv`,
    /// and install MCP/kernel deps from the bundled requirements file when needed.
    pub fn ensure(app_data: &Path) -> Result<Self> {
        let venv = app_data.join("python").join(".venv");
        let python = if cfg!(target_os = "windows") {
            venv.join("Scripts").join("python.exe")
        } else {
            venv.join("bin").join("python")
        };
        let uv = Self::find_uv().ok_or_else(|| anyhow!("uv not found on PATH; install uv or set UV_PATH"))?;
        if !python.exists() {
            std::fs::create_dir_all(venv.parent().unwrap_or(Path::new(".")))?;
            let mut cmd = Command::new(&uv);
            cmd.arg("venv").arg(&venv);
            wisp_tools::process::hide_console(&mut cmd);
            let out = cmd.output()?;
            if !out.status.success() {
                return Err(anyhow!("uv venv failed: {}", String::from_utf8_lossy(&out.stderr)));
            }
        }
        Self::install_deps(&uv, &python, &venv)?;
        Ok(Self { venv })
    }

    fn install_deps(uv: &Path, python: &Path, venv: &Path) -> Result<()> {
        let Some(req) = wisp_paths::mcp_requirements_path() else {
            return Ok(());
        };
        let marker = venv.join(".wisp_deps_ok");
        if marker.is_file() {
            return Ok(());
        }
        let mut cmd = Command::new(uv);
        cmd.args(["pip", "install", "-r"])
            .arg(&req)
            .arg("--python")
            .arg(python);
        wisp_tools::process::hide_console(&mut cmd);
        let out = cmd.output()?;
        if !out.status.success() {
            return Err(anyhow!(
                "uv pip install failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        std::fs::write(&marker, b"ok")?;
        Ok(())
    }
}

/// Path to the kernel worker bundled with the app (`python/kernel_worker.py`).
pub fn bundled_worker_path() -> Option<PathBuf> {
    wisp_paths::kernel_worker_path()
}

/// Path to the mock MCP server bundled with the app.
pub fn bundled_mock_mcp_path() -> Option<PathBuf> {
    wisp_paths::python_dir()
        .map(|d| d.join("mock_mcp_server.py"))
        .filter(|p| p.is_file())
}

/// Resolve a script path, remapping stale locations to bundled `python/` when missing.
pub fn resolve_bundled_script(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_file() {
        return p;
    }
    match p.file_name().and_then(|n| n.to_str()) {
        Some("kernel_worker.py") => bundled_worker_path().unwrap_or(p),
        Some("mock_mcp_server.py") => bundled_mock_mcp_path().unwrap_or(p),
        _ => p,
    }
}
