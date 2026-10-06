use super::{terminal_ui_events, AppState};
use crate::file_browser::mime_for_path;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use tauri::{AppHandle, State};
use wisp_llm::Message;
use wisp_store::Store;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct PipPkg {
    name: String,
    #[serde(default)]
    version: String,
}

#[derive(serde::Serialize)]
struct ProvInput {
    path: String,
    produced_here: bool,
}

#[derive(serde::Serialize)]
struct ProvEnv {
    name: Option<String>,
    packages: Vec<PipPkg>,
}

#[derive(serde::Serialize)]
pub(super) struct ArtifactProvenance {
    code: String,
    language: String,
    output: String,
    exit_status: String,
    inputs: Vec<ProvInput>,
    env: Option<ProvEnv>,
}

impl ArtifactProvenance {
    pub(super) fn into_source(self) -> (String, String) {
        (self.code, self.language)
    }
}

#[derive(serde::Serialize)]
struct ExportToolResult {
    tool_call_id: String,
    tool_name: String,
    content: String,
}

#[derive(serde::Serialize)]
struct ExportToolCall {
    id: String,
    name: String,
    arguments: serde_json::Value,
    arguments_raw: String,
    result: Option<ExportToolResult>,
}

#[derive(serde::Serialize)]
struct ExportArtifactManifest {
    source_path: String,
    workspace_path: String,
    zip_path: String,
    mime: String,
    bytes: u64,
    provenance_path: Option<String>,
}

struct ExportArtifactFile {
    source_path: String,
    workspace_path: String,
    zip_path: String,
    mime: String,
    real_path: std::path::PathBuf,
    bytes: u64,
    reviewed_checksum: Option<String>,
}

#[derive(serde::Serialize)]
struct MissingExportArtifact {
    path: String,
    error: String,
}

#[derive(serde::Serialize)]
struct ExportManifest {
    session_id: String,
    exported_at: String,
    message_count: usize,
    tool_call_count: usize,
    terminal_event_count: usize,
    artifacts: Vec<ExportArtifactManifest>,
    missing_artifacts: Vec<MissingExportArtifact>,
}

/// Normalize a UI path (absolute or relative) to the workspace-relative form used
/// in `execution_log.files_written`.
pub(super) fn to_workspace_rel(root: &std::path::Path, path: &str) -> String {
    let p = std::path::Path::new(path);
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(super) fn zip_component(raw: &str) -> String {
    let s = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let s = s.trim_matches(['.', '_', '-']);
    if s.is_empty() {
        "file".into()
    } else {
        s.to_string()
    }
}

fn markdown_fence(lang: &str, body: &str) -> String {
    format!("```{lang}\n{body}\n```\n")
}

fn render_export_transcript(messages: &[Message]) -> String {
    let mut out = String::from("# wisp-science session export\n\n");
    for (idx, msg) in messages.iter().enumerate() {
        match msg.role {
            wisp_llm::Role::System => {}
            wisp_llm::Role::User => {
                out.push_str(&format!(
                    "## User {}\n\n{}\n\n",
                    idx + 1,
                    msg.content.as_text()
                ));
            }
            wisp_llm::Role::Assistant => {
                if let Some(reasoning) = msg.reasoning.as_deref().filter(|s| !s.trim().is_empty()) {
                    out.push_str("### Reasoning\n\n");
                    out.push_str(&markdown_fence("text", reasoning));
                    out.push('\n');
                }
                let text = msg.content.as_text();
                if !text.trim().is_empty() {
                    let model = msg
                        .model_name
                        .as_deref()
                        .map(|m| format!(" ({m})"))
                        .unwrap_or_default();
                    out.push_str(&format!("## Assistant{model}\n\n{text}\n\n"));
                }
                if !msg.tool_calls.is_empty() {
                    out.push_str("### Tool calls\n\n");
                    for tc in &msg.tool_calls {
                        out.push_str(&format!("- `{}` `{}`\n", tc.function.name, tc.id));
                        out.push_str(&markdown_fence("json", &tc.function.arguments));
                    }
                    out.push('\n');
                }
            }
            wisp_llm::Role::Tool => {
                let name = msg.tool_name.as_deref().unwrap_or("tool");
                out.push_str(&format!("## Tool result: {name}\n\n"));
                out.push_str(&markdown_fence("text", &msg.content.as_text()));
                out.push('\n');
            }
        }
    }
    out
}

fn export_tool_calls(messages: &[Message]) -> Vec<ExportToolCall> {
    let mut results = HashMap::<String, ExportToolResult>::new();
    for msg in messages {
        if msg.role != wisp_llm::Role::Tool {
            continue;
        }
        let Some(id) = msg.tool_call_id.clone() else {
            continue;
        };
        results.insert(
            id.clone(),
            ExportToolResult {
                tool_call_id: id,
                tool_name: msg.tool_name.clone().unwrap_or_else(|| "tool".into()),
                content: msg.content.as_text(),
            },
        );
    }

    let mut calls = vec![];
    for msg in messages {
        if msg.role != wisp_llm::Role::Assistant {
            continue;
        }
        for tc in &msg.tool_calls {
            let raw = tc.function.arguments.clone();
            let arguments = if raw.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&raw)
                    .unwrap_or_else(|_| serde_json::Value::String(raw.clone()))
            };
            calls.push(ExportToolCall {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                arguments,
                arguments_raw: raw,
                result: results.remove(&tc.id),
            });
        }
    }
    calls
}

