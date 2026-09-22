use super::{
    build_project_summary, exploration_commands, workspace_manifest, workspace_scan, AppState,
    ProjectSummary,
};
use crate::workspace_surface::WorkspaceSurface;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

const ARCHIVE_KIND: &str = "wisp-project";
const ARCHIVE_VERSION: u32 = 1;
const MANIFEST_PATH: &str = "manifest.json";
const DATABASE_PATH: &str = "metadata/project.sqlite";
const WORKSPACE_PREFIX: &str = "workspace";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const PROJECT_TRANSFER_PROGRESS_EVENT: &str = "project-transfer-progress";
const COPY_PROGRESS_INTERVAL: u64 = 1024 * 1024;
const PROGRESS_EVENT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectTransferProgress {
    pub(crate) direction: &'static str,
    pub(crate) stage: &'static str,
    pub(crate) project_id: Option<String>,
    pub(crate) completed_files: u64,
    pub(crate) total_files: Option<u64>,
    pub(crate) completed_bytes: u64,
    pub(crate) total_bytes: Option<u64>,
    pub(crate) current_path: Option<String>,
}

#[derive(Clone)]
struct TransferReporter {
    app: AppHandle,
    window_label: String,
    direction: &'static str,
    state: Arc<Mutex<TransferReporterState>>,
}

#[derive(Default)]
struct TransferReporterState {
    project_id: Option<String>,
    last_stage: Option<&'static str>,
    last_emit: Option<Instant>,
}

impl TransferReporterState {
    fn should_emit(&mut self, stage: &'static str, finished: bool, now: Instant) -> bool {
        let stage_changed = self.last_stage != Some(stage);
        let interval_elapsed = self
            .last_emit
            .is_none_or(|last| now.saturating_duration_since(last) >= PROGRESS_EVENT_INTERVAL);
        if !stage_changed && !finished && !interval_elapsed {
            return false;
        }
        self.last_stage = Some(stage);
        self.last_emit = Some(now);
        true
    }
}

impl TransferReporter {
    fn new(
        app: AppHandle,
        window: &WorkspaceSurface,
        direction: &'static str,
        project_id: Option<String>,
    ) -> Self {
        Self {
            app,
            window_label: window.label().to_string(),
            direction,
            state: Arc::new(Mutex::new(TransferReporterState {
                project_id,
                ..TransferReporterState::default()
            })),
        }
    }

    fn set_project_id(&self, project_id: String) {
        if let Ok(mut state) = self.state.lock() {
            state.project_id = Some(project_id);
        }
    }

    fn report(
        &self,
        stage: &'static str,
        completed_files: u64,
        total_files: Option<u64>,
        completed_bytes: u64,
        total_bytes: Option<u64>,
        current_path: Option<&str>,
    ) {
        let finished = stage == "complete"
            || (total_files.is_some_and(|total| completed_files >= total)
                && total_bytes.is_some_and(|total| completed_bytes >= total));
        let project_id = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if !state.should_emit(stage, finished, Instant::now()) {
                return;
            }
            state.project_id.clone()
        };
        let _ = self.app.emit_to(
            &self.window_label,
            PROJECT_TRANSFER_PROGRESS_EVENT,
            ProjectTransferProgress {
                direction: self.direction,
                stage,
                project_id,
                completed_files,
                total_files,
                completed_bytes,
                total_bytes,
                current_path: current_path.map(str::to_owned),
            },
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectArchiveManifest {
    archive_kind: String,
    archive_version: u32,
    exported_at: String,
    source_os: String,
    source_app_version: String,
    project: ArchivedProject,
    contents: ArchivedContents,
    path_policy: ArchivedPathPolicy,
    skipped_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArchivedProject {
    id: String,
    name: String,
    description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ArchivedContents {
    workspace_files: u64,
    workspace_bytes: u64,
    frames: i64,
    messages: i64,
    artifacts: i64,
    runs: i64,
    path_warnings: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArchivedPathPolicy {
    workspace_paths: String,
    remote_references: String,
    machine_local_state: String,
}

#[derive(Debug, Clone)]
pub(super) enum WorkspaceEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone)]
pub(super) struct WorkspaceEntry {
    pub(super) source: PathBuf,
    pub(super) archive_path: String,
    pub(super) kind: WorkspaceEntryKind,
    pub(super) size: u64,
    pub(super) mode: Option<u32>,
}

#[derive(Default)]
pub(super) struct CollectedWorkspace {
    pub(super) entries: Vec<WorkspaceEntry>,
    pub(super) skipped_paths: Vec<String>,
}

struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{suffix}", self.0.to_string_lossy()));
            let _ = std::fs::remove_file(sidecar);
        }
    }
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn archive_component(raw: &str) -> String {
    let value = raw
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let value = value.trim_matches(['.', '_', '-']);
    if value.is_empty() {
        "project".into()
    } else {
        value.into()
    }
}

pub(super) fn directory_component(raw: &str) -> String {
    let mut value = raw
        .trim()
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '-'
            } else {
                character
            }
        })
        .collect::<String>();
    while value.ends_with([' ', '.']) {
        value.pop();
    }
    if value.is_empty() {
        "wisp-project".into()
    } else {
        value
    }
}

