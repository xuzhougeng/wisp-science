use super::AppState;
use base64::Engine;
use serde::Serialize;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use tauri::{ipc::Response, State, WebviewWindow};

const REMOTE_DIR_PROTOCOL: &[u8] = b"WISP_REMOTE_DIR_V1\0";
const REMOTE_FILE_PROTOCOL: &[u8] = b"WISP_REMOTE_FILE_V1\0";
/// Text-oriented remote reads: size + truncated head so multi-GB logs never
/// cross the wire just for a UI preview.
const REMOTE_FILE_TEXT_PROTOCOL: &[u8] = b"WISP_REMOTE_FILE_TEXT_V1\0";
/// Remote previews stay capped at 32 MB: the bytes cross an SSH connection.
const REMOTE_FILE_MAX_BYTES: u64 = 32 * 1024 * 1024;
/// Local previews match the 100 MB upload cap — a 38 MB journal PDF is routine
/// and pdf.js renders one page at a time, so memory stays bounded (#485).
const LOCAL_FILE_MAX_BYTES: u64 = 100 * 1024 * 1024;
const DEFAULT_FILE_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Default budget for text-ish `read_file` / artifact previews when the caller
/// omits `maxBytes`. Head-only; never reject oversized text solely for size.
const DEFAULT_TEXT_PREVIEW_BYTES: u64 = 1024 * 1024;
const OOXML_MAX_ENTRIES: usize = 4096;
const OOXML_MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
const OOXML_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const OOXML_MAX_COMPRESSION_RATIO: u64 = 100;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub(super) struct DirEntry {
    name: String,
    is_dir: bool,
    size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_unix_millis: Option<u64>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub(super) struct DirectoryListing {
    path: String,
    entries: Vec<DirEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteCommand {
    context_id: String,
    program: String,
    args: Vec<String>,
    envs: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteOutput {
    status: i32,
    stdout: Vec<u8>,
    stderr: String,
}

trait RemoteRunner: Send {
    fn run(&mut self, command: &RemoteCommand) -> Result<RemoteOutput, String>;
}

struct ProcessRemoteRunner;

impl RemoteRunner for ProcessRemoteRunner {
    fn run(&mut self, command: &RemoteCommand) -> Result<RemoteOutput, String> {
        if let Some(payload) =
            crate::ssh_master::eligible_payload(&command.program, &command.args, None)
        {
            let ssh_args = command.args[..command.args.len() - 1].to_vec();
            let result = crate::ssh_master::run_blocking(
                &command.context_id,
                ssh_args,
                &command.envs,
                payload,
                std::time::Duration::from_secs(120),
            )
            .map(|output| RemoteOutput {
                status: output.exit_code as i32,
                stdout: output.stdout,
                stderr: output.stderr,
            });
            crate::ssh_hosts::cleanup_password_auth_env(&command.envs);
            return result;
        }
        let mut process = std::process::Command::new(&command.program);
        process.args(&command.args);
        if !command.envs.is_empty() {
            process.envs(command.envs.iter().cloned());
        }
        wisp_tools::process::hide_console(&mut process);
        let output = process
            .output()
            .map_err(|e| format!("failed to run {}: {e}", command.program))?;
        crate::ssh_hosts::cleanup_password_auth_env(&command.envs);
        Ok(RemoteOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

#[derive(Serialize, Clone, Debug)]
pub(super) struct FileContent {
    path: String,
    mime: String,
    text: Option<String>,
    /// Base64 payload for binary files (images, pdf, pdb, …).
    base64: Option<String>,
    /// True when only a leading prefix was returned (large text/log/CSV).
    truncated: bool,
    /// Full on-disk size when known (local metadata or remote `stat`).
    #[serde(skip_serializing_if = "Option::is_none")]
    total_bytes: Option<u64>,
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
        Some("doc" | "docm") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xls" | "xlsm" | "xlsb") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("ppt" | "pps" | "pot" | "pptm" | "ppsx" | "ppsm") => "application/vnd.ms-powerpoint",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("odt") => "application/vnd.oasis.opendocument.text",
        Some("ods") => "application/vnd.oasis.opendocument.spreadsheet",
        Some("odp") => "application/vnd.oasis.opendocument.presentation",
        Some("rtf") => "application/rtf",
        Some("epub") => "application/epub+zip",
        Some("bib") => "text/x-bibtex",
        Some("csv") => "text/csv",
        Some("tsv") => "text/tab-separated-values",
        Some("html" | "htm") => "text/html",
        Some("json") => "application/json",
        Some("ipynb") => "application/x-ipynb+json",
        Some("md") => "text/markdown",
        Some("r") => "text/x-r",
        Some("py") => "text/x-python",
        Some("sh") => "text/x-shellscript",
        Some("fasta" | "fa") => "text/x-fasta",
        Some("pdb") | Some("mol2") | Some("cif") => "chemical/x-pdb",
        Some("sdf" | "mol") => "chemical/x-mdl-molfile",
        _ => "application/octet-stream",
    }
}

fn preview_byte_cap(max_bytes: Option<u64>) -> u64 {
    max_bytes
        .unwrap_or(DEFAULT_FILE_MAX_BYTES)
        .min(LOCAL_FILE_MAX_BYTES)
}

fn text_preview_byte_cap(max_bytes: Option<u64>) -> u64 {
    max_bytes
        .unwrap_or(DEFAULT_TEXT_PREVIEW_BYTES)
        .min(LOCAL_FILE_MAX_BYTES)
}

/// MIME types that remain useful when only a leading prefix is available.
fn supports_prefix_preview(mime: &str) -> bool {
    is_text_mime(mime)
        || mime == "chemical/x-pdb"
        || mime == "chemical/x-mdl-molfile"
        || mime == "application/octet-stream"
}

/// Read at most `cap` bytes from the start of a file. Never loads the tail of a
/// multi-GB log just to discover it is text.
fn read_path_prefix(path: &Path, cap: u64) -> Result<(Vec<u8>, u64), String> {
    let total = std::fs::metadata(path).map_err(|e| format!("{e}"))?.len();
    if total == 0 || cap == 0 {
        return Ok((Vec::new(), total));
    }
    let mut file = std::fs::File::open(path).map_err(|e| format!("{e}"))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(cap.min(total))
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{e}"))?;
    Ok((bytes, total))
}

fn is_text_preview_bytes(mime: &str, bytes: &[u8]) -> bool {
    is_text_mime(mime)
        || mime == "chemical/x-pdb"
        || mime == "chemical/x-mdl-molfile"
        || (mime == "application/octet-stream" && looks_like_text(bytes))
}

fn is_ooxml_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("docx" | "xlsx" | "pptx")
    )
}

fn validate_external_relationships(xml: &str) -> Result<(), String> {
    let relationship = regex::Regex::new(r"(?is)<Relationship\b[^>]*>")
        .map_err(|error| format!("could not compile relationship validator: {error}"))?;
    let external = regex::Regex::new(r#"(?i)\bTargetMode\s*=\s*["']External["']"#)
        .map_err(|error| format!("could not compile relationship validator: {error}"))?;
    let relationship_type = regex::Regex::new(r#"(?i)\bType\s*=\s*["']([^"']+)["']"#)
        .map_err(|error| format!("could not compile relationship validator: {error}"))?;
    let target = regex::Regex::new(r#"(?i)\bTarget\s*=\s*["']([^"']+)["']"#)
        .map_err(|error| format!("could not compile relationship validator: {error}"))?;

    for tag in relationship.find_iter(xml).map(|matched| matched.as_str()) {
        if !external.is_match(tag) {
            continue;
        }
        let kind = relationship_type
            .captures(tag)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_ascii_lowercase());
        if !kind
            .as_deref()
            .is_some_and(|kind| kind.ends_with("/hyperlink"))
        {
            return Err("OOXML archive contains an external media relationship".into());
        }
        let destination = target
            .captures(tag)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str())
            .ok_or_else(|| "OOXML external hyperlink is missing its target".to_string())?;
        let scheme = destination
            .split_once(':')
            .map(|(scheme, _)| scheme.to_ascii_lowercase());
        if !matches!(scheme.as_deref(), Some("http" | "https" | "mailto")) {
            return Err("OOXML archive contains an unsafe external hyperlink".into());
        }
    }
    Ok(())
}

/// Reject OOXML archives that can expand beyond the preview budget before a
/// browser parser sees them. This examines only the central directory, so the
/// check is cheap and does not materialize any ZIP entry.
pub(super) fn validate_ooxml_archive(bytes: &[u8]) -> Result<(), String> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|error| format!("invalid OOXML archive: {error}"))?;
    if archive.len() > OOXML_MAX_ENTRIES {
        return Err(format!(
            "OOXML archive contains more than {OOXML_MAX_ENTRIES} entries"
        ));
    }

    let mut total_uncompressed = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("invalid OOXML ZIP entry: {error}"))?;
        let name = entry.name().to_string();
        if entry.enclosed_name().is_none() {
            return Err(format!("OOXML archive contains an unsafe path: {name}"));
        }
        let normalized_name = name.replace('\\', "/").to_ascii_lowercase();
        if normalized_name.starts_with('/')
            || normalized_name
                .split('/')
                .any(|component| component == "..")
            || normalized_name
                .split('/')
                .next()
                .is_some_and(|component| component.ends_with(':'))
        {
            return Err(format!("OOXML archive contains an unsafe path: {name}"));
        }
        if normalized_name.ends_with("vbaproject.bin")
            || normalized_name.contains("/activex/")
            || normalized_name.contains("/embeddings/")
        {
            return Err(format!(
                "OOXML archive contains unsupported active content: {name}"
            ));
        }

        let uncompressed = entry.size();
        let compressed = entry.compressed_size();
        if uncompressed > OOXML_MAX_ENTRY_BYTES {
            return Err(format!(
                "OOXML ZIP entry exceeds {OOXML_MAX_ENTRY_BYTES} byte limit: {name}"
            ));
        }
        total_uncompressed = total_uncompressed
            .checked_add(uncompressed)
            .ok_or_else(|| "OOXML archive expanded size overflowed".to_string())?;
        if total_uncompressed > OOXML_MAX_TOTAL_BYTES {
            return Err(format!(
                "OOXML archive exceeds {OOXML_MAX_TOTAL_BYTES} expanded byte limit"
            ));
        }
        if uncompressed > 0
            && (compressed == 0
                || uncompressed > compressed.saturating_mul(OOXML_MAX_COMPRESSION_RATIO))
        {
            return Err(format!(
                "OOXML ZIP entry exceeds {OOXML_MAX_COMPRESSION_RATIO}:1 compression ratio: {name}"
            ));
        }
        if normalized_name.ends_with(".rels") && uncompressed > 0 {
            let mut xml = String::new();
            entry
                .read_to_string(&mut xml)
                .map_err(|error| format!("could not inspect OOXML relationships: {error}"))?;
            validate_external_relationships(&xml)?;
        }
    }
    Ok(())
}