fn collect_export_artifacts(
    root: &std::path::Path,
    artifact_paths: Vec<String>,
    stored_artifacts: Vec<(String, String, String, String, i64, Option<String>)>,
) -> (Vec<ExportArtifactFile>, Vec<MissingExportArtifact>) {
    // Validation returns physical paths. Strip the physical workspace root too,
    // including macOS /var -> /private/var, so imports receive relative paths.
    let physical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut candidates = artifact_paths;
    candidates.extend(
        stored_artifacts
            .into_iter()
            .map(|(_, _, _, path, _, _)| path),
    );

    let mut seen = HashSet::<String>::new();
    let mut files = vec![];
    let mut missing = vec![];
    for source_path in candidates {
        let real = match wisp_tools::safety::validate_file_path(root, &source_path) {
            Ok(real) => real,
            Err(error) => {
                missing.push(MissingExportArtifact {
                    path: source_path,
                    error,
                });
                continue;
            }
        };
        let workspace_path = to_workspace_rel(&physical_root, &real.to_string_lossy());
        if !seen.insert(workspace_path.clone()) {
            continue;
        }
        let bytes = match std::fs::File::open(&real).and_then(|file| {
            let metadata = file.metadata()?;
            if metadata.is_file() {
                Ok(metadata.len())
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "artifact is not a regular file",
                ))
            }
        }) {
            Ok(bytes) => bytes,
            Err(e) => {
                missing.push(MissingExportArtifact {
                    path: source_path,
                    error: format!("{e}"),
                });
                continue;
            }
        };
        let name = real
            .file_name()
            .and_then(|n| n.to_str())
            .map(zip_component)
            .unwrap_or_else(|| "artifact".into());
        let zip_path = format!("artifacts/{:03}-{name}", files.len() + 1);
        files.push(ExportArtifactFile {
            source_path,
            workspace_path,
            zip_path,
            mime: mime_for_path(&real).into(),
            real_path: real,
            bytes,
            reviewed_checksum: None,
        });
    }
    (files, missing)
}

fn zip_text<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &str,
    body: &str,
) -> Result<(), String> {
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    zip.start_file(path, opts).map_err(|e| format!("{e}"))?;
    zip.write_all(body.as_bytes()).map_err(|e| format!("{e}"))
}

#[cfg(test)]
fn zip_file<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &str,
    source: &std::path::Path,
    bytes: u64,
) -> Result<(), String> {
    zip_file_verified(zip, path, source, bytes, None)
}

fn zip_file_verified<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &str,
    source: &std::path::Path,
    bytes: u64,
    reviewed_checksum: Option<&str>,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(bytes > u32::MAX as u64)
        .unix_permissions(0o644);
    zip.start_file(path, opts).map_err(|e| format!("{e}"))?;
    let mut file = std::fs::File::open(source).map_err(|e| format!("{e}"))?;
    if !file.metadata().map_err(|e| format!("{e}"))?.is_file() {
        return Err(format!(
            "artifact is not a regular file: {}",
            source.display()
        ));
    }
    let mut copied = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        copied = copied
            .checked_add(count as u64)
            .ok_or("Export source is too large")?;
        if copied > bytes {
            break;
        }
        hash.update(&buffer[..count]);
        zip.write_all(&buffer[..count]).map_err(|e| e.to_string())?;
    }
    if copied != bytes
        || reviewed_checksum.is_some_and(|expected| expected != hex::encode(hash.finalize()))
    {
        return Err(format!(
            "artifact changed while exporting: {}",
            source.display()
        ));
    }
    Ok(())
}

