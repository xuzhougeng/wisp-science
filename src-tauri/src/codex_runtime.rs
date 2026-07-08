//! Per-project Codex CLI runtime for the `codex` tool.
//!
//! Prepares `<project>/.wisp/codex-home`, seeded from the user's `~/.codex`
//! (so auth and preferences carry over), and injects a marked `wisp_bridge`
//! MCP block into the copied `config.toml` so Codex can reach Wisp's skills,
//! bundled bio MCP, and custom MCP connections via the stdio bridge.
//!
//! Ported from the runner-as-provider work in #135 (experimental/local-runners,
//! author jarxunlai), trimmed to the codex-as-tool scope.
// ponytail: no WSL path translation here — the tool spawns codex on the host;
// port runner_env_path/to_wsl_path from experimental/local-runners if WSL
// codex setups show up.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpBridgeLaunch {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerRuntime {
    pub home_dir: PathBuf,
    pub config_path: PathBuf,
    pub env: Vec<(String, String)>,
    pub diagnostics: Vec<String>,
}

pub fn prepare_codex_runtime(
    project_root: &Path,
    bridge: &McpBridgeLaunch,
) -> Result<RunnerRuntime, String> {
    let home_dir = project_root.join(".wisp").join("codex-home");
    prepare_runtime_dir(&home_dir)?;
    let source = user_home_dir().map(|h| h.join(".codex"));
    let mut diagnostics = Vec::new();
    match source.as_deref() {
        Some(src) if src.is_dir() => sync_cli_home(src, &home_dir)?,
        Some(src) => diagnostics.push(format!(
            "Local Codex config directory not found: {}. Wisp generated a minimal CODEX_HOME.",
            src.display()
        )),
        None => diagnostics
            .push("Cannot locate user home directory; Wisp generated a minimal CODEX_HOME.".into()),
    }
    let config_path = home_dir.join("config.toml");
    inject_codex_config_block(&config_path, bridge)?;
    let env_home = home_dir.display().to_string();
    Ok(RunnerRuntime {
        home_dir,
        config_path,
        env: vec![("CODEX_HOME".into(), env_home)],
        diagnostics,
    })
}

fn prepare_runtime_dir(home_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(home_dir).map_err(|e| {
        format!(
            "Failed to create codex runtime '{}': {e}",
            home_dir.display()
        )
    })
}

fn user_home_dir() -> Option<PathBuf> {
    dirs::home_dir()
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            Some(PathBuf::from(format!(
                "{}{}",
                drive.to_string_lossy(),
                path.to_string_lossy()
            )))
        })
}

/// Copy the user's CLI config into the per-project home, skipping caches,
/// logs, and session state that must not leak between projects.
fn sync_cli_home(source: &Path, target: &Path) -> Result<(), String> {
    let skip = [
        ".wisp", "cache", ".cache", "logs", "log", "tmp", "temp", "sessions", "history",
    ]
    .into_iter()
    .collect::<HashSet<_>>();
    let entries = fs::read_dir(source).map_err(|e| {
        format!(
            "Failed to read local CLI config directory '{}': {e}",
            source.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{e}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if skip.contains(name_s.as_ref()) || name_s.ends_with(".lock") || name_s.ends_with(".log") {
            continue;
        }
        let dest = target.join(&name);
        let meta = entry.metadata().map_err(|e| format!("{e}"))?;
        if meta.is_dir() {
            if dest.exists() {
                fs::remove_dir_all(&dest).map_err(|e| {
                    format!(
                        "Failed to refresh inherited config dir '{}': {e}",
                        dest.display()
                    )
                })?;
            }
            copy_dir_recursive(&path, &dest)?;
        } else if meta.is_file() {
            fs::copy(&path, &dest).map_err(|e| {
                format!(
                    "Failed to copy inherited config '{}' to '{}': {e}",
                    path.display(),
                    dest.display()
                )
            })?;
        }
    }
    Ok(())
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| {
        format!(
            "Failed to create inherited config dir '{}': {e}",
            target.display()
        )
    })?;
    for entry in fs::read_dir(source).map_err(|e| format!("{e}"))? {
        let entry = entry.map_err(|e| format!("{e}"))?;
        let src = entry.path();
        let dst = target.join(entry.file_name());
        let meta = entry.metadata().map_err(|e| format!("{e}"))?;
        if meta.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else if meta.is_file() {
            fs::copy(&src, &dst).map_err(|e| {
                format!(
                    "Failed to copy inherited config '{}' to '{}': {e}",
                    src.display(),
                    dst.display()
                )
            })?;
        }
    }
    Ok(())
}

const WISP_BLOCK_BEGIN: &str = "# BEGIN WISP BUILTINS";
const WISP_BLOCK_END: &str = "# END WISP BUILTINS";

fn inject_codex_config_block(config_path: &Path, bridge: &McpBridgeLaunch) -> Result<(), String> {
    let existing = fs::read_to_string(config_path).unwrap_or_default();
    let block = codex_config_block(bridge);
    let updated = replace_marked_block(&existing, &block);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
    }
    let mut f = fs::File::create(config_path).map_err(|e| {
        format!(
            "Failed to write Codex runtime config '{}': {e}",
            config_path.display()
        )
    })?;
    f.write_all(updated.as_bytes()).map_err(|e| format!("{e}"))
}

