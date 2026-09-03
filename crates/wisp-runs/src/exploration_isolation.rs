//! Fail-closed local path boundary for exploration conversations.
//!
//! Dedicated file tools already resolve writes under `ToolEnv::project_root`,
//! but free-form shells and language runtimes can name the live mainline with
//! an absolute path. Explorations keep their own materialized workspace, so a
//! literal reference to the stored mainline root must never reach a local
//! interpreter.

use std::path::{Path, PathBuf};
use wisp_store::{StateScope, Store};

const ERR_SCOPE_VIOLATION: &str = "exploration_scope_violation";

#[derive(Clone, Debug)]
pub struct ExplorationIsolationBoundary {
    mainline_roots: Vec<NormalizedRoot>,
}

#[derive(Clone, Debug)]
struct NormalizedRoot {
    display: String,
    value: String,
    windows: bool,
}

impl ExplorationIsolationBoundary {
    pub fn new(mainline_root: &Path) -> Self {
        let mut roots = vec![mainline_root.to_path_buf()];
        if let Some(wsl_root) = wsl_mount_alias(&mainline_root.to_string_lossy()) {
            roots.push(wsl_root);
        }
        if let Ok(canonical) = dunce::canonicalize(mainline_root) {
            if canonical != mainline_root {
                roots.push(canonical);
            }
        }
        Self::from_roots(roots)
    }

    fn from_roots(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut mainline_roots = Vec::new();
        for root in roots {
            let display = root.to_string_lossy().into_owned();
            let windows = is_windows_path(&display);
            let value = normalize_path_text(&display, windows);
            if value.is_empty()
                || mainline_roots
                    .iter()
                    .any(|existing: &NormalizedRoot| existing.value == value)
            {
                continue;
            }
            mainline_roots.push(NormalizedRoot {
                display,
                value,
                windows,
            });
        }
        Self { mainline_roots }
    }

    pub fn check_local_source(&self, source: &str) -> Result<(), String> {
        for root in &self.mainline_roots {
            let normalized = normalize_path_text(source, root.windows);
            if contains_path(&normalized, &root.value, root.windows) {
                return Err(format!(
                    "{ERR_SCOPE_VIOLATION}: local execution references the live mainline workspace '{}'. Use paths relative to the isolated exploration workspace instead.",
                    root.display
                ));
            }
        }
        Ok(())
    }
}

pub fn is_host_local_context(context_id: &str) -> bool {
    context_id == "local" || context_id.starts_with("wsl:")
}

pub async fn boundary_for_scope(
    store: &Store,
    scope: &StateScope,
) -> Result<Option<ExplorationIsolationBoundary>, String> {
    let StateScope::Exploration { project_id, .. } = scope else {
        return Ok(None);
    };
    let (_, workspace_dir) = store
        .get_project(project_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Project not found".to_string())?;
    Ok(Some(ExplorationIsolationBoundary::new(Path::new(
        &workspace_dir,
    ))))
}

fn is_windows_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    (bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/'))
        || value.starts_with("\\\\")
        || value.starts_with("//")
}

fn wsl_mount_alias(value: &str) -> Option<PathBuf> {
    let bytes = value.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
    {
        return None;
    }
    let drive = char::from(bytes[0]).to_ascii_lowercase();
    let rest = value[3..].replace('\\', "/");
    Some(PathBuf::from(format!(
        "/mnt/{drive}{}{}",
        if rest.is_empty() { "" } else { "/" },
        rest
    )))
}

fn normalize_path_text(value: &str, windows: bool) -> String {
    let separator = if windows { '\\' } else { '/' };
    let mut normalized = String::with_capacity(value.len());
    let mut previous_separator = false;
    for character in value.chars() {
        let is_separator = if windows {
            matches!(character, '\\' | '/')
        } else {
            character == '/'
        };
        if is_separator {
            if !previous_separator {
                normalized.push(separator);
            }
        } else {
            normalized.push(if windows {
                character.to_ascii_lowercase()
            } else {
                character
            });
        }
        previous_separator = is_separator;
    }
    while normalized.len() > 1 && normalized.ends_with(separator) {
        normalized.pop();
    }
    normalized
}

fn contains_path(source: &str, root: &str, windows: bool) -> bool {
    let separator = if windows { '\\' } else { '/' };
    source.match_indices(root).any(|(start, matched)| {
        let before = source[..start].chars().next_back();
        let after = source[start + matched.len()..].chars().next();
        path_start_boundary(before, windows) && path_end_boundary(after, separator)
    })
}

fn path_start_boundary(character: Option<char>, windows: bool) -> bool {
    match character {
        None => true,
        Some(character) if windows => {
            !character.is_ascii_alphanumeric() && !matches!(character, '_' | ':' | '.')
        }
        Some(character) => !character.is_ascii_alphanumeric() && !matches!(character, '_' | '.'),
    }
}

fn path_end_boundary(character: Option<char>, separator: char) -> bool {
    match character {
        None => true,
        Some(character) if character == separator => true,
        Some(character) => {
            !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-' | '.')
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_mainline_references_are_case_and_separator_insensitive() {
        let boundary = ExplorationIsolationBoundary::new(Path::new(r"D:\0906"));
        for source in [
            r"Rscript 'D:\0906\analysis\script.R'",
            r#"source("d:/0906/analysis/script.R")"#,
            r#"read.csv("D:\\0906\\data.csv")"#,
            r"Set-Location D:/0906",
            r"Rscript /mnt/d/0906/analysis/script.R",
        ] {
            assert!(boundary.check_local_source(source).is_err(), "{source}");
        }
    }

    #[test]
    fn sibling_names_and_unrelated_interpreters_remain_allowed() {
        let boundary = ExplorationIsolationBoundary::from_roots([PathBuf::from(r"D:\0906")]);
        for source in [
            r"Rscript analysis/script.R",
            r"C:\Program Files\R\R-4.5.0\bin\Rscript.exe analysis\script.R",
            r"Get-Content D:\09060\result.txt",
            r"echo prefixD:\0906\result.txt",
        ] {
            assert!(boundary.check_local_source(source).is_ok(), "{source}");
        }
    }

    #[test]
    fn posix_mainline_reference_is_case_sensitive_and_path_bounded() {
        let boundary =
            ExplorationIsolationBoundary::from_roots([PathBuf::from("/Users/research/Project")]);
        assert!(boundary
            .check_local_source("python /Users/research/Project/run.py")
            .is_err());
        assert!(boundary
            .check_local_source("python /Users/research/project/run.py")
            .is_ok());
        assert!(boundary
            .check_local_source("python /Users/research/Project-old/run.py")
            .is_ok());
    }
}