fn same_existing_file(left: &std::path::Path, right: &std::path::Path) -> bool {
    same_file::is_same_file(left, right).unwrap_or(false)
}

fn zip_json<W: Write + std::io::Seek, T: serde::Serialize>(
    zip: &mut zip::ZipWriter<W>,
    path: &str,
    value: &T,
) -> Result<(), String> {
    let body = serde_json::to_string_pretty(value).map_err(|e| format!("{e}"))?;
    zip_text(zip, path, &body)
}

/// Parse `uv pip list --format=json` / `pip list --format=json` output.
fn parse_pip_list(json: &str) -> Vec<PipPkg> {
    serde_json::from_str::<Vec<PipPkg>>(json).unwrap_or_default()
}

/// Capture the kernel venv's package list once; store it hashed; return the hash.
/// Non-fatal: any failure returns `None` and the Environment panel shows "unavailable".
pub(super) async fn capture_env(
    store: &wisp_store::Store,
    app_data: &std::path::Path,
) -> Option<String> {
    let venv = app_data.join("python").join(".venv");
    let python = wisp_runtime::PythonEnv { venv }.python();
    let uv = wisp_runtime::PythonEnv::find_uv()?;
    let mut command = tokio::process::Command::new(&uv);
    command
        .args(["pip", "list", "--format=json", "--python"])
        .arg(&python);
    wisp_tools::process::hide_console_async(&mut command);
    let out = command.output().await.ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    let json = String::from_utf8_lossy(&out.stdout).into_owned();
    let packages = parse_pip_list(&json);
    if packages.is_empty() {
        return None;
    }
    let packages_json = serde_json::to_string(&packages).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&packages_json, &mut h);
    let hash = format!("{:016x}", std::hash::Hasher::finish(&h));
    store
        .record_env_snapshot(&hash, Some("kernel"), &packages_json)
        .await
        .ok()?;
    Some(hash)
}

/// Provenance for a produced artifact, addressed by workspace path. `None` when the
/// path has no recorded producing cell (uploads, pre-feature figures) → empty modal.
#[tauri::command]
pub(super) async fn get_artifact_provenance(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    session_id: Option<String>,
    path: String,
) -> Result<Option<ArtifactProvenance>, String> {
    let frame_id = match session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => Some(id.to_string()),
        None => state.active_frame(window.label()),
    };
    let Some(fid) = frame_id else { return Ok(None) };
    let ap = state.require_active(window.label())?;
    artifact_provenance_for_path(&state.store, &fid, &ap.root, &path).await
}

pub(super) async fn artifact_provenance_for_path(
    store: &Store,
    frame_id: &str,
    root: &std::path::Path,
    path: &str,
) -> Result<Option<ArtifactProvenance>, String> {
    let rel = to_workspace_rel(root, path);
    let Some(e) = store
        .find_provenance_by_path(frame_id, &rel)
        .await
        .map_err(|e| format!("{e}"))?
    else {
        return Ok(None);
    };
    let written = store
        .frame_written_paths(frame_id)
        .await
        .unwrap_or_default();
    let inputs = e
        .files_read
        .iter()
        .map(|p| ProvInput {
            path: p.clone(),
            produced_here: written.contains(p),
        })
        .collect();
    let env = match e.env_hash.as_deref() {
        Some(h) => store
            .get_env_snapshot(h)
            .await
            .ok()
            .flatten()
            .map(|(name, pj)| ProvEnv {
                name,
                packages: parse_pip_list(&pj),
            }),
        None => None,
    };
    Ok(Some(ArtifactProvenance {
        code: e.source,
        language: e.language,
        output: e.stdout,
        exit_status: e.exit_status,
        inputs,
        env,
    }))
}