pub(super) fn collect_workspace(
    root: &Path,
    excluded: &Path,
) -> Result<CollectedWorkspace, String> {
    collect_workspace_capped(root, excluded, workspace_scan::MAX_WORKSPACE_ENTRIES)
}

fn collect_workspace_capped(
    root: &Path,
    excluded: &Path,
    max_entries: usize,
) -> Result<CollectedWorkspace, String> {
    let mut collected = CollectedWorkspace::default();
    let nodes = workspace_scan::scan_workspace(
        root,
        &workspace_scan::WorkspaceScanOptions {
            excluded_roots: vec![excluded.to_path_buf()],
            max_entries,
            ..workspace_scan::WorkspaceScanOptions::default()
        },
    )?;
    for node in nodes {
        match node.kind {
            workspace_scan::WorkspaceNodeKind::Directory => {
                collected.entries.push(WorkspaceEntry {
                    source: node.path,
                    archive_path: node.relative_path,
                    kind: WorkspaceEntryKind::Directory,
                    size: 0,
                    mode: node.mode,
                });
            }
            workspace_scan::WorkspaceNodeKind::File => {
                collected.entries.push(WorkspaceEntry {
                    source: node.path,
                    archive_path: node.relative_path,
                    kind: WorkspaceEntryKind::File,
                    size: node.size_bytes,
                    mode: node.mode,
                });
            }
            workspace_scan::WorkspaceNodeKind::Symlink
            | workspace_scan::WorkspaceNodeKind::Other => {
                collected.skipped_paths.push(node.relative_path);
            }
        }
    }
    Ok(collected)
}

fn zip_options(mode: Option<u32>, large: bool) -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(large)
        .unix_permissions(mode.unwrap_or(0o644))
}

fn copy_with_progress<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    reporter: Option<&TransferReporter>,
    stage: &'static str,
    current_path: &str,
    completed_files: u64,
    total_files: u64,
    completed_bytes: u64,
    total_bytes: u64,
) -> std::io::Result<u64> {
    let mut buffer = [0u8; 64 * 1024];
    let mut copied = 0u64;
    let mut last_reported = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        copied = copied.saturating_add(read as u64);
        if copied.saturating_sub(last_reported) >= COPY_PROGRESS_INTERVAL {
            if let Some(reporter) = reporter {
                reporter.report(
                    stage,
                    completed_files,
                    Some(total_files),
                    completed_bytes.saturating_add(copied),
                    Some(total_bytes),
                    Some(current_path),
                );
            }
            last_reported = copied;
        }
    }
    Ok(copied)
}

