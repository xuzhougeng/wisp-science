use super::AppState;
use base64::Engine;
use serde::Serialize;
use std::path::Path;
use tauri::{State, WebviewWindow};

#[derive(Serialize, Clone)]
pub(super) struct DirEntry {
    name: String,
    is_dir: bool,
    size: u64,
}

#[derive(Serialize, Clone)]
pub(super) struct FileContent {
    path: String,
    mime: String,
    text: Option<String>,
    /// Base64 payload for binary files (images, pdf, pdb, …).
    base64: Option<String>,
}

#[derive(Serialize, Clone)]
pub(super) struct FileSearchHit {
    path: String,
    name: String,
    is_dir: bool,
    size: u64,
}

pub(super) fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("csv") => "text/csv",
        Some("tsv") => "text/tab-separated-values",
        Some("json") => "application/json",
        Some("md") => "text/markdown",
        Some("fasta" | "fa") => "text/x-fasta",
        Some("pdb") | Some("mol2") | Some("cif") => "chemical/x-pdb",
        Some("sdf" | "mol") => "chemical/x-mdl-molfile",
        _ => "application/octet-stream",
    }
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/") || mime == "application/json" || mime == "text/markdown"
}

/// Skip bulky or hidden trees during project-wide filename search.
fn search_skip_dir(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules" | "target" | "__pycache__" | ".git" | "dist" | "build"
        )
}

fn collect_file_search_hits(
    root: &Path,
    rel_base: &str,
    query: &str,
    limit: usize,
    out: &mut Vec<FileSearchHit>,
) -> Result<(), String> {
    if out.len() >= limit {
        return Ok(());
    }
    let dir = wisp_tools::safety::resolve_under_root(root, rel_base)?;
    if !dir.is_dir() {
        return Ok(());
    }
    let q = query.to_lowercase();
    for ent in std::fs::read_dir(&dir).map_err(|e| format!("{e}"))? {
        if out.len() >= limit {
            break;
        }
        let ent = ent.map_err(|e| format!("{e}"))?;
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let meta = ent.metadata().map_err(|e| format!("{e}"))?;
        let is_dir = meta.is_dir();
        let rel = if rel_base == "." {
            name.clone()
        } else {
            format!("{rel_base}/{name}")
        };
        if name.to_lowercase().contains(&q) {
            out.push(FileSearchHit {
                path: rel.clone(),
                name: name.clone(),
                is_dir,
                size: meta.len(),
            });
        }
        if is_dir && !search_skip_dir(&name) {
            collect_file_search_hits(root, &rel, query, limit, out)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub(super) fn search_files(
    state: State<'_, AppState>,
    window: WebviewWindow,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<FileSearchHit>, String> {
    let ap = state.active(window.label());
    let q = query.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let cap = limit.unwrap_or(200).clamp(1, 500);
    let mut hits = Vec::new();
    collect_file_search_hits(&ap.root, ".", q, cap, &mut hits)?;
    hits.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.path.cmp(&b.path))
    });
    Ok(hits)
}

#[tauri::command]
pub(super) fn list_dir(
    state: State<'_, AppState>,
    window: WebviewWindow,
    path: Option<String>,
) -> Result<Vec<DirEntry>, String> {
    let ap = state.active(window.label());
    let rel = path.unwrap_or_else(|| ".".into());
    let dir = wisp_tools::safety::resolve_under_root(&ap.root, &rel)?;
    if !dir.is_dir() {
        return Err(format!("'{}' is not a directory", rel));
    }
    let mut entries = vec![];
    for ent in std::fs::read_dir(&dir).map_err(|e| format!("{e}"))? {
        let ent = ent.map_err(|e| format!("{e}"))?;
        let meta = ent.metadata().map_err(|e| format!("{e}"))?;
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        entries.push(DirEntry {
            name,
            is_dir: meta.is_dir(),
            size: meta.len(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

pub(super) fn read_file_at(
    root: &Path,
    path: String,
    max_bytes: Option<u64>,
) -> Result<FileContent, String> {
    let real = wisp_tools::safety::validate_file_path(root, &path)?;
    let mime = mime_for_path(&real);
    let cap = max_bytes.unwrap_or(8 * 1024 * 1024).min(32 * 1024 * 1024);
    // Size check before the read: the old order slurped a file of any size
    // into memory just to reject it.
    let len = std::fs::metadata(&real).map_err(|e| format!("{e}"))?.len();
    if len > cap {
        return Err(format!("file exceeds {cap} byte limit"));
    }
    let bytes = std::fs::read(&real).map_err(|e| format!("{e}"))?;
    let path_str = real.to_string_lossy().into_owned();
    if is_text_mime(mime)
        || mime == "text/csv"
        || mime == "text/tab-separated-values"
        || mime == "text/x-fasta"
        || mime == "chemical/x-pdb"
        || mime == "chemical/x-mdl-molfile"
    {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Ok(FileContent {
            path: path_str,
            mime: mime.into(),
            text: Some(text),
            base64: None,
        })
    } else {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(FileContent {
            path: path_str,
            mime: mime.into(),
            text: None,
            base64: Some(b64),
        })
    }
}

#[tauri::command]
pub(super) fn read_file(
    state: State<'_, AppState>,
    window: WebviewWindow,
    path: String,
    max_bytes: Option<u64>,
) -> Result<FileContent, String> {
    read_file_at(&state.active(window.label()).root, path, max_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn collect_file_search_hits_matches_by_name_across_dirs() {
        let base = std::env::temp_dir().join(format!(
            "wisp_search_files_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let up = base.join("up");
        let down = base.join("down");
        std::fs::create_dir_all(&up).unwrap();
        std::fs::create_dir_all(&down).unwrap();
        std::fs::write(up.join("barplot.pdf"), b"pdf").unwrap();
        std::fs::write(down.join("barplot.pdf"), b"pdf2").unwrap();
        std::fs::write(base.join("notes.txt"), b"txt").unwrap();

        let mut hits = Vec::new();
        collect_file_search_hits(&base, ".", "barplot", 50, &mut hits).unwrap();
        assert_eq!(hits.len(), 2);
        let paths: HashSet<_> = hits.iter().map(|h| h.path.as_str()).collect();
        assert!(paths.contains("up/barplot.pdf"));
        assert!(paths.contains("down/barplot.pdf"));

        hits.clear();
        collect_file_search_hits(&base, ".", "notes", 50, &mut hits).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "notes.txt");

        let _ = std::fs::remove_dir_all(&base);
    }
}