/// The WebView and native host share this archive representation. It captures
/// the head context, matching the established messages.json import contract.
pub(crate) struct PreparedSessionExport {
    root: std::path::PathBuf,
    include_artifacts: bool,
    messages: Vec<Message>,
    terminal_events: Vec<serde_json::Value>,
    tool_calls: Vec<ExportToolCall>,
    files: Vec<ExportArtifactFile>,
    manifest: ExportManifest,
    provenance_files: Vec<(String, ArtifactProvenance)>,
}

pub(crate) async fn prepare_session_export(
    store: &Store,
    root: &std::path::Path,
    session_id: &str,
    artifact_paths: Vec<String>,
    include_artifacts: bool,
) -> Result<PreparedSessionExport, String> {
    let messages = store
        .load_messages(session_id)
        .await
        .map_err(|e| e.to_string())?;
    if messages.is_empty() {
        return Err("No messages to export.".into());
    }
    let terminal_events = terminal_ui_events(
        &store
            .load_session_ui_events(session_id)
            .await
            .map_err(|e| e.to_string())?,
    );
    let stored_artifacts = if include_artifacts {
        store
            .list_artifacts(session_id)
            .await
            .map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    let (files, missing_artifacts) = if include_artifacts {
        let root = root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            collect_export_artifacts(&root, artifact_paths, stored_artifacts)
        })
        .await
        .map_err(|e| e.to_string())?
    } else {
        (Vec::new(), Vec::new())
    };
    let tool_calls = export_tool_calls(&messages);
    let mut artifacts = Vec::new();
    let mut provenance_files = Vec::new();
    for file in &files {
        let stem = std::path::Path::new(&file.zip_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(zip_component)
            .unwrap_or_else(|| "artifact".into());
        let provenance_path =
            match artifact_provenance_for_path(store, session_id, root, &file.workspace_path)
                .await?
            {
                Some(provenance) => {
                    let path = format!("provenance/{stem}.json");
                    provenance_files.push((path.clone(), provenance));
                    Some(path)
                }
                None => None,
            };
        artifacts.push(ExportArtifactManifest {
            source_path: file.source_path.clone(),
            workspace_path: file.workspace_path.clone(),
            zip_path: file.zip_path.clone(),
            mime: file.mime.clone(),
            bytes: file.bytes,
            provenance_path,
        });
    }
    let manifest = ExportManifest {
        session_id: session_id.into(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        message_count: messages.len(),
        tool_call_count: tool_calls.len(),
        terminal_event_count: terminal_events.len(),
        artifacts,
        missing_artifacts,
    };
    Ok(PreparedSessionExport {
        root: root.to_path_buf(),
        include_artifacts,
        messages,
        terminal_events,
        tool_calls,
        files,
        manifest,
        provenance_files,
    })
}

impl PreparedSessionExport {
    /// Content hashes bind native previews to the bytes the writer will stream.
    /// Legacy WebView exports keep their existing snapshot-at-dialog behavior.
    pub(crate) async fn reviewed(
        mut self,
        source_revision: String,
    ) -> Result<(Self, String), String> {
        tokio::task::spawn_blocking(move || {
            for file in &mut self.files {
                let source = wisp_tools::safety::validate_file_path(
                    &self.root,
                    &file.real_path.to_string_lossy(),
                )?;
                file.reviewed_checksum = Some(hash_regular_file(&source, file.bytes)?);
            }
            let checksums: Vec<_> = self
                .files
                .iter()
                .map(|file| (&file.workspace_path, &file.reviewed_checksum))
                .collect();
            let payload = serde_json::to_vec(&(
                source_revision,
                self.include_artifacts,
                &self.messages,
                &self.terminal_events,
                &self.tool_calls,
                &self.manifest.artifacts,
                &self.manifest.missing_artifacts,
                &self.provenance_files,
                checksums,
            ))
            .map_err(|e| e.to_string())?;
            let revision = wisp_sync::sha256_hex(&payload);
            Ok((self, revision))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub(crate) fn preview(
        &self,
        project: &str,
        title: String,
        revision: String,
        head_epoch: i64,
    ) -> Result<wisp_dto::native_session_export::Preview, String> {
        use wisp_dto::native_session_export as dto;
        let artifact_bytes = self
            .files
            .iter()
            .try_fold(0u64, |sum, file| sum.checked_add(file.bytes))
            .ok_or("Export artifact sizes overflow")?;
        Ok(dto::Preview {
            schema: dto::SCHEMA.into(),
            project_id: project.into(),
            session_id: self.manifest.session_id.clone(),
            title,
            revision,
            include_artifacts: self.include_artifacts,
            head_epoch,
            message_count: self.messages.len(),
            tool_call_count: self.tool_calls.len(),
            terminal_event_count: self.terminal_events.len(),
            artifact_bytes,
            artifacts: self
                .files
                .iter()
                .map(|file| dto::Artifact {
                    path: file.workspace_path.clone(),
                    mime: file.mime.clone(),
                    bytes: file.bytes,
                })
                .collect(),
            missing_artifacts: self
                .manifest
                .missing_artifacts
                .iter()
                .map(|file| dto::MissingArtifact {
                    path: file.path.clone(),
                    error: file.error.clone(),
                })
                .collect(),
            default_filename: format!(
                "wisp-session-{}.zip",
                zip_component(&self.manifest.session_id)
            ),
        })
    }
}

fn hash_regular_file(path: &std::path::Path, bytes: u64) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Export source is not a regular file".into());
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut count = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count = count
            .checked_add(n as u64)
            .ok_or("Export source is too large")?;
        if count > bytes {
            return Err(format!(
                "Artifact changed while exporting: {}",
                path.display()
            ));
        }
        hasher.update(&buffer[..n]);
    }
    if count != bytes {
        return Err(format!(
            "Artifact changed while exporting: {}",
            path.display()
        ));
    }
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) async fn write_session_export(
    prepared: PreparedSessionExport,
    destination: std::path::PathBuf,
) -> Result<(u64, String), String> {
    tokio::task::spawn_blocking(move || write_prepared_export(prepared, &destination))
        .await
        .map_err(|e| e.to_string())?
}

fn write_prepared_export(
    prepared: PreparedSessionExport,
    destination: &std::path::Path,
) -> Result<(u64, String), String> {
    if !destination.is_absolute() || destination.is_dir() {
        return Err("Choose an absolute archive file destination".into());
    }
    if prepared
        .files
        .iter()
        .any(|file| same_existing_file(destination, &file.real_path))
    {
        return Err("Export destination cannot overwrite an exported artifact.".into());
    }
    let parent = destination
        .parent()
        .ok_or("The archive destination has no parent directory")?;
    // Stage alongside the destination so replacement uses the same filesystem.
    // A failed read/compression never truncates a previous archive.
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    {
        let mut zip = zip::ZipWriter::new(temporary.as_file_mut());
        zip_json(&mut zip, "manifest.json", &prepared.manifest)?;
        zip_text(
            &mut zip,
            "transcript.md",
            &render_export_transcript(&prepared.messages),
        )?;
        zip_json(&mut zip, "messages.json", &prepared.messages)?;
        zip_json(&mut zip, "tool-calls.json", &prepared.tool_calls)?;
        zip_json(&mut zip, "terminal-events.json", &prepared.terminal_events)?;
        for file in &prepared.files {
            let source = wisp_tools::safety::validate_file_path(
                &prepared.root,
                &file.real_path.to_string_lossy(),
            )?;
            zip_file_verified(
                &mut zip,
                &file.zip_path,
                &source,
                file.bytes,
                file.reviewed_checksum.as_deref(),
            )?;
        }
        for (path, provenance) in &prepared.provenance_files {
            zip_json(&mut zip, path, provenance)?;
        }
        zip.finish().map_err(|e| e.to_string())?;
    }
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    let bytes = temporary
        .as_file()
        .metadata()
        .map_err(|e| e.to_string())?
        .len();
    let checksum = hash_regular_file(temporary.path(), bytes)?;
    temporary
        .persist(destination)
        .map_err(|e| e.error.to_string())?;
    Ok((bytes, checksum))
}

#[tauri::command]
pub(super) async fn export_session(
    app: AppHandle,
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    session_id: String,
    artifact_paths: Vec<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let project = state.require_active(window.label())?;
    crate::native_conversations::require_owner(&state.store, &project.id, &session_id).await?;
    let prepared = prepare_session_export(
        &state.store,
        &project.root,
        &session_id,
        artifact_paths,
        true,
    )
    .await?;
    let default_name = format!("wisp-session-{}.zip", zip_component(&session_id));
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&default_name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(destination) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let destination = std::path::PathBuf::from(destination.to_string());
    write_session_export(prepared, destination.clone()).await?;
    Ok(Some(destination.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_tool_calls_matches_results_by_id() {
        let mut assistant = Message::assistant("");
        assistant.tool_calls = vec![wisp_llm::ToolCall {
            id: "call_1".into(),
            kind: "function".into(),
            function: wisp_llm::FunctionCall {
                name: "python".into(),
                arguments: r#"{"code":"print(1)"}"#.into(),
            },
        }];
        let tool = Message::tool("call_1", "python", "ok");

        let calls = export_tool_calls(&[assistant, tool]);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "python");
        assert_eq!(calls[0].arguments["code"], "print(1)");
        assert_eq!(calls[0].result.as_ref().unwrap().content, "ok");
    }

    #[test]
    fn parse_pip_list_reads_name_version() {
        let json = r#"[{"name":"numpy","version":"1.26.0"},{"name":"pandas","version":"2.2.0"}]"#;
        let pkgs = parse_pip_list(json);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "numpy");
        assert_eq!(pkgs[1].version, "2.2.0");
        assert!(parse_pip_list("not json").is_empty());
    }

    #[test]
    fn to_workspace_rel_normalizes_absolute_and_passes_relative() {
        use std::path::Path;
        let root = Path::new("/proj");
        // absolute path under root → stripped to workspace-relative
        assert_eq!(to_workspace_rel(root, "/proj/out/fig.png"), "out/fig.png");
        // already-relative path → passed through unchanged
        assert_eq!(to_workspace_rel(root, "out/fig.png"), "out/fig.png");
        // path not under root → left as-is (strip_prefix fails, falls through)
        assert_eq!(to_workspace_rel(root, "/other/x.png"), "/other/x.png");
    }

    #[test]
    fn artifact_contents_are_streamed_into_zip() {
        let root = std::env::temp_dir().join(format!("wisp_export_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("results")).unwrap();
        std::fs::write(root.join("results/data.txt"), b"stream me").unwrap();

        let (files, missing) =
            collect_export_artifacts(&root, vec!["results/data.txt".into()], vec![]);
        assert!(missing.is_empty());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].bytes, 9);

        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip_file(
            &mut zip,
            &files[0].zip_path,
            &files[0].real_path,
            files[0].bytes,
        )
        .unwrap();
        let cursor = zip.finish().unwrap();
        let mut archive = zip::ZipArchive::new(cursor).unwrap();
        let mut contents = String::new();
        std::io::Read::read_to_string(&mut archive.by_index(0).unwrap(), &mut contents).unwrap();
        assert_eq!(contents, "stream me");
        assert!(same_existing_file(
            &root.join("results/data.txt"),
            &files[0].real_path
        ));
        #[cfg(unix)]
        {
            let alias = root.join("results/data-hardlink.txt");
            std::fs::hard_link(&files[0].real_path, &alias).unwrap();
            assert!(same_existing_file(&alias, &files[0].real_path));
        }

        let _ = std::fs::remove_dir_all(root);
    }

    async fn export_store(root: &std::path::Path) -> Store {
        let store = Store::open(std::path::Path::new(":memory:")).await.unwrap();
        store
            .create_project("p", "Project", root.to_str().unwrap())
            .await
            .unwrap();
        store
            .create_frame("s", "p", "OPERON", "fake")
            .await
            .unwrap();
        store
            .append_message("s", 0, &Message::user("Saved question"))
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn reviewed_export_revision_binds_source_file_bytes_and_choice_but_not_preview_time() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("data.csv"), b"A,B\n1,2\n").unwrap();
        let store = export_store(root.path()).await;
        let prepare =
            || prepare_session_export(&store, root.path(), "s", vec!["data.csv".into()], true);
        let (first, revision) = prepare()
            .await
            .unwrap()
            .reviewed("source-v1".into())
            .await
            .unwrap();
        let preview = first
            .preview("p", "Saved title".into(), revision.clone(), 0)
            .unwrap();
        assert_eq!(preview.artifact_bytes, 8);
        assert_eq!(preview.artifacts[0].path, "data.csv");
        assert_eq!(preview.message_count, 1);
        let mut second = prepare().await.unwrap();
        second.manifest.exported_at = "later preview time".into();
        assert_eq!(
            second.reviewed("source-v1".into()).await.unwrap().1,
            revision
        );
        assert_ne!(
            prepare()
                .await
                .unwrap()
                .reviewed("source-v2".into())
                .await
                .unwrap()
                .1,
            revision
        );
        std::fs::write(root.path().join("data.csv"), b"A,B\n9,8\n").unwrap();
        assert_ne!(
            prepare()
                .await
                .unwrap()
                .reviewed("source-v1".into())
                .await
                .unwrap()
                .1,
            revision
        );
        let (without, no_files) =
            prepare_session_export(&store, root.path(), "s", vec!["data.csv".into()], false)
                .await
                .unwrap()
                .reviewed("source-v1".into())
                .await
                .unwrap();
        assert_ne!(no_files, revision);
        assert!(without.files.is_empty());
        assert!(without.manifest.missing_artifacts.is_empty());
    }

    #[tokio::test]
    async fn changed_artifacts_never_replace_destination_or_leave_a_partial_archive() {
        let root = tempfile::tempdir().unwrap();
        let store = export_store(root.path()).await;
        let source = root.path().join("data.txt");
        let destination = root.path().join("existing.zip");
        for changed in ["different", "grew much longer", "short"] {
            std::fs::write(&source, "original!").unwrap();
            std::fs::write(&destination, "previous archive").unwrap();
            let (prepared, _) =
                prepare_session_export(&store, root.path(), "s", vec!["data.txt".into()], true)
                    .await
                    .unwrap()
                    .reviewed("reviewed".into())
                    .await
                    .unwrap();
            std::fs::write(&source, changed).unwrap();
            let error = write_session_export(prepared, destination.clone())
                .await
                .unwrap_err();
            assert!(error.contains("changed while exporting"), "{error}");
            assert_eq!(std::fs::read(&destination).unwrap(), b"previous archive");
            assert_eq!(
                std::fs::read_dir(root.path()).unwrap().count(),
                2,
                "Temporary archive must be removed"
            );
        }
    }

    #[tokio::test]
    async fn export_refuses_overwriting_its_own_artifact_and_hardlink_alias() {
        let root = tempfile::tempdir().unwrap();
        let store = export_store(root.path()).await;
        let source = root.path().join("artifact.zip");
        std::fs::write(&source, b"original artifact").unwrap();
        let prepare =
            || prepare_session_export(&store, root.path(), "s", vec!["artifact.zip".into()], true);
        assert!(
            write_session_export(prepare().await.unwrap(), source.clone())
                .await
                .unwrap_err()
                .contains("overwrite")
        );
        let alias = root.path().join("alias.zip");
        std::fs::hard_link(&source, &alias).unwrap();
        assert!(write_session_export(prepare().await.unwrap(), alias)
            .await
            .unwrap_err()
            .contains("overwrite"));
        assert_eq!(std::fs::read(source).unwrap(), b"original artifact");
    }

    #[tokio::test]
    async fn export_matches_head_context_after_compaction_and_reports_missing_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let store = export_store(root.path()).await;
        store
            .open_context_epoch(
                "s",
                wisp_store::OpenContextEpoch {
                    messages: &[Message::user("[context summary checkpoint] Summary")],
                    strategy: "manual",
                    kind: "semantic",
                    before_tokens: 20,
                    after_tokens: 4,
                    checkpoint_index: Some(0),
                    first_kept_seq: None,
                    archive_ref: None,
                    ui_event_seq: None,
                },
            )
            .await
            .unwrap();
        let prepared =
            prepare_session_export(&store, root.path(), "s", vec!["gone.csv".into()], true)
                .await
                .unwrap();
        assert_eq!(prepared.manifest.message_count, 1);
        assert_eq!(
            prepared.messages[0].content.as_text(),
            "[context summary checkpoint] Summary"
        );
        assert_eq!(prepared.manifest.missing_artifacts[0].path, "gone.csv");
        assert!(prepared.files.is_empty());
        store
            .create_frame("empty", "p", "OPERON", "fake")
            .await
            .unwrap();
        assert!(
            prepare_session_export(&store, root.path(), "empty", vec![], false)
                .await
                .err()
                .unwrap()
                .contains("No messages")
        );
    }
}