/// Replace (or append) the marked Wisp block, preserving user config around it.
fn replace_marked_block(existing: &str, block: &str) -> String {
    let Some(start) = existing.find(WISP_BLOCK_BEGIN) else {
        let mut out = existing.trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(block);
        out.push('\n');
        return out;
    };
    let Some(rel_end) = existing[start..].find(WISP_BLOCK_END) else {
        let mut out = existing[..start].trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(block);
        out.push('\n');
        return out;
    };
    let end = start + rel_end + WISP_BLOCK_END.len();
    let mut out = String::new();
    out.push_str(existing[..start].trim_end());
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(block);
    out.push_str(existing[end..].trim_start_matches(['\r', '\n']));
    out
}

fn codex_config_block(bridge: &McpBridgeLaunch) -> String {
    format!(
        "{WISP_BLOCK_BEGIN}\n\
[mcp_servers.wisp_bridge]\n\
transport = \"stdio\"\n\
command = {}\n\
args = {}\n\
startup_timeout_sec = 120\n\
{WISP_BLOCK_END}",
        toml_string(&bridge.command),
        toml_string_array(&bridge.args)
    )
}

fn toml_string_array(values: &[String]) -> String {
    let inner = values
        .iter()
        .map(|s| toml_string(s))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

fn toml_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_config_block_preserves_user_config_and_replaces_old_block() {
        let bridge = McpBridgeLaunch {
            command: r"C:\Wisp\wisp-tauri.exe".into(),
            args: vec![
                "--wisp-mcp-bridge".into(),
                "--project-root".into(),
                r"C:\repo".into(),
            ],
        };
        let original = r#"model = "gpt-5"

# BEGIN WISP BUILTINS
[mcp_servers.wisp_bridge]
command = "old"
# END WISP BUILTINS

[profiles.default]
model = "local"
"#;
        let updated = replace_marked_block(original, &codex_config_block(&bridge));
        assert!(updated.contains(r#"model = "gpt-5""#));
        assert!(updated.contains("[profiles.default]"));
        assert!(updated.contains("transport = \"stdio\""));
        assert!(updated.contains("startup_timeout_sec = 120"));
        assert!(!updated.contains("command = \"old\""));
        assert!(updated.contains("wisp-tauri.exe"));
    }

    #[test]
    fn sync_cli_home_skips_cache_and_copies_config_assets() {
        let base = std::env::temp_dir().join(format!(
            "wisp-codex-rt-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let src = base.join("src");
        let dst = base.join("dst");
        std::fs::create_dir_all(src.join("skills").join("s")).unwrap();
        std::fs::create_dir_all(src.join("cache")).unwrap();
        std::fs::write(src.join("config.toml"), "model = 'x'").unwrap();
        std::fs::write(src.join("auth.json"), "{}").unwrap();
        std::fs::write(src.join("skills").join("s").join("SKILL.md"), "body").unwrap();
        std::fs::write(src.join("cache").join("stale"), "no").unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        sync_cli_home(&src, &dst).unwrap();
        assert!(dst.join("config.toml").is_file());
        assert!(dst.join("auth.json").is_file());
        assert!(dst.join("skills").join("s").join("SKILL.md").is_file());
        assert!(!dst.join("cache").exists());
        let _ = std::fs::remove_dir_all(base);
    }
}