fn write_project_archive(
    destination: &Path,
    database: &Path,
    workspace: &Path,
    excluded: &Path,
    project: ArchivedProject,
    stats: &wisp_store::ProjectTransferStats,
    reporter: Option<&TransferReporter>,
) -> Result<(), String> {
    if let Some(reporter) = reporter {
        reporter.report("scanning", 0, None, 0, None, None);
    }
    let collected = collect_workspace(workspace, excluded)?;
    let total_files = collected
        .entries
        .iter()
        .filter(|entry| matches!(entry.kind, WorkspaceEntryKind::File))
        .count() as u64;
    let total_bytes = collected
        .entries
        .iter()
        .filter(|entry| matches!(entry.kind, WorkspaceEntryKind::File))
        .fold(0u64, |bytes, entry| bytes.saturating_add(entry.size));
    let result = (|| -> Result<(), String> {
        let output = std::fs::File::create(destination)
            .map_err(|error| format!("cannot create export: {error}"))?;
        let mut zip = zip::ZipWriter::new(output);
        let database_size = std::fs::metadata(database)
            .map_err(|error| error.to_string())?
            .len();
        zip.start_file(
            DATABASE_PATH,
            zip_options(None, database_size > u32::MAX as u64),
        )
        .map_err(|error| error.to_string())?;
        let mut database_file = std::fs::File::open(database).map_err(|error| error.to_string())?;
        std::io::copy(&mut database_file, &mut zip).map_err(|error| error.to_string())?;

        let mut workspace_files = 0u64;
        let mut workspace_bytes = 0u64;
        if let Some(reporter) = reporter {
            reporter.report("writing", 0, Some(total_files), 0, Some(total_bytes), None);
        }
        for entry in &collected.entries {
            let archive_path = format!("{WORKSPACE_PREFIX}/{}", entry.archive_path);
            match entry.kind {
                WorkspaceEntryKind::Directory => {
                    zip.add_directory(
                        format!("{archive_path}/"),
                        zip_options(entry.mode.or(Some(0o755)), false),
                    )
                    .map_err(|error| error.to_string())?;
                }
                WorkspaceEntryKind::File => {
                    zip.start_file(
                        archive_path,
                        zip_options(entry.mode, entry.size > u32::MAX as u64),
                    )
                    .map_err(|error| error.to_string())?;
                    let mut source = std::fs::File::open(&entry.source).map_err(|error| {
                        format!("cannot read {}: {error}", entry.source.display())
                    })?;
                    let copied = copy_with_progress(
                        &mut source,
                        &mut zip,
                        reporter,
                        "writing",
                        &entry.archive_path,
                        workspace_files,
                        total_files,
                        workspace_bytes,
                        total_bytes,
                    )
                    .map_err(|error| {
                        format!("cannot archive {}: {error}", entry.source.display())
                    })?;
                    workspace_files += 1;
                    workspace_bytes = workspace_bytes.saturating_add(copied);
                    if let Some(reporter) = reporter {
                        reporter.report(
                            "writing",
                            workspace_files,
                            Some(total_files),
                            workspace_bytes,
                            Some(total_bytes),
                            Some(&entry.archive_path),
                        );
                    }
                }
            }
        }

        let manifest = ProjectArchiveManifest {
            archive_kind: ARCHIVE_KIND.into(),
            archive_version: ARCHIVE_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            source_os: std::env::consts::OS.into(),
            source_app_version: env!("CARGO_PKG_VERSION").into(),
            project,
            contents: ArchivedContents {
                workspace_files,
                workspace_bytes,
                frames: stats.frames,
                messages: stats.messages,
                artifacts: stats.artifacts,
                runs: stats.runs,
                path_warnings: stats.path_warnings,
            },
            path_policy: ArchivedPathPolicy {
                workspace_paths: "relative-forward-slash".into(),
                remote_references: "preserved-not-reconnected".into(),
                machine_local_state: "excluded".into(),
            },
            skipped_paths: collected.skipped_paths,
        };
        let manifest_bytes =
            serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
        zip.start_file(MANIFEST_PATH, zip_options(None, false))
            .map_err(|error| error.to_string())?;
        zip.write_all(&manifest_bytes)
            .map_err(|error| error.to_string())?;
        zip.finish().map_err(|error| error.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

fn read_manifest(archive_path: &Path) -> Result<ProjectArchiveManifest, String> {
    let input = std::fs::File::open(archive_path)
        .map_err(|error| format!("cannot open project archive: {error}"))?;
    let mut zip = zip::ZipArchive::new(input)
        .map_err(|error| format!("not a valid project archive: {error}"))?;
    let mut manifest_file = zip
        .by_name(MANIFEST_PATH)
        .map_err(|_| "project archive has no manifest.json".to_string())?;
    if manifest_file.size() > MAX_MANIFEST_BYTES {
        return Err("project archive manifest is too large".into());
    }
    let mut bytes = Vec::with_capacity(manifest_file.size() as usize);
    manifest_file
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read project archive manifest: {error}"))?;
    let manifest: ProjectArchiveManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid project archive manifest: {error}"))?;
    if manifest.archive_kind != ARCHIVE_KIND || manifest.archive_version != ARCHIVE_VERSION {
        return Err(format!(
            "unsupported project archive format (kind {}, version {})",
            manifest.archive_kind, manifest.archive_version
        ));
    }
    if manifest.project.id.trim().is_empty() || manifest.project.name.trim().is_empty() {
        return Err("project archive manifest is missing project identity".into());
    }
    Ok(manifest)
}

/// Reopen a newly written archive and consume every entry before publishing it.
/// This catches a ZIP that was successfully closed but whose final directory or
/// payload does not agree with its manifest, without exposing it at the
/// user-selected filename first.
fn verify_project_archive(
    archive_path: &Path,
    manifest: &ProjectArchiveManifest,
    reporter: Option<&TransferReporter>,
) -> Result<(), String> {
    let input = std::fs::File::open(archive_path).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(input)
        .map_err(|error| format!("not a valid project archive: {error}"))?;
    let mut seen = HashSet::<PathBuf>::new();
    let mut manifest_found = false;
    let mut database_found = false;
    let mut workspace_files = 0u64;
    let mut workspace_bytes = 0u64;
    let total_files = manifest.contents.workspace_files;
    let total_bytes = manifest.contents.workspace_bytes;

    if let Some(reporter) = reporter {
        reporter.report(
            "validating",
            0,
            Some(total_files),
            0,
            Some(total_bytes),
            None,
        );
    }

    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(|error| error.to_string())?;
        let name = file.name().to_string();
        if name.contains('\\') {
            return Err("project archive contains a non-portable entry name".into());
        }
        if name == MANIFEST_PATH {
            if manifest_found || file.is_dir() || is_symlink_mode(file.unix_mode()) {
                return Err("project archive has an invalid manifest entry".into());
            }
            manifest_found = true;
            std::io::copy(&mut file, &mut std::io::sink()).map_err(|error| error.to_string())?;
            continue;
        }
        if name == DATABASE_PATH {
            if database_found || file.is_dir() || is_symlink_mode(file.unix_mode()) {
                return Err("project archive has invalid metadata".into());
            }
            database_found = true;
            std::io::copy(&mut file, &mut std::io::sink()).map_err(|error| error.to_string())?;
            continue;
        }

        let enclosed = file
            .enclosed_name()
            .ok_or_else(|| "project archive contains an unsafe path".to_string())?;
        let relative = enclosed
            .strip_prefix(WORKSPACE_PREFIX)
            .map_err(|_| format!("unexpected project archive entry: {name}"))?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        if !seen.insert(relative.to_path_buf()) || is_symlink_mode(file.unix_mode()) {
            return Err("project archive contains a duplicate or linked workspace path".into());
        }
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err("project archive contains an unsafe workspace path".into());
        }
        if file.is_dir() {
            continue;
        }
        let relative_display = relative.to_string_lossy();
        let mut sink = std::io::sink();
        let copied = copy_with_progress(
            &mut file,
            &mut sink,
            reporter,
            "validating",
            &relative_display,
            workspace_files,
            total_files,
            workspace_bytes,
            total_bytes,
        )
        .map_err(|error| error.to_string())?;
        workspace_files += 1;
        workspace_bytes = workspace_bytes.saturating_add(copied);
        if let Some(reporter) = reporter {
            reporter.report(
                "validating",
                workspace_files,
                Some(total_files),
                workspace_bytes,
                Some(total_bytes),
                Some(&relative_display),
            );
        }
    }
    if !manifest_found || !database_found {
        return Err("project archive is missing its manifest or metadata database".into());
    }
    if workspace_files != total_files || workspace_bytes != total_bytes {
        return Err("project archive workspace is incomplete or inconsistent".into());
    }
    Ok(())
}

fn is_symlink_mode(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
}

fn extract_project_archive(
    archive_path: &Path,
    staging_workspace: &Path,
    database_path: &Path,
    manifest: &ProjectArchiveManifest,
    reporter: Option<&TransferReporter>,
) -> Result<(), String> {
    let input = std::fs::File::open(archive_path).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(input)
        .map_err(|error| format!("not a valid project archive: {error}"))?;
    let mut seen = HashSet::<PathBuf>::new();
    let mut manifest_found = false;
    let mut database_found = false;
    let mut workspace_files = 0u64;
    let mut workspace_bytes = 0u64;
    let total_files = manifest.contents.workspace_files;
    let total_bytes = manifest.contents.workspace_bytes;
    if let Some(reporter) = reporter {
        reporter.report(
            "extracting",
            0,
            Some(total_files),
            0,
            Some(total_bytes),
            None,
        );
    }
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(|error| error.to_string())?;
        let name = file.name().to_string();
        if name.contains('\\') {
            return Err("project archive contains a non-portable entry name".into());
        }
        if name == MANIFEST_PATH {
            if manifest_found || file.is_dir() || is_symlink_mode(file.unix_mode()) {
                return Err("project archive has an invalid manifest entry".into());
            }
            manifest_found = true;
            continue;
        }
        if name == DATABASE_PATH {
            if database_found || file.is_dir() || is_symlink_mode(file.unix_mode()) {
                return Err("project archive has invalid metadata".into());
            }
            database_found = true;
            let mut output = std::fs::File::create(database_path)
                .map_err(|error| format!("cannot stage project metadata: {error}"))?;
            std::io::copy(&mut file, &mut output).map_err(|error| error.to_string())?;
            continue;
        }
        let enclosed = file
            .enclosed_name()
            .ok_or_else(|| "project archive contains an unsafe path".to_string())?;
        let relative = enclosed
            .strip_prefix(WORKSPACE_PREFIX)
            .map_err(|_| format!("unexpected project archive entry: {name}"))?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        if !seen.insert(relative.to_path_buf()) || is_symlink_mode(file.unix_mode()) {
            return Err("project archive contains a duplicate or linked workspace path".into());
        }
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err("project archive contains an unsafe workspace path".into());
        }
        let destination = staging_workspace.join(relative);
        if file.is_dir() {
            std::fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = std::fs::File::create(&destination)
            .map_err(|error| format!("cannot extract {}: {error}", destination.display()))?;
        let relative_display = relative.to_string_lossy();
        let copied = copy_with_progress(
            &mut file,
            &mut output,
            reporter,
            "extracting",
            &relative_display,
            workspace_files,
            total_files,
            workspace_bytes,
            total_bytes,
        )
        .map_err(|error| error.to_string())?;
        workspace_files += 1;
        workspace_bytes = workspace_bytes.saturating_add(copied);
        if let Some(reporter) = reporter {
            reporter.report(
                "extracting",
                workspace_files,
                Some(total_files),
                workspace_bytes,
                Some(total_bytes),
                Some(&relative_display),
            );
        }
        #[cfg(unix)]
        if let Some(mode) = file.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(mode & 0o777))
                .map_err(|error| error.to_string())?;
        }
    }
    if !manifest_found || !database_found {
        return Err("project archive is missing its manifest or metadata database".into());
    }
    if workspace_files != manifest.contents.workspace_files
        || workspace_bytes != manifest.contents.workspace_bytes
    {
        return Err("project archive workspace is incomplete or inconsistent".into());
    }
    Ok(())
}