fn is_tiff_media_entry(name: &str) -> bool {
    let lower = name.replace('\\', "/").to_ascii_lowercase();
    lower.contains("/media/") && (lower.ends_with(".tif") || lower.ends_with(".tiff"))
}

/// Browsers cannot decode TIFF, so a DOCX whose embedded figures are TIFF
/// previews as text with blank image boxes. Rewrite the archive in memory:
/// TIFF media parts are transcoded to PNG under the same entry name (`<img>`
/// sniffs magic bytes, not extensions) and the content-type map is pointed at
/// image/png. Any failure returns the original bytes — a missing figure beats
/// a failed preview. Runs only on archives that actually contain TIFF media.
pub(super) fn transcode_ooxml_tiff_media(bytes: Vec<u8>) -> Vec<u8> {
    let has_tiff = zip::ZipArchive::new(Cursor::new(&bytes))
        .is_ok_and(|archive| archive.file_names().any(is_tiff_media_entry));
    if !has_tiff {
        return bytes;
    }
    match transcode_ooxml_tiff_media_inner(&bytes) {
        Ok(rewritten) => rewritten,
        Err(_) => bytes,
    }
}

fn transcode_ooxml_tiff_media_inner(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::with_capacity(bytes.len())));
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..archive.len() {
        let name = archive
            .by_index_raw(index)
            .map_err(|error| error.to_string())?
            .name()
            .to_string();
        if is_tiff_media_entry(&name) {
            let mut tiff = Vec::new();
            archive
                .by_index(index)
                .map_err(|error| error.to_string())?
                .read_to_end(&mut tiff)
                .map_err(|error| error.to_string())?;
            match tiff_to_png(&tiff) {
                Ok(png) => {
                    out.start_file(name, options).map_err(|e| e.to_string())?;
                    std::io::Write::write_all(&mut out, &png).map_err(|e| e.to_string())?;
                }
                // Undecodable TIFF (exotic colorspace/compression): keep the
                // original part so the rest of the document still previews.
                Err(_) => {
                    let raw = archive.by_index_raw(index).map_err(|e| e.to_string())?;
                    out.raw_copy_file(raw).map_err(|e| e.to_string())?;
                }
            }
        } else if name == "[Content_Types].xml" {
            let mut xml = String::new();
            archive
                .by_index(index)
                .map_err(|error| error.to_string())?
                .read_to_string(&mut xml)
                .map_err(|error| error.to_string())?;
            let patched = xml.replace("image/tiff", "image/png");
            out.start_file(name, options).map_err(|e| e.to_string())?;
            std::io::Write::write_all(&mut out, patched.as_bytes()).map_err(|e| e.to_string())?;
        } else {
            let raw = archive.by_index_raw(index).map_err(|e| e.to_string())?;
            out.raw_copy_file(raw).map_err(|e| e.to_string())?;
        }
    }
    Ok(out.finish().map_err(|e| e.to_string())?.into_inner())
}

fn tiff_to_png(tiff: &[u8]) -> Result<Vec<u8>, String> {
    let decoded = image::load_from_memory_with_format(tiff, image::ImageFormat::Tiff)
        .map_err(|error| error.to_string())?;
    let mut png = Vec::new();
    decoded
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(png)
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime == "application/json"
        || mime == "application/x-ipynb+json"
        || mime == "text/markdown"
}

/// An extension allowlist always lags reality — `.toml`, `.lock`, `.yaml`, `.R`
/// and friends previewed as "unsupported" purely because nothing named them.
/// For anything without an explicit mime, let the bytes decide instead.
fn looks_like_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
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
    let ap = state.require_active(window.label())?;
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
    let ap = state.require_active(window.label())?;
    let rel = path.unwrap_or_else(|| ".".into());
    let dir = wisp_tools::safety::resolve_under_root(&ap.root, &rel)?;
    if !dir.is_dir() {
        return Err(format!("'{}' is not a directory", rel));
    }
    list_dir_entries(&dir)
}

fn modified_unix_millis(metadata: &std::fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
}

fn list_dir_entries(dir: &Path) -> Result<Vec<DirEntry>, String> {
    let mut entries = vec![];
    for ent in std::fs::read_dir(dir).map_err(|e| format!("{e}"))? {
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
            modified_unix_millis: modified_unix_millis(&meta),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// Resolve the parent of a workspace entry before appending its final name.
/// Unlike `resolve_under_root`, this also works for a target that does not exist
/// yet and deliberately does not follow a symlink in the final component. That
/// lets rename/delete operate on the link itself while every parent still has
/// to resolve inside the project root.
fn workspace_entry_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("workspace path must not be empty".into());
    }
    let requested = Path::new(path);
    let absolute = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let name = absolute
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("path '{path}' does not name a workspace entry"))?;
    let parent = absolute
        .parent()
        .ok_or_else(|| format!("path '{path}' has no parent directory"))?;
    let parent = wisp_tools::safety::resolve_under_root(root, &parent.to_string_lossy())?;
    if !parent.is_dir() {
        return Err(format!("parent of '{path}' is not a directory"));
    }
    let target = parent.join(name);
    let root = wisp_tools::safety::resolve_under_root(root, ".")?;
    if target == root {
        return Err("the project root cannot be changed".into());
    }
    Ok(target)
}

pub(super) fn create_file_at(root: &Path, path: &str) -> Result<(), String> {
    let target = workspace_entry_path(root, path)?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map(|_| ())
        .map_err(|error| format!("could not create file '{path}': {error}"))
}

