use super::AppState;
use base64::Engine;
use serde::Serialize;
use std::path::Path;
use tauri::{State, WebviewWindow};

const REMOTE_DIR_PROTOCOL: &[u8] = b"WISP_REMOTE_DIR_V1\0";

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub(super) struct DirEntry {
    name: String,
    is_dir: bool,
    size: u64,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub(super) struct DirectoryListing {
    path: String,
    entries: Vec<DirEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteDirectoryCommand {
    program: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteDirectoryOutput {
    status: i32,
    stdout: Vec<u8>,
    stderr: String,
}

trait RemoteDirectoryRunner: Send {
    fn run(&mut self, command: &RemoteDirectoryCommand) -> Result<RemoteDirectoryOutput, String>;
}

struct ProcessRemoteDirectoryRunner;

impl RemoteDirectoryRunner for ProcessRemoteDirectoryRunner {
    fn run(&mut self, command: &RemoteDirectoryCommand) -> Result<RemoteDirectoryOutput, String> {
        let mut process = std::process::Command::new(&command.program);
        process.args(&command.args);
        wisp_tools::process::hide_console(&mut process);
        let output = process
            .output()
            .map_err(|e| format!("failed to run {}: {e}", command.program))?;
        Ok(RemoteDirectoryOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
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
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("csv") => "text/csv",
        Some("tsv") => "text/tab-separated-values",
        Some("html" | "htm") => "text/html",
        Some("json") => "application/json",
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

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/") || mime == "application/json" || mime == "text/markdown"
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

fn validate_remote_path(path: &str) -> Result<(), String> {
    if path.len() > 4096 {
        return Err("Remote directory path exceeds 4096 bytes".into());
    }
    if path.contains(['\0', '\n', '\r']) {
        return Err("Remote directory path must not contain NUL or line breaks".into());
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
  if [ -d "$entry" ]; then
    kind=d
    size=0
  else
    kind=f
    size=$(stat -c '%s' "$entry" 2>/dev/null) ||
      size=$(stat -f '%z' "$entry" 2>/dev/null) ||
      size=0
  fi
  printf '%s\000%s\000%s\000' "$kind" "$size" "$name"
done"#
    ))
}

fn build_remote_directory_command(
    context: &wisp_store::ExecutionContext,
    path: Option<&str>,
) -> Result<RemoteDirectoryCommand, String> {
    let connection = crate::ssh_hosts::SshConnection::from_execution_context(context)?;
    let mut args = connection.ssh_args()?;
    args.push(remote_directory_script(path)?);
    Ok(RemoteDirectoryCommand {
        program: "ssh".into(),
        args,
    })
}

fn protocol_payload(stdout: &[u8]) -> Result<&[u8], String> {
    stdout
        .windows(REMOTE_DIR_PROTOCOL.len())
        .position(|window| window == REMOTE_DIR_PROTOCOL)
        .map(|start| &stdout[start + REMOTE_DIR_PROTOCOL.len()..])
        .ok_or_else(|| {
            "Remote directory response did not contain the expected protocol marker".into()
        })
}

fn parse_remote_directory(stdout: &[u8]) -> Result<DirectoryListing, String> {
    let fields = protocol_payload(stdout)?
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
    if records.len() % 3 != 0 {
        return Err("Remote directory response contained an incomplete entry".into());
    }
    let mut entries = Vec::with_capacity(records.len() / 3);
    for record in records.chunks_exact(3) {
        let is_dir = match record[0] {
            b"d" => true,
            b"f" => false,
            _ => return Err("Remote directory response contained an invalid entry kind".into()),
        };
        let size = String::from_utf8_lossy(record[1])
            .trim()
            .parse::<u64>()
            .map_err(|_| "Remote directory response contained an invalid file size".to_string())?;
        entries.push(DirEntry {
            name: String::from_utf8_lossy(record[2]).into_owned(),
            is_dir,
            size,
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
    runner: &mut dyn RemoteDirectoryRunner,
) -> Result<DirectoryListing, String> {
    let command = build_remote_directory_command(context, path)?;
    let output = runner.run(&command)?;
    if output.status != 0 {
        let detail = if output.stderr.is_empty() {
            "no error details returned".to_string()
        } else {
            output.stderr
        };
        return Err(format!(
            "Remote directory request failed (exit {}): {detail}",
            output.status
        ));
    }
    parse_remote_directory(&output.stdout)
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
        let mut runner = ProcessRemoteDirectoryRunner;
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
        || mime == "chemical/x-pdb"
        || mime == "chemical/x-mdl-molfile"
        || (mime == "application/octet-stream" && looks_like_text(&bytes))
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

/// Overwrite a text file inside the project root with edited content. Used by
/// the preview's inline editor for Markdown/plain-text files. Rejects paths
/// outside the root via the shared validator; the parent directory must exist.
pub(super) fn write_file_at(root: &Path, path: &str, content: &str) -> Result<(), String> {
    let real = wisp_tools::safety::validate_file_path(root, path)?;
    std::fs::write(&real, content).map_err(|e| format!("could not write file: {e}"))
}

#[tauri::command]
pub(super) fn write_file(
    state: State<'_, AppState>,
    window: WebviewWindow,
    path: String,
    content: String,
) -> Result<(), String> {
    write_file_at(&state.active(window.label()).root, &path, &content)
}

#[tauri::command]
pub(super) fn append_review_note(
    state: State<'_, AppState>,
    window: WebviewWindow,
    source_path: String,
    quote: String,
    note: Option<String>,
) -> Result<String, String> {
    append_review_note_at(
        &state.active(window.label()).root,
        &source_path,
        &quote,
        note.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    struct FakeRemoteDirectoryRunner {
        output: Option<Result<RemoteDirectoryOutput, String>>,
        commands: Vec<RemoteDirectoryCommand>,
    }

    impl FakeRemoteDirectoryRunner {
        fn returning(output: RemoteDirectoryOutput) -> Self {
            Self {
                output: Some(Ok(output)),
                commands: Vec::new(),
            }
        }
    }

    impl RemoteDirectoryRunner for FakeRemoteDirectoryRunner {
        fn run(
            &mut self,
            command: &RemoteDirectoryCommand,
        ) -> Result<RemoteDirectoryOutput, String> {
            self.commands.push(command.clone());
            self.output.take().expect("fake output configured")
        }
    }

    fn ssh_context() -> wisp_store::ExecutionContext {
        let mut context = wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap();
        context.config_json = serde_json::json!({
            "alias": "gpu.example",
            "user": "researcher",
            "port": 2222,
            "identity_file": "/tmp/test-key"
        })
        .to_string();
        context
    }

    #[test]
    fn remote_directory_command_uses_context_connection_and_quotes_path() {
        let command =
            build_remote_directory_command(&ssh_context(), Some("/work/O'Brien; printf unsafe"))
                .unwrap();
        assert_eq!(command.program, "ssh");
        assert!(command.args.windows(2).any(|args| args == ["-p", "2222"]));
        assert!(command
            .args
            .windows(2)
            .any(|args| args == ["-i", "/tmp/test-key"]));
        assert!(command
            .args
            .iter()
            .any(|arg| arg == "researcher@gpu.example"));
        let script = command.args.last().unwrap();
        assert!(script.contains("dir='/work/O'\"'\"'Brien; printf unsafe'"));
        assert!(script.contains("WISP_REMOTE_DIR_V1\\000"));
        assert!(script.contains("stat -c '%s'"));
        assert!(script.contains("stat -f '%z'"));
        assert!(!script.contains("wc -c"));
    }

    #[test]
    fn remote_directory_rejects_paths_with_line_breaks() {
        let error = remote_directory_script(Some("/work\nmalformed")).unwrap_err();
        assert!(error.contains("line breaks"));
    }

    #[test]
    fn remote_directory_runner_parses_banner_and_sorts_directories_first() {
        let stdout = b"login banner\nWISP_REMOTE_DIR_V1\0/home/research\0f\012\0notes.txt\0d\00\0projects\0f\03\0a.csv\0".to_vec();
        let mut runner = FakeRemoteDirectoryRunner::returning(RemoteDirectoryOutput {
            status: 0,
            stdout,
            stderr: String::new(),
        });
        let listing = list_remote_dir_with_runner(&ssh_context(), Some("~"), &mut runner).unwrap();
        assert_eq!(listing.path, "/home/research");
        assert_eq!(
            listing.entries,
            vec![
                DirEntry {
                    name: "projects".into(),
                    is_dir: true,
                    size: 0,
                },
                DirEntry {
                    name: "a.csv".into(),
                    is_dir: false,
                    size: 3,
                },
                DirEntry {
                    name: "notes.txt".into(),
                    is_dir: false,
                    size: 12,
                },
            ]
        );
        assert_eq!(runner.commands.len(), 1);
    }

    #[test]
    fn remote_directory_runner_surfaces_ssh_failure() {
        let mut runner = FakeRemoteDirectoryRunner::returning(RemoteDirectoryOutput {
            status: 255,
            stdout: Vec::new(),
            stderr: "Permission denied".into(),
        });
        let error =
            list_remote_dir_with_runner(&ssh_context(), Some("~"), &mut runner).unwrap_err();
        assert!(error.contains("exit 255"));
        assert!(error.contains("Permission denied"));
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

    #[test]
    fn write_file_overwrites_within_root_and_blocks_escape() {
        let base = std::env::temp_dir().join(format!(
            "wisp_write_file_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("notes.md"), b"# old\n").unwrap();

        write_file_at(&base, "notes.md", "# new\n\nedited body\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(base.join("notes.md")).unwrap(),
            "# new\n\nedited body\n"
        );

        // Escaping the root is refused.
        assert!(write_file_at(&base, "../escape.md", "nope").is_err());

        let _ = std::fs::remove_dir_all(&base);
    }
}