fn temporary_archive_path(destination: &Path) -> Result<PathBuf, String> {
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let filename = destination
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "the export destination must include a file name".to_string())?;
    Ok(parent.join(format!(".{filename}.{}.partial", uuid::Uuid::new_v4())))
}

/// Publish a completed archive only after it has passed verification. If a
/// previous archive exists, keep it as a short-lived backup until the rename
/// succeeds so a failed publish cannot replace a known-good export.
fn publish_archive(temporary: &Path, destination: &Path) -> Result<(), String> {
    if !destination.exists() {
        return std::fs::rename(temporary, destination)
            .map_err(|error| format!("cannot publish project archive: {error}"));
    }

    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let filename = destination
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "the export destination must include a file name".to_string())?;
    let backup = parent.join(format!(".{filename}.{}.backup", uuid::Uuid::new_v4()));
    std::fs::rename(destination, &backup).map_err(|error| {
        format!("cannot prepare existing project archive for replacement: {error}")
    })?;
    if let Err(error) = std::fs::rename(temporary, destination) {
        let _ = std::fs::rename(&backup, destination);
        return Err(format!("cannot publish project archive: {error}"));
    }
    let _ = std::fs::remove_file(backup);
    Ok(())
}

pub(super) fn unique_destination(parent: &Path, name: &str) -> Result<PathBuf, String> {
    if !parent.is_dir() {
        return Err("the selected import destination is not a directory".into());
    }
    let base = directory_component(name);
    for suffix in 0..1000 {
        let candidate = if suffix == 0 {
            parent.join(&base)
        } else {
            parent.join(format!("{base}-{}", suffix + 1))
        };
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("could not choose an unused project directory".into())
}

async fn pick_archive(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Wisp project", &["zip"])
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    receiver
        .await
        .map_err(|error| error.to_string())?
        .map(|path| path.into_path().map_err(|error| error.to_string()))
        .transpose()
}

pub(super) async fn pick_import_parent(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = sender.send(path);
    });
    receiver
        .await
        .map_err(|error| error.to_string())?
        .map(|path| path.into_path().map_err(|error| error.to_string()))
        .transpose()
}