async fn writable_active_project(
    state: &AppState,
    window_label: &str,
) -> Result<
    (
        crate::ActiveProject,
        wisp_store::StateScope,
        tokio::sync::OwnedRwLockReadGuard<()>,
    ),
    String,
> {
    let (project, scope) =
        crate::exploration_commands::working_project_for_active_frame(state, window_label).await?;
    let activity = state.begin_project_activity(&project.id)?;
    crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
    Ok((project, scope, activity))
}

#[tauri::command]
pub(super) async fn create_file(
    state: State<'_, AppState>,
    window: WebviewWindow,
    path: String,
) -> Result<(), String> {
    let (project, scope, _activity) = writable_active_project(&state, window.label()).await?;
    create_file_at(&project.root, &path)?;
    state
        .store
        .bump_state_generation(&scope)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Byte ceiling for a user-driven editor save. The center editor refuses to
/// edit files it could not load in full, so anything larger is a bug or abuse.
const SAVE_FILE_MAX_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn save_file_at(root: &Path, path: &str, content: &str) -> Result<(), String> {
    if content.len() > SAVE_FILE_MAX_BYTES {
        return Err(format!(
            "file exceeds {SAVE_FILE_MAX_BYTES} byte save limit"
        ));
    }
    // Existing files only: the editor edits what a preview loaded, and file
    // creation stays with `create_file` / the agent's write tool.
    let real = wisp_tools::safety::validate_file_path(root, path)?;
    std::fs::write(&real, content).map_err(|error| format!("could not save '{path}': {error}"))
}

/// Persist an edited center-preview source file. User-driven like
/// `execute_runtime`: the user is looking at the content they typed, so this
/// deliberately does not route through agent tool approval.
#[tauri::command]
pub(super) async fn save_file(
    state: State<'_, AppState>,
    window: WebviewWindow,
    path: String,
    content: String,
) -> Result<(), String> {
    let (project, scope, _activity) = writable_active_project(&state, window.label()).await?;
    save_file_at(&project.root, &path, &content)?;
    state
        .store
        .bump_state_generation(&scope)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn create_directory_at(root: &Path, path: &str) -> Result<(), String> {
    let target = workspace_entry_path(root, path)?;
    std::fs::create_dir(&target)
        .map_err(|error| format!("could not create directory '{path}': {error}"))
}

#[tauri::command]
pub(super) async fn create_directory(
    state: State<'_, AppState>,
    window: WebviewWindow,
    path: String,
) -> Result<(), String> {
    let (project, scope, _activity) = writable_active_project(&state, window.label()).await?;
    create_directory_at(&project.root, &path)?;
    state
        .store
        .bump_state_generation(&scope)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn rename_entry_at(root: &Path, path: &str, new_path: &str) -> Result<(), String> {
    let source = workspace_entry_path(root, path)?;
    std::fs::symlink_metadata(&source)
        .map_err(|error| format!("workspace entry '{path}' was not found: {error}"))?;
    let target = workspace_entry_path(root, new_path)?;
    if source == target {
        return Ok(());
    }
    if target.exists() || std::fs::symlink_metadata(&target).is_ok() {
        let same_entry = matches!(
            (
                std::fs::canonicalize(&source),
                std::fs::canonicalize(&target)
            ),
            (Ok(source), Ok(target)) if source == target
        );
        if !same_entry {
            return Err(format!("workspace entry '{new_path}' already exists"));
        }
    }
    std::fs::rename(&source, &target)
        .map_err(|error| format!("could not rename '{path}' to '{new_path}': {error}"))
}

#[tauri::command]
pub(super) async fn rename_entry(
    state: State<'_, AppState>,
    window: WebviewWindow,
    path: String,
    new_path: String,
) -> Result<(), String> {
    let (project, scope, _activity) = writable_active_project(&state, window.label()).await?;
    rename_entry_at(&project.root, &path, &new_path)?;
    state
        .store
        .bump_state_generation(&scope)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn delete_entry_at(root: &Path, path: &str) -> Result<(), String> {
    let target = workspace_entry_path(root, path)?;
    let metadata = std::fs::symlink_metadata(&target)
        .map_err(|error| format!("workspace entry '{path}' was not found: {error}"))?;
    let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(&target)
    } else {
        std::fs::remove_file(&target)
    };
    result.map_err(|error| format!("could not delete '{path}': {error}"))
}

#[tauri::command]
pub(super) async fn delete_entry(
    state: State<'_, AppState>,
    window: WebviewWindow,
    path: String,
) -> Result<(), String> {
    let (project, scope, _activity) = writable_active_project(&state, window.label()).await?;
    delete_entry_at(&project.root, &path)?;
    state
        .store
        .bump_state_generation(&scope)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn validate_remote_path(path: &str) -> Result<(), String> {
    if path.len() > 4096 {
        return Err("Remote path exceeds 4096 bytes".into());
    }
    if path.contains(['\0', '\n', '\r']) {
        return Err("Remote path must not contain NUL or line breaks".into());
    }
    Ok(())
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn remote_path_expression(path: Option<&str>) -> Result<String, String> {
    let path = path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or("~");
    validate_remote_path(path)?;
    if path == "~" {
        Ok("\"$HOME\"".into())
    } else if let Some(rest) = path.strip_prefix("~/") {
        Ok(format!("\"$HOME\"/{}", shell_single_quote(rest)))
    } else {
        Ok(shell_single_quote(path))
    }
}

fn remote_directory_script(path: Option<&str>) -> Result<String, String> {
    let path = remote_path_expression(path)?;
    Ok(format!(
        r#"LC_ALL=C
dir={path}
case "$dir" in -*) dir="./$dir" ;; esac
if ! CDPATH= cd "$dir" 2>/dev/null; then
  printf 'Cannot open remote directory: %s\n' "$dir" >&2
  exit 66
fi
printf 'WISP_REMOTE_DIR_V1\000%s\000' "$(pwd -P)"
for entry in ./*; do
  if [ ! -e "$entry" ] && [ ! -L "$entry" ]; then
    continue
  fi
  name=${{entry#./}}
  mtime=$(stat -c '%Y' "$entry" 2>/dev/null) ||
    mtime=$(stat -f '%m' "$entry" 2>/dev/null) ||
    mtime=0
  if [ -d "$entry" ]; then
    kind=d
    size=0
  else
    kind=f
    size=$(stat -c '%s' "$entry" 2>/dev/null) ||
      size=$(stat -f '%z' "$entry" 2>/dev/null) ||
      size=0
  fi
  printf '%s\000%s\000%s\000%s\000' "$kind" "$size" "$mtime" "$name"
done"#
    ))
}

fn build_remote_directory_command(
    context: &wisp_store::ExecutionContext,
    path: Option<&str>,
) -> Result<RemoteCommand, String> {
    let connection = crate::ssh_hosts::SshConnection::from_execution_context(context)?;
    let mut args = connection.ssh_args()?;
    args.push(remote_directory_script(path)?);
    Ok(RemoteCommand {
        context_id: context.id.clone(),
        program: "ssh".into(),
        args,
        envs: crate::ssh_hosts::auth_envs_for_connection(&connection)?,
    })
}

/// Slice off everything before (and including) the protocol marker, so login
/// banners and motd noise never reach the parser.
fn protocol_payload<'a>(stdout: &'a [u8], marker: &[u8]) -> Result<&'a [u8], String> {
    stdout
        .windows(marker.len())
        .position(|window| window == marker)
        .map(|start| &stdout[start + marker.len()..])
        .ok_or_else(|| "Remote response did not contain the expected protocol marker".into())
}

fn parse_remote_directory(stdout: &[u8]) -> Result<DirectoryListing, String> {
    let fields = protocol_payload(stdout, REMOTE_DIR_PROTOCOL)?
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>();
    let Some(path) = fields.first().filter(|field| !field.is_empty()) else {
        return Err("Remote directory response omitted its path".into());
    };
    let records = &fields[1..];
    let records = if records.last().is_some_and(|field| field.is_empty()) {
        &records[..records.len() - 1]
    } else {
        records
    };
    if records.len() % 4 != 0 {
        return Err("Remote directory response contained an incomplete entry".into());
    }
    let mut entries = Vec::with_capacity(records.len() / 4);
    for record in records.chunks_exact(4) {
        let is_dir = match record[0] {
            b"d" => true,
            b"f" => false,
            _ => return Err("Remote directory response contained an invalid entry kind".into()),
        };
        let size = String::from_utf8_lossy(record[1])
            .trim()
            .parse::<u64>()
            .map_err(|_| "Remote directory response contained an invalid file size".to_string())?;
        let mtime_secs = String::from_utf8_lossy(record[2])
            .trim()
            .parse::<u64>()
            .map_err(|_| {
                "Remote directory response contained an invalid modified time".to_string()
            })?;
        entries.push(DirEntry {
            name: String::from_utf8_lossy(record[3]).into_owned(),
            is_dir,
            size,
            modified_unix_millis: (mtime_secs > 0).then_some(mtime_secs.saturating_mul(1000)),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(DirectoryListing {
        path: String::from_utf8_lossy(path).into_owned(),
        entries,
    })
}

fn list_remote_dir_with_runner(
    context: &wisp_store::ExecutionContext,
    path: Option<&str>,
    runner: &mut dyn RemoteRunner,
) -> Result<DirectoryListing, String> {
    crate::ssh_hosts::require_managed_ssh_ready(context)?;
    let command = build_remote_directory_command(context, path)?;
    let output = match runner.run(&command) {
        Ok(output) => output,
        Err(error) => {
            if crate::ssh_guard::is_authentication_failure(&error) {
                crate::ssh_guard::record_failure(&context.id, &error);
            }
            return Err(error);
        }
    };
    if output.status != 0 {
        let detail = if output.stderr.is_empty() {
            "no error details returned".to_string()
        } else {
            output.stderr
        };
        let error = format!(
            "Remote directory request failed (exit {}): {detail}",
            output.status
        );
        if crate::ssh_guard::is_authentication_failure(&error) {
            crate::ssh_guard::record_failure(&context.id, &error);
        }
        return Err(error);
    }
    crate::ssh_guard::record_success(&context.id);
    parse_remote_directory(&output.stdout)
}

/// POSIX-sh script that size-checks then streams one remote file in full.
/// Binary / office previews need an intact payload, so oversized files are
/// rejected on the remote before any bytes cross the wire.
fn remote_file_script(path: &str, cap: u64) -> Result<String, String> {
    let path = remote_path_expression(Some(path))?;
    Ok(format!(
        r#"LC_ALL=C
f={path}
case "$f" in -*) f="./$f" ;; esac
if [ ! -f "$f" ]; then
  printf 'Cannot read remote file: %s\n' "$f" >&2
  exit 66
fi
size=$(stat -c '%s' "$f" 2>/dev/null) ||
  size=$(stat -f '%z' "$f" 2>/dev/null) ||
  size=0
if [ "$size" -gt {cap} ]; then
  printf 'Remote file exceeds {cap} byte limit: %s\n' "$f" >&2
  exit 67
fi
printf 'WISP_REMOTE_FILE_V1\000'
cat "$f""#
    ))
}

/// POSIX-sh script that returns a bounded head of a remote text-ish file plus
/// the real size, so multi-GB MEDLINE dumps preview without a full transfer.
fn remote_text_file_script(path: &str, cap: u64) -> Result<String, String> {
    let path = remote_path_expression(Some(path))?;
    Ok(format!(
        r#"LC_ALL=C
f={path}
case "$f" in -*) f="./$f" ;; esac
if [ ! -f "$f" ]; then
  printf 'Cannot read remote file: %s\n' "$f" >&2
  exit 66
fi
size=$(stat -c '%s' "$f" 2>/dev/null) ||
  size=$(stat -f '%z' "$f" 2>/dev/null) ||
  size=0
printf 'WISP_REMOTE_FILE_TEXT_V1\000'
printf '%s\000' "$size"
if [ "$size" -eq 0 ] || [ {cap} -eq 0 ]; then
  :
elif command -v head >/dev/null 2>&1; then
  head -c {cap} "$f"
else
  dd if="$f" bs={cap} count=1 2>/dev/null
fi"#
    ))
}

fn build_remote_file_command(
    context: &wisp_store::ExecutionContext,
    path: &str,
    cap: u64,
) -> Result<RemoteCommand, String> {
    let connection = crate::ssh_hosts::SshConnection::from_execution_context(context)?;
    let mut args = connection.ssh_args()?;
    args.push(remote_file_script(path, cap)?);
    Ok(RemoteCommand {
        context_id: context.id.clone(),
        program: "ssh".into(),
        args,
        envs: crate::ssh_hosts::auth_envs_for_connection(&connection)?,
    })
}

fn build_remote_text_file_command(
    context: &wisp_store::ExecutionContext,
    path: &str,
    cap: u64,
) -> Result<RemoteCommand, String> {
    let connection = crate::ssh_hosts::SshConnection::from_execution_context(context)?;
    let mut args = connection.ssh_args()?;
    args.push(remote_text_file_script(path, cap)?);
    Ok(RemoteCommand {
        context_id: context.id.clone(),
        program: "ssh".into(),
        args,
        envs: crate::ssh_hosts::auth_envs_for_connection(&connection)?,
    })
}

fn run_remote_command(
    context: &wisp_store::ExecutionContext,
    command: &RemoteCommand,
    runner: &mut dyn RemoteRunner,
) -> Result<RemoteOutput, String> {
    match runner.run(command) {
        Ok(output) => Ok(output),
        Err(error) => {
            if crate::ssh_guard::is_authentication_failure(&error) {
                crate::ssh_guard::record_failure(&context.id, &error);
            }
            Err(error)
        }
    }
}

fn remote_command_failed(context: &wisp_store::ExecutionContext, output: &RemoteOutput) -> String {
    let detail = if output.stderr.is_empty() {
        "no error details returned".to_string()
    } else {
        output.stderr.clone()
    };
    let error = format!(
        "Remote file request failed (exit {}): {detail}",
        output.status
    );
    if crate::ssh_guard::is_authentication_failure(&error) {
        crate::ssh_guard::record_failure(&context.id, &error);
    }
    error
}

/// Payload shape: `total_size\0<body>` after the text-protocol marker.
fn parse_remote_text_payload(stdout: &[u8]) -> Result<(Vec<u8>, u64), String> {
    let payload = protocol_payload(stdout, REMOTE_FILE_TEXT_PROTOCOL)?;
    let zero = payload
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| "Remote text response omitted its size field".to_string())?;
    let total = std::str::from_utf8(&payload[..zero])
        .map_err(|_| "Remote text response size was not UTF-8".to_string())?
        .parse::<u64>()
        .map_err(|_| "Remote text response size was not a number".to_string())?;
    Ok((payload[zero + 1..].to_vec(), total))
}

fn read_remote_file_bytes_with_runner(
    context: &wisp_store::ExecutionContext,
    path: &str,
    max_bytes: Option<u64>,
    runner: &mut dyn RemoteRunner,
) -> Result<Vec<u8>, String> {
    crate::ssh_hosts::require_managed_ssh_ready(context)?;
    let cap = preview_byte_cap(max_bytes).min(REMOTE_FILE_MAX_BYTES);
    let command = build_remote_file_command(context, path, cap)?;
    let output = run_remote_command(context, &command, runner)?;
    if output.status != 0 {
        return Err(remote_command_failed(context, &output));
    }
    crate::ssh_guard::record_success(&context.id);
    let mut bytes = protocol_payload(&output.stdout, REMOTE_FILE_PROTOCOL)?.to_vec();
    if bytes.len() as u64 > cap {
        return Err(format!("remote file exceeds {cap} byte limit"));
    }
    if is_ooxml_path(Path::new(path)) {
        validate_ooxml_archive(&bytes)?;
        bytes = transcode_ooxml_tiff_media(bytes);
    }
    Ok(bytes)
}

fn read_remote_file_with_runner(
    context: &wisp_store::ExecutionContext,
    path: &str,
    max_bytes: Option<u64>,
    runner: &mut dyn RemoteRunner,
) -> Result<FileContent, String> {
    crate::ssh_hosts::require_managed_ssh_ready(context)?;
    let cap = text_preview_byte_cap(max_bytes).min(REMOTE_FILE_MAX_BYTES);
    let command = build_remote_text_file_command(context, path, cap)?;
    let output = run_remote_command(context, &command, runner)?;
    if output.status != 0 {
        return Err(remote_command_failed(context, &output));
    }
    crate::ssh_guard::record_success(&context.id);
    let (bytes, total) = parse_remote_text_payload(&output.stdout)?;
    if bytes.len() as u64 > cap {
        return Err(format!("remote file exceeds {cap} byte limit"));
    }
    let mime = mime_for_path(Path::new(path));
    if !is_text_preview_bytes(mime, &bytes) {
        // Binary: require a full, under-budget transfer via the bytes path.
        if total > cap {
            return Err(format!("remote file exceeds {cap} byte limit"));
        }
        let full = read_remote_file_bytes_with_runner(context, path, Some(cap), runner)?;
        if let Some(Ok(markdown)) = wisp_tools::read::document_markdown(Path::new(path), &full) {
            return Ok(FileContent {
                path: path.to_string(),
                mime: mime.into(),
                text: Some(markdown),
                base64: None,
                truncated: false,
                total_bytes: Some(total),
            });
        }
        return Ok(file_content_from_bytes(
            path.to_string(),
            mime,
            full,
            Some(total),
            false,
        ));
    }
    let truncated = total > bytes.len() as u64;
    Ok(file_content_from_bytes(
        path.to_string(),
        mime,
        bytes,
        Some(total),
        truncated,
    ))
}

#[tauri::command]
pub(super) async fn read_remote_file(
    state: State<'_, AppState>,
    context_id: String,
    path: String,
    max_bytes: Option<u64>,
) -> Result<FileContent, String> {
    crate::run_context::remote_files::refuse_if_context_path_discarded(
        &state.store,
        &context_id,
        &path,
    )
    .await?;
    let context = state
        .store
        .get_execution_context(&context_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
    tokio::task::spawn_blocking(move || {
        let mut runner = ProcessRemoteRunner;
        read_remote_file_with_runner(&context, &path, max_bytes, &mut runner)
    })
    .await
    .map_err(|e| format!("Remote file task failed: {e}"))?
}

#[tauri::command]
pub(super) async fn read_remote_file_bytes(
    state: State<'_, AppState>,
    context_id: String,
    path: String,
    max_bytes: Option<u64>,
) -> Result<Response, String> {
    crate::run_context::remote_files::refuse_if_context_path_discarded(
        &state.store,
        &context_id,
        &path,
    )
    .await?;
    let context = state
        .store
        .get_execution_context(&context_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
    let bytes = tokio::task::spawn_blocking(move || {
        let mut runner = ProcessRemoteRunner;
        read_remote_file_bytes_with_runner(&context, &path, max_bytes, &mut runner)
    })
    .await
    .map_err(|e| format!("Remote file task failed: {e}"))??;
    Ok(Response::new(bytes))
}

#[tauri::command]
pub(super) async fn list_remote_dir(
    state: State<'_, AppState>,
    context_id: String,
    path: Option<String>,
) -> Result<DirectoryListing, String> {
    let context = state
        .store
        .get_execution_context(&context_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
    tokio::task::spawn_blocking(move || {
        let mut runner = ProcessRemoteRunner;
        list_remote_dir_with_runner(&context, path.as_deref(), &mut runner)
    })
    .await
    .map_err(|e| format!("Remote directory task failed: {e}"))?
}

pub(super) fn read_file_at(
    root: &Path,
    path: String,
    max_bytes: Option<u64>,
) -> Result<FileContent, String> {
    let real = wisp_tools::safety::validate_file_path(root, &path)?;
    let mime = mime_for_path(&real);
    // Prefetch only a prefix so multi-GB text never lands fully in RAM.
    // Binary / OOXML still require a complete payload under the higher ceiling.
    let text_cap = text_preview_byte_cap(max_bytes);
    let (prefix, total) = read_path_prefix(&real, text_cap)?;
    if supports_prefix_preview(mime) && is_text_preview_bytes(mime, &prefix) {
        let truncated = total > prefix.len() as u64;
        return Ok(file_content_from_bytes(
            real.to_string_lossy().into_owned(),
            mime,
            prefix,
            Some(total),
            truncated,
        ));
    }

    let full_cap = preview_byte_cap(max_bytes);
    if total > full_cap {
        return Err(format!("file exceeds {full_cap} byte limit"));
    }
    let bytes = if total <= prefix.len() as u64 {
        prefix
    } else {
        std::fs::read(&real).map_err(|e| format!("{e}"))?
    };
    if let Some(Ok(markdown)) = wisp_tools::read::document_markdown(&real, &bytes) {
        return Ok(FileContent {
            path: real.to_string_lossy().into_owned(),
            mime: mime.into(),
            text: Some(markdown),
            base64: None,
            truncated: false,
            total_bytes: Some(total),
        });
    }
    Ok(file_content_from_bytes(
        real.to_string_lossy().into_owned(),
        mime,
        bytes,
        Some(total),
        false,
    ))
}

pub(super) fn read_file_bytes_at(
    root: &Path,
    path: &str,
    max_bytes: Option<u64>,
) -> Result<Vec<u8>, String> {
    let real = wisp_tools::safety::validate_file_path(root, path)?;
    let cap = preview_byte_cap(max_bytes);
    let len = std::fs::metadata(&real).map_err(|e| format!("{e}"))?.len();
    if len > cap {
        return Err(format!("file exceeds {cap} byte limit"));
    }
    let mut bytes = std::fs::read(&real).map_err(|e| format!("{e}"))?;
    if is_ooxml_path(&real) {
        validate_ooxml_archive(&bytes)?;
        bytes = transcode_ooxml_tiff_media(bytes);
    }
    Ok(bytes)
}

/// The shared text-vs-binary decision for previews, local or remote: named text
/// mimes go out as text, unnamed extensions are sniffed, the rest is base64.
fn file_content_from_bytes(
    path: String,
    mime: &'static str,
    bytes: Vec<u8>,
    total_bytes: Option<u64>,
    truncated: bool,
) -> FileContent {
    if is_text_preview_bytes(mime, &bytes) {
        FileContent {
            path,
            mime: mime.into(),
            text: Some(String::from_utf8_lossy(&bytes).into_owned()),
            base64: None,
            truncated,
            total_bytes,
        }
    } else {
        FileContent {
            path,
            mime: mime.into(),
            text: None,
            base64: Some(base64::engine::general_purpose::STANDARD.encode(&bytes)),
            truncated: false,
            total_bytes,
        }
    }
}

#[tauri::command]
pub(super) fn read_file(
    state: State<'_, AppState>,
    window: WebviewWindow,
    path: String,
    max_bytes: Option<u64>,
) -> Result<FileContent, String> {
    read_file_at(&state.require_active(window.label())?.root, path, max_bytes)
}

#[tauri::command]
pub(super) fn read_file_bytes(
    state: State<'_, AppState>,
    window: WebviewWindow,
    path: String,
    max_bytes: Option<u64>,
) -> Result<Response, String> {
    let bytes = read_file_bytes_at(
        &state.require_active(window.label())?.root,
        &path,
        max_bytes,
    )?;
    Ok(Response::new(bytes))
}

/// Derive the `reviews/<stem>.md` sidecar path for a previewed source file and
/// append a quoted passage to it. The sidecar is plain Markdown so the agent
/// reads it back with its ordinary read/grep tools — no new protocol. Returns
/// the sidecar's path relative to the project root (for a UI confirmation).
pub(super) fn append_review_note_at(
    root: &Path,
    source_path: &str,
    quote: &str,
    note: Option<&str>,
) -> Result<String, String> {
    let quote = quote.trim();
    if quote.is_empty() {
        return Err("nothing selected to annotate".into());
    }
    // Name the sidecar after the source file's stem; a bare selection with no
    // source still lands in a shared `reviews/notes.md`.
    let stem = Path::new(source_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "notes".into());
    let reviews_dir = root.join("reviews");
    std::fs::create_dir_all(&reviews_dir)
        .map_err(|e| format!("could not create reviews folder: {e}"))?;

    let rel = format!("reviews/{stem}.md");
    let real = wisp_tools::safety::validate_file_path(root, &rel)?;

    let source_name = Path::new(source_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| source_path.to_string());
    let quoted = quote
        .lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut block = format!("\n{quoted}\n\n— {source_name}\n");
    if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
        block = format!("\n{quoted}\n\n{note}\n\n— {source_name}\n");
    }
    // Seed a heading the first time so the file reads as a review document.
    let mut out = String::new();
    if !real.exists() {
        out.push_str(&format!("# Review notes — {source_name}\n"));
    }
    out.push_str(&block);

    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&real)
        .map_err(|e| format!("could not open review file: {e}"))?;
    file.write_all(out.as_bytes())
        .map_err(|e| format!("could not write review note: {e}"))?;
    Ok(rel)
}

#[tauri::command]
pub(super) async fn append_review_note(
    state: State<'_, AppState>,
    window: WebviewWindow,
    source_path: String,
    quote: String,
    note: Option<String>,
) -> Result<String, String> {
    let (project, scope, _activity) = writable_active_project(&state, window.label()).await?;
    let path = append_review_note_at(&project.root, &source_path, &quote, note.as_deref())?;
    state
        .store
        .bump_state_generation(&scope)
        .await
        .map_err(|error| error.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::io::Write;

    struct FakeRemoteRunner {
        outputs: Vec<Result<RemoteOutput, String>>,
        commands: Vec<RemoteCommand>,
    }

    impl FakeRemoteRunner {
        fn returning(output: RemoteOutput) -> Self {
            Self {
                outputs: vec![Ok(output)],
                commands: Vec::new(),
            }
        }

        fn sequence(outputs: Vec<RemoteOutput>) -> Self {
            Self {
                outputs: outputs.into_iter().map(Ok).collect(),
                commands: Vec::new(),
            }
        }
    }

    impl RemoteRunner for FakeRemoteRunner {
        fn run(&mut self, command: &RemoteCommand) -> Result<RemoteOutput, String> {
            self.commands.push(command.clone());
            if self.outputs.is_empty() {
                panic!("fake remote runner ran out of outputs");
            }
            self.outputs.remove(0)
        }
    }

    fn test_identity_file() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "wisp-file-browser-test-key-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, b"test-key\n").unwrap();
        path
    }

    fn ssh_context(identity_file: &std::path::Path) -> wisp_store::ExecutionContext {
        let mut context = wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap();
        context.config_json = serde_json::json!({
            "alias": "gpu.example",
            "user": "researcher",
            "port": 2222,
            "identity_file": identity_file.to_string_lossy(),
        })
        .to_string();
        context.last_probe_status = Some("ok".into());
        context
    }

    #[test]
    fn bibliography_files_are_read_as_text() {
        assert_eq!(mime_for_path(Path::new("references.bib")), "text/x-bibtex");
        assert!(is_text_mime(mime_for_path(Path::new("references.bib"))));
    }

    #[test]
    fn office_files_have_specific_mime_types() {
        assert_eq!(
            mime_for_path(Path::new("results.xlsx")),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
        assert_eq!(
            mime_for_path(Path::new("talk.pptx")),
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        );
        assert_eq!(mime_for_path(Path::new("notes.rtf")), "application/rtf");
    }

    #[test]
    fn rich_document_preview_uses_anydoc_markdown() {
        let base =
            std::env::temp_dir().join(format!("wisp-anydoc-preview-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(
            base.join("protocol.rtf"),
            br#"{\rtf1\ansi\b Experimental protocol\b0\par Centrifuge at 12000 g.}"#,
        )
        .unwrap();

        let content = read_file_at(&base, "protocol.rtf".into(), None).unwrap();
        assert_eq!(content.mime, "application/rtf");
        assert!(content.text.unwrap().contains("**Experimental protocol**"));
        assert!(content.base64.is_none());
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn remote_directory_command_uses_context_connection_and_quotes_path() {
        let identity = test_identity_file();
        let command = build_remote_directory_command(
            &ssh_context(&identity),
            Some("/work/O'Brien; printf unsafe"),
        )
        .unwrap();
        assert_eq!(command.program, "ssh");
        assert!(command.args.windows(2).any(|args| args == ["-p", "2222"]));
        assert!(command
            .args
            .windows(2)
            .any(|args| { args[0] == "-i" && args[1] == identity.to_string_lossy() }));
        assert!(command
            .args
            .iter()
            .any(|arg| arg == "researcher@gpu.example"));
        let script = command.args.last().unwrap();
        assert!(script.contains("dir='/work/O'\"'\"'Brien; printf unsafe'"));
        assert!(script.contains("WISP_REMOTE_DIR_V1\\000"));
        assert!(script.contains("stat -c '%s'"));
        assert!(script.contains("stat -f '%z'"));
        assert!(script.contains("stat -c '%Y'"));
        assert!(script.contains("stat -f '%m'"));
        assert!(!script.contains("wc -c"));
    }

    #[test]
    fn remote_directory_rejects_paths_with_line_breaks() {
        let error = remote_directory_script(Some("/work\nmalformed")).unwrap_err();
        assert!(error.contains("line breaks"));
    }

    #[test]
    fn remote_directory_runner_parses_banner_and_sorts_directories_first() {
        let identity = test_identity_file();
        let stdout = b"login banner\nWISP_REMOTE_DIR_V1\0/home/research\0f\012\01700000000\0notes.txt\0d\00\01700001000\0projects\0f\03\01700002000\0a.csv\0".to_vec();
        let mut runner = FakeRemoteRunner::returning(RemoteOutput {
            status: 0,
            stdout,
            stderr: String::new(),
        });
        let listing =
            list_remote_dir_with_runner(&ssh_context(&identity), Some("~"), &mut runner).unwrap();
        assert_eq!(listing.path, "/home/research");
        assert_eq!(
            listing.entries,
            vec![
                DirEntry {
                    name: "projects".into(),
                    is_dir: true,
                    size: 0,
                    modified_unix_millis: Some(1_700_001_000_000),
                },
                DirEntry {
                    name: "a.csv".into(),
                    is_dir: false,
                    size: 3,
                    modified_unix_millis: Some(1_700_002_000_000),
                },
                DirEntry {
                    name: "notes.txt".into(),
                    is_dir: false,
                    size: 12,
                    modified_unix_millis: Some(1_700_000_000_000),
                },
            ]
        );
        assert_eq!(runner.commands.len(), 1);
    }

    #[test]
    fn remote_directory_runner_surfaces_ssh_failure() {
        let identity = test_identity_file();
        let mut runner = FakeRemoteRunner::returning(RemoteOutput {
            status: 255,
            stdout: Vec::new(),
            stderr: "Permission denied".into(),
        });
        let error = list_remote_dir_with_runner(&ssh_context(&identity), Some("~"), &mut runner)
            .unwrap_err();
        assert!(error.contains("exit 255"));
        assert!(error.contains("Permission denied"));
    }

    #[test]
    fn remote_file_command_quotes_path_and_guards_size() {
        let identity = test_identity_file();
        let command = build_remote_file_command(
            &ssh_context(&identity),
            "/work/O'Brien results.html",
            REMOTE_FILE_MAX_BYTES,
        )
        .unwrap();
        assert_eq!(command.program, "ssh");
        let script = command.args.last().unwrap();
        assert!(script.contains("f='/work/O'\"'\"'Brien results.html'"));
        assert!(script.contains("WISP_REMOTE_FILE_V1\\000"));
        assert!(script.contains(&format!("-gt {REMOTE_FILE_MAX_BYTES}")));
        assert!(script.contains("cat \"$f\""));
    }

    #[test]
    fn remote_text_file_command_streams_head_with_size() {
        let identity = test_identity_file();
        let command =
            build_remote_text_file_command(&ssh_context(&identity), "/data/medline.txt", 1024)
                .unwrap();
        let script = command.args.last().unwrap();
        assert!(script.contains("WISP_REMOTE_FILE_TEXT_V1\\000"));
        assert!(script.contains("head -c 1024"));
        assert!(!script.contains("cat \"$f\""));
        assert!(!script.contains("-gt 1024"));
    }

    #[test]
    fn local_text_preview_reads_only_a_prefix() {
        let base = std::env::temp_dir().join(format!(
            "wisp_text_prefix_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let body = "alpha\n".repeat(10_000); // ~60 KB
        std::fs::write(base.join("huge.txt"), &body).unwrap();

        let content = read_file_at(&base, "huge.txt".into(), Some(64)).unwrap();
        assert!(content.truncated);
        assert_eq!(content.total_bytes, Some(body.len() as u64));
        assert_eq!(content.text.as_deref(), Some(&body[..64]));
        assert!(content.base64.is_none());

        // Oversize binary still rejects rather than returning a partial image.
        let bin = vec![0u8; 128];
        std::fs::write(base.join("huge.bin"), &bin).unwrap();
        let error = read_file_at(&base, "huge.bin".into(), Some(32)).unwrap_err();
        assert!(error.contains("byte limit"));

        let _ = std::fs::remove_dir_all(&base);
    }

    fn test_ooxml(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut output);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, contents) in entries {
                archive.start_file(*name, options).unwrap();
                archive.write_all(contents).unwrap();
            }
            archive.finish().unwrap();
        }
        output.into_inner()
    }

    #[test]
    fn ooxml_validation_accepts_bounded_archives() {
        let bytes = test_ooxml(&[
            ("[Content_Types].xml", b"<Types/>"),
            ("xl/workbook.xml", b"<workbook/>"),
        ]);
        validate_ooxml_archive(&bytes).unwrap();
    }

    #[test]
    fn ooxml_validation_rejects_unsafe_paths_and_active_content() {
        let traversal = test_ooxml(&[("../outside.xml", b"unsafe")]);
        assert!(validate_ooxml_archive(&traversal)
            .unwrap_err()
            .contains("unsafe path"));

        let macro_archive = test_ooxml(&[("xl/vbaProject.bin", b"macro")]);
        assert!(validate_ooxml_archive(&macro_archive)
            .unwrap_err()
            .contains("active content"));
    }

    #[test]
    fn ooxml_validation_blocks_external_media_and_unsafe_links() {
        let external_image = test_ooxml(&[(
            "word/_rels/document.xml.rels",
            br#"<Relationships><Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.invalid/pixel.png" TargetMode="External"/></Relationships>"#,
        )]);
        assert!(validate_ooxml_archive(&external_image)
            .unwrap_err()
            .contains("external media"));

        let script_link = test_ooxml(&[(
            "word/_rels/document.xml.rels",
            br#"<Relationships><Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="javascript:alert(1)" TargetMode="External"/></Relationships>"#,
        )]);
        assert!(validate_ooxml_archive(&script_link)
            .unwrap_err()
            .contains("unsafe external hyperlink"));

        let safe_link = test_ooxml(&[(
            "word/_rels/document.xml.rels",
            br#"<Relationships><Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.org/paper" TargetMode="External"/></Relationships>"#,
        )]);
        validate_ooxml_archive(&safe_link).unwrap();
    }

    #[test]
    fn raw_file_reader_preserves_binary_bytes_and_validates_ooxml() {
        let base = std::env::temp_dir().join(format!(
            "wisp_raw_preview_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let bytes = test_ooxml(&[("[Content_Types].xml", b"<Types/>")]);
        std::fs::write(base.join("results.xlsx"), &bytes).unwrap();
        assert_eq!(
            read_file_bytes_at(&base, "results.xlsx", None).unwrap(),
            bytes
        );
        std::fs::write(base.join("broken.pptx"), b"not a zip").unwrap();
        assert!(read_file_bytes_at(&base, "broken.pptx", None)
            .unwrap_err()
            .contains("invalid OOXML archive"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn ooxml_tiff_media_transcodes_to_png() {
        let mut tiff = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0])))
            .write_to(&mut Cursor::new(&mut tiff), image::ImageFormat::Tiff)
            .unwrap();
        let bytes = test_ooxml(&[
            (
                "[Content_Types].xml",
                br#"<Types><Default Extension="tiff" ContentType="image/tiff"/></Types>"#,
            ),
            ("word/document.xml", b"<document/>"),
            ("word/media/image1.tiff", &tiff),
        ]);
        let rewritten = transcode_ooxml_tiff_media(bytes.clone());
        assert_ne!(rewritten, bytes);
        let mut archive = zip::ZipArchive::new(Cursor::new(&rewritten)).unwrap();
        let mut media = Vec::new();
        archive
            .by_name("word/media/image1.tiff")
            .unwrap()
            .read_to_end(&mut media)
            .unwrap();
        assert!(media.starts_with(b"\x89PNG"));
        let mut types = String::new();
        archive
            .by_name("[Content_Types].xml")
            .unwrap()
            .read_to_string(&mut types)
            .unwrap();
        assert!(types.contains("image/png"));
        assert!(!types.contains("image/tiff"));
        let mut doc = Vec::new();
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_end(&mut doc)
            .unwrap();
        assert_eq!(doc, b"<document/>");
    }

    #[test]
    fn ooxml_without_tiff_media_passes_through_untouched() {
        let plain = test_ooxml(&[
            ("[Content_Types].xml", b"<Types/>"),
            ("word/media/image1.png", b"\x89PNGnot-really"),
        ]);
        assert_eq!(transcode_ooxml_tiff_media(plain.clone()), plain);
        // A TIFF that fails to decode keeps its original bytes.
        let broken = test_ooxml(&[("word/media/image1.tiff", b"II*\0garbage")]);
        let rewritten = transcode_ooxml_tiff_media(broken);
        let mut archive = zip::ZipArchive::new(Cursor::new(&rewritten)).unwrap();
        let mut media = Vec::new();
        archive
            .by_name("word/media/image1.tiff")
            .unwrap()
            .read_to_end(&mut media)
            .unwrap();
        assert_eq!(media, b"II*\0garbage");
    }

    #[test]
    fn remote_file_runner_sniffs_text_after_banner() {
        let identity = test_identity_file();
        let stdout = b"motd noise\nWISP_REMOTE_FILE_TEXT_V1\012\0print('hi')\n".to_vec();
        let mut runner = FakeRemoteRunner::returning(RemoteOutput {
            status: 0,
            stdout,
            stderr: String::new(),
        });
        let content = read_remote_file_with_runner(
            &ssh_context(&identity),
            "~/analysis.py",
            None,
            &mut runner,
        )
        .unwrap();
        assert_eq!(content.mime, "text/x-python");
        assert_eq!(content.text.as_deref(), Some("print('hi')\n"));
        assert!(content.base64.is_none());
        assert_eq!(content.path, "~/analysis.py");
        assert!(!content.truncated);
        assert_eq!(content.total_bytes, Some(12));
    }

    #[test]
    fn remote_file_runner_returns_truncated_text_head() {
        let identity = test_identity_file();
        let head = b"PMID- 1\nTI  - plant\n";
        let mut stdout = b"WISP_REMOTE_FILE_TEXT_V1\0".to_vec();
        stdout.extend_from_slice(b"999999999\0");
        stdout.extend_from_slice(head);
        let mut runner = FakeRemoteRunner::returning(RemoteOutput {
            status: 0,
            stdout,
            stderr: String::new(),
        });
        let content = read_remote_file_with_runner(
            &ssh_context(&identity),
            "/data/medline.txt",
            Some(head.len() as u64),
            &mut runner,
        )
        .unwrap();
        assert_eq!(content.text.as_deref(), Some("PMID- 1\nTI  - plant\n"));
        assert!(content.truncated);
        assert_eq!(content.total_bytes, Some(999_999_999));
    }

    #[test]
    fn remote_file_runner_returns_binary_as_base64() {
        let identity = test_identity_file();
        // Text protocol sniffs binary and re-fetches full file via V1 bytes path.
        let head = b"WISP_REMOTE_FILE_TEXT_V1\05\0\x89PNG\0\x01";
        let full = b"WISP_REMOTE_FILE_V1\0\x89PNG\0\x01";
        let mut runner = FakeRemoteRunner::sequence(vec![
            RemoteOutput {
                status: 0,
                stdout: head.to_vec(),
                stderr: String::new(),
            },
            RemoteOutput {
                status: 0,
                stdout: full.to_vec(),
                stderr: String::new(),
            },
        ]);
        let content = read_remote_file_with_runner(
            &ssh_context(&identity),
            "/plots/figure.png",
            None,
            &mut runner,
        )
        .unwrap();
        assert_eq!(content.mime, "image/png");
        assert!(content.text.is_none());
        assert_eq!(
            content.base64.as_deref(),
            Some(
                base64::engine::general_purpose::STANDARD
                    .encode(b"\x89PNG\0\x01")
                    .as_str()
            )
        );
        assert!(!content.truncated);
        assert_eq!(runner.commands.len(), 2);
    }

    #[test]
    fn remote_file_runner_surfaces_size_limit_failure_for_oversize_binary() {
        let identity = test_identity_file();
        // NULs in the head make it binary; total size above the text cap rejects.
        let mut stdout = b"WISP_REMOTE_FILE_TEXT_V1\0".to_vec();
        stdout.extend_from_slice(format!("{}\0", REMOTE_FILE_MAX_BYTES + 1).as_bytes());
        stdout.extend_from_slice(b"MZ\0bin");
        let mut runner = FakeRemoteRunner::returning(RemoteOutput {
            status: 0,
            stdout,
            stderr: String::new(),
        });
        let error = read_remote_file_with_runner(
            &ssh_context(&identity),
            "/big.bam",
            Some(REMOTE_FILE_MAX_BYTES),
            &mut runner,
        )
        .unwrap_err();
        assert!(error.contains("byte limit"));
    }

    #[test]
    fn remote_file_bytes_runner_surfaces_size_limit_failure() {
        let identity = test_identity_file();
        let mut runner = FakeRemoteRunner::returning(RemoteOutput {
            status: 67,
            stdout: Vec::new(),
            stderr: "Remote file exceeds 33554432 byte limit: /big.bam".into(),
        });
        let error = read_remote_file_bytes_with_runner(
            &ssh_context(&identity),
            "/big.bam",
            Some(REMOTE_FILE_MAX_BYTES),
            &mut runner,
        )
        .unwrap_err();
        assert!(error.contains("exit 67"));
        assert!(error.contains("byte limit"));
    }

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

    #[test]
    fn list_dir_entries_include_size_and_modified_time() {
        let base = std::env::temp_dir().join(format!(
            "wisp_list_dir_mtime_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("notes.md"), b"hello").unwrap();
        std::fs::create_dir(base.join("analysis")).unwrap();

        let entries = list_dir_entries(&base).unwrap();
        assert_eq!(entries[0].name, "analysis");
        assert!(entries[0].is_dir);
        assert!(entries[0].modified_unix_millis.is_some());
        assert_eq!(entries[1].name, "notes.md");
        assert!(!entries[1].is_dir);
        assert_eq!(entries[1].size, 5);
        let modified = entries[1].modified_unix_millis.expect("mtime");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(
            now.saturating_sub(modified) < 60_000,
            "listed mtime {modified} should be recent relative to {now}"
        );

        let json = serde_json::to_value(&entries[1]).unwrap();
        assert_eq!(json["size"], 5);
        assert_eq!(json["modified_unix_millis"], modified);
        let dto: wisp_dto::DirEntry = serde_json::from_value(json).unwrap();
        assert_eq!(dto.name, "notes.md");
        assert_eq!(dto.size, 5);
        assert_eq!(dto.modified_unix_millis, Some(modified));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn workspace_entry_operations_create_rename_and_delete_files_and_directories() {
        let base = std::env::temp_dir().join(format!(
            "wisp_file_operations_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&base).unwrap();

        create_file_at(&base, "notes.md").unwrap();
        assert_eq!(std::fs::read(base.join("notes.md")).unwrap(), b"");
        assert!(create_file_at(&base, "notes.md").is_err());

        create_directory_at(&base, "analysis").unwrap();
        create_file_at(&base, "analysis/results.csv").unwrap();
        rename_entry_at(&base, "analysis/results.csv", "analysis/final-results.csv").unwrap();
        assert!(!base.join("analysis/results.csv").exists());
        assert!(base.join("analysis/final-results.csv").is_file());

        rename_entry_at(&base, "analysis", "results").unwrap();
        assert!(base.join("results/final-results.csv").is_file());
        delete_entry_at(&base, "notes.md").unwrap();
        delete_entry_at(&base, "results").unwrap();
        assert!(!base.join("notes.md").exists());
        assert!(!base.join("results").exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn save_file_at_overwrites_in_root_and_rejects_escapes() {
        let base = std::env::temp_dir().join(format!(
            "wisp_save_file_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("analysis.R"), b"plot(1:3)\n").unwrap();

        save_file_at(&base, "analysis.R", "plot(4:6)\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(base.join("analysis.R")).unwrap(),
            "plot(4:6)\n"
        );

        // The editor saves what a preview loaded; escapes stay rejected.
        assert!(save_file_at(&base, "../outside.R", "x").is_err());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn workspace_entry_operations_stay_inside_root_and_never_overwrite() {
        let base = std::env::temp_dir().join(format!(
            "wisp_file_operation_safety_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("source.txt"), b"source").unwrap();
        std::fs::write(base.join("existing.txt"), b"keep").unwrap();

        rename_entry_at(&base, "source.txt", "source.txt").unwrap();
        assert!(create_file_at(&base, "../outside.txt").is_err());
        assert!(create_directory_at(&base, "../outside-dir").is_err());
        assert!(rename_entry_at(&base, "source.txt", "../moved.txt").is_err());
        assert!(delete_entry_at(&base, "..").is_err());
        assert!(rename_entry_at(&base, "source.txt", "existing.txt").is_err());
        assert_eq!(std::fs::read(base.join("source.txt")).unwrap(), b"source");
        assert_eq!(std::fs::read(base.join("existing.txt")).unwrap(), b"keep");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn script_files_are_text_and_unnamed_extensions_fall_back_to_sniffing() {
        let base = std::env::temp_dir().join(format!(
            "wisp_script_preview_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&base).unwrap();

        for (name, mime) in [
            ("analysis.R", "text/x-r"),
            ("analysis.py", "text/x-python"),
            ("analysis.sh", "text/x-shellscript"),
            ("analysis.ipynb", "application/x-ipynb+json"),
        ] {
            std::fs::write(base.join(name), b"print('preview')\n").unwrap();
            let content = read_file_at(&base, name.into(), None).unwrap();
            assert_eq!(content.mime, mime);
            assert_eq!(content.text.as_deref(), Some("print('preview')\n"));
            assert!(content.base64.is_none());
        }

        // #307: an extension nothing has a mime for (.toml, .lock, .unknown) used
        // to preview as "unsupported file type" even when it was plainly text.
        // The bytes decide now, so the mime stays octet-stream but text comes back.
        std::fs::write(base.join("analysis.unknown"), b"plain but unsupported\n").unwrap();
        let unnamed = read_file_at(&base, "analysis.unknown".into(), None).unwrap();
        assert_eq!(unnamed.mime, "application/octet-stream");
        assert_eq!(unnamed.text.as_deref(), Some("plain but unsupported\n"));
        assert!(unnamed.base64.is_none());

        // ...but a NUL byte still means binary, even amid valid UTF-8.
        std::fs::write(base.join("blob.unknown"), b"MZ\0\x01binary").unwrap();
        let binary = read_file_at(&base, "blob.unknown".into(), None).unwrap();
        assert!(binary.text.is_none(), "binary must not be sent as text");
        assert!(binary.base64.is_some());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn append_review_note_creates_sidecar_and_appends_quotes() {
        let base = std::env::temp_dir().join(format!(
            "wisp_review_note_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&base).unwrap();

        let rel = append_review_note_at(&base, "paper/manuscript.docx", "line one\nline two", None)
            .unwrap();
        assert_eq!(rel, "reviews/manuscript.md");
        let body = std::fs::read_to_string(base.join(&rel)).unwrap();
        assert!(body.starts_with("# Review notes — manuscript.docx"));
        assert!(body.contains("> line one\n> line two"));
        assert!(body.contains("— manuscript.docx"));

        // A second note with a comment appends without re-adding the heading.
        append_review_note_at(
            &base,
            "paper/manuscript.docx",
            "another passage",
            Some("fix wording"),
        )
        .unwrap();
        let body = std::fs::read_to_string(base.join(&rel)).unwrap();
        assert_eq!(body.matches("# Review notes").count(), 1);
        assert!(body.contains("> another passage"));
        assert!(body.contains("fix wording"));

        // Empty selection is rejected; path traversal is blocked by validate_file_path.
        assert!(append_review_note_at(&base, "x", "   ", None).is_err());

        let _ = std::fs::remove_dir_all(&base);
    }
}
