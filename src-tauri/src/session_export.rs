use super::AppState;
use crate::file_browser::mime_for_path;
use std::collections::{HashMap, HashSet};
use std::io::Write;
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
    artifacts: Vec<ExportArtifactManifest>,
    missing_artifacts: Vec<MissingExportArtifact>,
}

/// Normalize a UI path (absolute or relative) to the workspace-relative form used
/// in `execution_log.files_written`.
fn to_workspace_rel(root: &std::path::Path, path: &str) -> String {
    let p = std::path::Path::new(path);
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn zip_component(raw: &str) -> String {
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
    stored_artifacts: Vec<(String, String, String, String, i64)>,
) -> (Vec<ExportArtifactFile>, Vec<MissingExportArtifact>) {
    let mut candidates = artifact_paths;
    candidates.extend(stored_artifacts.into_iter().map(|(_, _, _, path, _)| path));

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
        let workspace_path = to_workspace_rel(root, &real.to_string_lossy());
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

fn zip_file<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &str,
    source: &std::path::Path,
) -> Result<(), String> {
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    zip.start_file(path, opts).map_err(|e| format!("{e}"))?;
    let mut file = std::fs::File::open(source).map_err(|e| format!("{e}"))?;
    std::io::copy(&mut file, zip)
        .map(|_| ())
        .map_err(|e| format!("{e}"))
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
    window: tauri::WebviewWindow,
    session_id: Option<String>,
    path: String,
) -> Result<Option<ArtifactProvenance>, String> {
    let frame_id = match session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => Some(id.to_string()),
        None => state.active_frame(window.label()),
    };
    let Some(fid) = frame_id else { return Ok(None) };
    let ap = state.active(window.label());
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

#[tauri::command]
pub(super) async fn export_session(
    app: AppHandle,
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: String,
    artifact_paths: Vec<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let messages = state
        .store
        .load_messages(&session_id)
        .await
        .map_err(|e| format!("{e}"))?;
    if messages.is_empty() {
        return Err("No messages to export.".into());
    }

    let ap = state.active(window.label());
    let stored_artifacts = state
        .store
        .list_artifacts(&session_id)
        .await
        .unwrap_or_default();
    let (files, missing_artifacts) = {
        // Resolve and stat every artifact off the async runtime. File contents are
        // streamed only after the user chooses an export destination.
        let root = ap.root.clone();
        tokio::task::spawn_blocking(move || {
            collect_export_artifacts(&root, artifact_paths, stored_artifacts)
        })
        .await
        .map_err(|e| format!("{e}"))?
    };
    let tool_calls = export_tool_calls(&messages);

    let mut artifact_manifest = Vec::<ExportArtifactManifest>::new();
    let mut provenance_files = Vec::<(String, ArtifactProvenance)>::new();
    for file in &files {
        let stem = std::path::Path::new(&file.zip_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(zip_component)
            .unwrap_or_else(|| "artifact".into());
        let provenance_path = match artifact_provenance_for_path(
            &state.store,
            &session_id,
            &ap.root,
            &file.workspace_path,
        )
        .await?
        {
            Some(prov) => {
                let path = format!("provenance/{stem}.json");
                provenance_files.push((path.clone(), prov));
                Some(path)
            }
            None => None,
        };
        artifact_manifest.push(ExportArtifactManifest {
            source_path: file.source_path.clone(),
            workspace_path: file.workspace_path.clone(),
            zip_path: file.zip_path.clone(),
            mime: file.mime.clone(),
            bytes: file.bytes,
            provenance_path,
        });
    }

    let manifest = ExportManifest {
        session_id: session_id.clone(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        message_count: messages.len(),
        tool_call_count: tool_calls.len(),
        artifacts: artifact_manifest,
        missing_artifacts,
    };

    let default_name = format!("wisp-session-{}.zip", zip_component(&session_id));
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&default_name)
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(dest) = rx.await.map_err(|e| format!("{e}"))? else {
        return Ok(None);
    };
    let dest_path = std::path::PathBuf::from(dest.to_string());
    // Compression is CPU-bound and the archive can carry many MB of
    // artifacts — keep it off the async runtime.
    let out_path = dest_path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let out = std::fs::File::create(&out_path).map_err(|e| format!("{e}"))?;
        let mut zip = zip::ZipWriter::new(out);

        zip_json(&mut zip, "manifest.json", &manifest)?;
        zip_text(
            &mut zip,
            "transcript.md",
            &render_export_transcript(&messages),
        )?;
        zip_json(&mut zip, "messages.json", &messages)?;
        zip_json(&mut zip, "tool-calls.json", &tool_calls)?;
        for file in &files {
            zip_file(&mut zip, &file.zip_path, &file.real_path)?;
        }
        for (path, provenance) in &provenance_files {
            zip_json(&mut zip, path, provenance)?;
        }
        zip.finish().map_err(|e| format!("{e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("{e}"))??;

    Ok(Some(dest_path.to_string_lossy().into_owned()))
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
        zip_file(&mut zip, &files[0].zip_path, &files[0].real_path).unwrap();
        let cursor = zip.finish().unwrap();
        let mut archive = zip::ZipArchive::new(cursor).unwrap();
        let mut contents = String::new();
        std::io::Read::read_to_string(&mut archive.by_index(0).unwrap(), &mut contents).unwrap();
        assert_eq!(contents, "stream me");

        let _ = std::fs::remove_dir_all(root);
    }
}