#[tauri::command]
pub(super) async fn export_project(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WorkspaceSurface,
    id: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    exploration_commands::reject_private_exploration_project_mutation(
        &state.store,
        &id,
        "Project export",
    )
    .await?;
    let reporter = TransferReporter::new(app.clone(), &window, "export", Some(id.clone()));
    reporter.report("selecting_export_destination", 0, None, 0, None, None);
    let _project_activity = state.begin_project_exclusive_activity(&id)?;
    let (name, description, workspace_dir) = state
        .store
        .get_project_meta(&id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Project not found".to_string())?;
    let running_frames = state.running_turns.lock().await.clone();
    for frame_id in running_frames {
        if state
            .store
            .frame_project_id(&frame_id)
            .await
            .map_err(|error| error.to_string())?
            .as_deref()
            == Some(id.as_str())
        {
            return Err(
                "Wait for running sessions to finish before exporting this project.".into(),
            );
        }
    }
    if state
        .store
        .list_active_runs()
        .await
        .map_err(|error| error.to_string())?
        .iter()
        .any(|run| run.project_id == id)
    {
        return Err("Wait for running jobs to finish before exporting this project.".into());
    }

    let default_name = format!("wisp-project-{}.zip", archive_component(&name));
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Wisp project", &["zip"])
        .set_file_name(&default_name)
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(destination) = receiver
        .await
        .map_err(|error| error.to_string())?
        .map(|path| path.into_path().map_err(|error| error.to_string()))
        .transpose()?
    else {
        return Ok(None);
    };

    reporter.report("preparing", 0, None, 0, None, None);
    std::fs::create_dir_all(&state.app_data).map_err(|error| error.to_string())?;
    let database = TempFile(
        state
            .app_data
            .join(format!("project-export-{}.sqlite", uuid::Uuid::new_v4())),
    );
    let stats = state
        .store
        .export_project_database(&id, &database.0)
        .await
        .map_err(|error| error.to_string())?;
    let workspace = PathBuf::from(workspace_dir);
    let project = ArchivedProject {
        id,
        name,
        description,
    };
    let temporary = temporary_archive_path(&destination)?;
    let _temporary_archive = TempFile(temporary.clone());
    let destination_for_task = destination.clone();
    let reporter_for_task = reporter.clone();
    tokio::task::spawn_blocking(move || {
        write_project_archive(
            &temporary,
            &database.0,
            &workspace,
            &destination_for_task,
            project,
            &stats,
            Some(&reporter_for_task),
        )?;
        let manifest = read_manifest(&temporary)?;
        verify_project_archive(&temporary, &manifest, Some(&reporter_for_task))?;
        reporter_for_task.report("publishing", 0, None, 0, None, None);
        publish_archive(&temporary, &destination_for_task)
    })
    .await
    .map_err(|error| error.to_string())??;
    reporter.report("complete", 0, None, 0, None, None);
    Ok(Some(destination.to_string_lossy().into_owned()))
}

#[tauri::command]
pub(super) async fn import_project(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WorkspaceSurface,
) -> Result<Option<ProjectSummary>, String> {
    let (_, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    if matches!(scope, wisp_store::StateScope::Exploration { .. }) {
        return Err(
            "exploration_project_mutation_blocked: Project import is unavailable inside an active exploration."
                .into(),
        );
    }
    let reporter = TransferReporter::new(app.clone(), &window, "import", None);
    reporter.report("selecting_archive", 0, None, 0, None, None);
    let Some(archive_path) = pick_archive(&app).await? else {
        return Ok(None);
    };
    reporter.report("reading", 0, None, 0, None, None);
    let archive_for_manifest = archive_path.clone();
    let manifest = tokio::task::spawn_blocking(move || read_manifest(&archive_for_manifest))
        .await
        .map_err(|error| error.to_string())??;
    reporter.set_project_id(manifest.project.id.clone());
    if state
        .store
        .get_project(&manifest.project.id)
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("This project is already present on this device.".into());
    }
    reporter.report("selecting_import_destination", 0, None, 0, None, None);
    let Some(parent) = pick_import_parent(&app).await? else {
        return Ok(None);
    };
    let id = import_archived_project_with_reporter(
        &state.store,
        &state.app_data,
        &archive_path,
        &parent,
        Some(&reporter),
    )
    .await?;
    let summary = build_project_summary(&state, &id).await;
    reporter.report("complete", 0, None, 0, None, None);
    Ok(Some(summary))
}

/// Place an archive into `parent` and register it. No file dialog and no
/// WebView window.
pub(crate) async fn import_archived_project(
    store: &wisp_store::Store,
    app_data: &Path,
    archive_path: &Path,
    parent: &Path,
) -> Result<String, String> {
    import_archived_project_with_reporter(store, app_data, archive_path, parent, None).await
}

async fn import_archived_project_with_reporter(
    store: &wisp_store::Store,
    app_data: &Path,
    archive_path: &Path,
    parent: &Path,
    reporter: Option<&TransferReporter>,
) -> Result<String, String> {
    let manifest = read_manifest(archive_path)?;
    if store
        .get_project(&manifest.project.id)
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("This project is already present on this device.".into());
    }
    let destination = unique_destination(parent, &manifest.project.name)?;
    let staging = TempDir(parent.join(format!(".wisp-import-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&staging.0)
        .map_err(|error| format!("cannot create import staging directory: {error}"))?;
    std::fs::create_dir_all(app_data).map_err(|error| error.to_string())?;
    let database =
        TempFile(app_data.join(format!("project-import-{}.sqlite", uuid::Uuid::new_v4())));
    let archive_for_extract = archive_path.to_path_buf();
    let staging_for_extract = staging.0.clone();
    let database_for_extract = database.0.clone();
    let manifest_for_extract = manifest.clone();
    let reporter_for_extract = reporter.cloned();
    tokio::task::spawn_blocking(move || {
        extract_project_archive(
            &archive_for_extract,
            &staging_for_extract,
            &database_for_extract,
            &manifest_for_extract,
            reporter_for_extract.as_ref(),
        )
    })
    .await
    .map_err(|error| error.to_string())??;
    if let Some(reporter) = reporter {
        reporter.report("registering", 0, None, 0, None, None);
    }
    let staging_path = staging.0.clone();
    if let Err(error) = std::fs::rename(&staging_path, &destination) {
        return Err(format!("cannot place imported project: {error}"));
    }
    // The staging guard would otherwise delete the moved directory's old path.
    std::mem::forget(staging);
    if let Err(error) = workspace_manifest::init_workspace_layout(
        &destination,
        &manifest.project.id,
        &manifest.project.name,
    ) {
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error);
    }
    if let Err(error) = store
        .import_project_database(&database.0, &manifest.project.id, &destination)
        .await
    {
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error.to_string());
    }
    Ok(manifest.project.id)
}

pub(crate) async fn export_project_archive(
    store: &wisp_store::Store,
    app_data: &Path,
    project_id: &str,
    destination: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(app_data).map_err(|error| error.to_string())?;
    let (name, description, workspace_dir) = store
        .get_project_meta(project_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Project not found".to_string())?;
    let database =
        TempFile(app_data.join(format!("project-export-{}.sqlite", uuid::Uuid::new_v4())));
    let stats = store
        .export_project_database(project_id, &database.0)
        .await
        .map_err(|error| error.to_string())?;
    let project = ArchivedProject {
        id: project_id.to_owned(),
        name,
        description,
    };
    let workspace = PathBuf::from(workspace_dir);
    let temporary = temporary_archive_path(destination)?;
    let temporary_archive = TempFile(temporary.clone());
    let destination = destination.to_path_buf();
    tokio::task::spawn_blocking(move || {
        write_project_archive(
            &temporary,
            &database.0,
            &workspace,
            &destination,
            project,
            &stats,
            None,
        )?;
        let manifest = read_manifest(&temporary)?;
        verify_project_archive(&temporary, &manifest, None)?;
        publish_archive(&temporary, &destination)
    })
    .await
    .map_err(|error| error.to_string())??;
    // publish_archive renamed the temporary zip onto `destination`.
    std::mem::forget(temporary_archive);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reporter_throttles_same_stage_but_not_stage_changes_or_completion() {
        let start = Instant::now();
        let mut state = TransferReporterState::default();
        assert!(state.should_emit("writing", false, start));
        assert!(!state.should_emit("writing", false, start + Duration::from_millis(20)));
        assert!(state.should_emit("validating", false, start + Duration::from_millis(20)));
        assert!(state.should_emit("validating", true, start + Duration::from_millis(21)));
        assert!(state.should_emit(
            "validating",
            false,
            start + PROGRESS_EVENT_INTERVAL + Duration::from_millis(21),
        ));
    }

    fn sample_manifest() -> ProjectArchiveManifest {
        ProjectArchiveManifest {
            archive_kind: ARCHIVE_KIND.into(),
            archive_version: ARCHIVE_VERSION,
            exported_at: "2026-07-12T00:00:00Z".into(),
            source_os: "windows".into(),
            source_app_version: "0.10.0".into(),
            project: ArchivedProject {
                id: "project-1".into(),
                name: "Cross-platform study".into(),
                description: String::new(),
            },
            contents: ArchivedContents::default(),
            path_policy: ArchivedPathPolicy {
                workspace_paths: "relative-forward-slash".into(),
                remote_references: "preserved-not-reconnected".into(),
                machine_local_state: "excluded".into(),
            },
            skipped_paths: vec![],
        }
    }

    #[test]
    fn archive_roundtrip_preserves_workspace_files() {
        let token = uuid::Uuid::new_v4();
        let base = std::env::temp_dir().join(format!("wisp_project_archive_{token}"));
        let workspace = base.join("source");
        let extracted = base.join("extracted");
        let database = base.join("project.sqlite");
        let extracted_database = base.join("extracted.sqlite");
        let archive = base.join("project.zip");
        std::fs::create_dir_all(workspace.join("figures")).unwrap();
        std::fs::write(workspace.join("figures/plot.txt"), b"plot").unwrap();
        let snapshot = workspace.join(".wisp/artifacts/sha256/ab/abcdef.png");
        std::fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        std::fs::write(&snapshot, b"snapshot").unwrap();
        let candidate_blob = workspace.join(".wisp/method-search/run-1/blobs/ca/candidate.py");
        std::fs::create_dir_all(candidate_blob.parent().unwrap()).unwrap();
        std::fs::write(&candidate_blob, b"candidate").unwrap();
        std::fs::write(&database, b"sqlite-placeholder").unwrap();
        let stats = wisp_store::ProjectTransferStats::default();
        write_project_archive(
            &archive,
            &database,
            &workspace,
            &archive,
            sample_manifest().project,
            &stats,
            None,
        )
        .unwrap();
        let manifest = read_manifest(&archive).unwrap();
        assert_eq!(manifest.source_os, std::env::consts::OS);
        assert_eq!(manifest.contents.workspace_files, 3);
        assert_eq!(manifest.contents.workspace_bytes, 21);
        verify_project_archive(&archive, &manifest, None).unwrap();
        std::fs::create_dir_all(&extracted).unwrap();
        extract_project_archive(&archive, &extracted, &extracted_database, &manifest, None)
            .unwrap();
        assert_eq!(
            std::fs::read(extracted.join("figures/plot.txt")).unwrap(),
            b"plot"
        );
        assert_eq!(
            std::fs::read(extracted.join(".wisp/artifacts/sha256/ab/abcdef.png")).unwrap(),
            b"snapshot"
        );
        assert_eq!(
            std::fs::read(extracted.join(".wisp/method-search/run-1/blobs/ca/candidate.py"))
                .unwrap(),
            b"candidate"
        );
        assert_eq!(
            std::fs::read(extracted_database).unwrap(),
            b"sqlite-placeholder"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn temporary_archive_is_hidden_beside_the_final_destination() {
        let destination = Path::new("C:/exports/wisp-project-study.zip");
        let temporary = temporary_archive_path(destination).unwrap();
        assert_eq!(temporary.parent(), destination.parent());
        assert!(temporary
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".wisp-project-study.zip."));
        assert!(temporary
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".partial"));
    }

    #[test]
    fn verification_rejects_a_manifest_that_does_not_match_the_workspace() {
        let base = std::env::temp_dir().join(format!(
            "wisp_project_archive_mismatch_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let archive = base.join("mismatch.zip");
        let mut manifest = sample_manifest();
        manifest.contents.workspace_files = 2;
        manifest.contents.workspace_bytes = 2;

        let output = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(output);
        zip.start_file(DATABASE_PATH, zip_options(None, false))
            .unwrap();
        zip.write_all(b"sqlite").unwrap();
        zip.start_file("workspace/figures/plot.txt", zip_options(None, false))
            .unwrap();
        zip.write_all(b"x").unwrap();
        zip.start_file(MANIFEST_PATH, zip_options(None, false))
            .unwrap();
        zip.write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        zip.finish().unwrap();

        let manifest = read_manifest(&archive).unwrap();
        assert_eq!(
            verify_project_archive(&archive, &manifest, None).unwrap_err(),
            "project archive workspace is incomplete or inconsistent"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_collection_stops_at_the_entry_limit() {
        let base =
            std::env::temp_dir().join(format!("wisp_project_entry_limit_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        for name in ["a", "b", "c"] {
            std::fs::write(base.join(name), b"x").unwrap();
        }

        let error = match collect_workspace_capped(&base, &base.join("excluded"), 2) {
            Ok(_) => panic!("workspace entry limit was not enforced"),
            Err(error) => error,
        };
        assert!(error.contains("more than 2 entries"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn destination_folder_is_cross_platform_safe_and_non_destructive() {
        let base =
            std::env::temp_dir().join(format!("wisp_project_destination_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(base.join("A-B")).unwrap();
        assert_eq!(directory_component(r#"A:B*"#), "A-B-");
        assert_eq!(
            unique_destination(&base, r#"A:B*"#).unwrap(),
            base.join("A-B-")
        );
        assert_eq!(
            unique_destination(&base, "A-B").unwrap(),
            base.join("A-B-2")
        );
        let _ = std::fs::remove_dir_all(base);
    }
}
