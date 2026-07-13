use super::{build_project_summary, workspace_manifest, AppState, ProjectSummary};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, State};

const ARCHIVE_KIND: &str = "wisp-project";
const ARCHIVE_VERSION: u32 = 1;
const MANIFEST_PATH: &str = "manifest.json";
const DATABASE_PATH: &str = "metadata/project.sqlite";
const WORKSPACE_PREFIX: &str = "workspace";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

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

fn portable_relative(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "workspace entry escaped the project root".to_string())?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_str().ok_or_else(|| {
                "project paths must be valid Unicode to move between operating systems".to_string()
            })?),
            _ => return Err("workspace contains a non-portable path".into()),
        }
    }
    Ok(parts.join("/"))
}

#[cfg(unix)]
fn file_mode(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn file_mode(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

pub(super) fn collect_workspace(
    root: &Path,
    excluded: &Path,
) -> Result<CollectedWorkspace, String> {
    fn visit(
        root: &Path,
        directory: &Path,
        excluded: &Path,
        collected: &mut CollectedWorkspace,
    ) -> Result<(), String> {
        let mut children = std::fs::read_dir(directory)
            .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let path = child.path();
            if same_path(&path, excluded) {
                continue;
            }
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            let relative = portable_relative(root, &path)?;
            if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
                collected.skipped_paths.push(relative);
                continue;
            }
            if metadata.is_dir() {
                collected.entries.push(WorkspaceEntry {
                    source: path.clone(),
                    archive_path: relative,
                    kind: WorkspaceEntryKind::Directory,
                    size: 0,
                    mode: file_mode(&metadata),
                });
                visit(root, &path, excluded, collected)?;
            } else {
                collected.entries.push(WorkspaceEntry {
                    source: path,
                    archive_path: relative,
                    kind: WorkspaceEntryKind::File,
                    size: metadata.len(),
                    mode: file_mode(&metadata),
                });
            }
        }
        Ok(())
    }

    if !root.is_dir() {
        return Err(format!(
            "project directory does not exist: {}",
            root.display()
        ));
    }
    let mut collected = CollectedWorkspace::default();
    visit(root, root, excluded, &mut collected)?;
    Ok(collected)
}

fn zip_options(mode: Option<u32>, large: bool) -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(large)
        .unix_permissions(mode.unwrap_or(0o644))
}

fn write_project_archive(
    destination: &Path,
    database: &Path,
    workspace: &Path,
    project: ArchivedProject,
    stats: &wisp_store::ProjectTransferStats,
) -> Result<(), String> {
    let collected = collect_workspace(workspace, destination)?;
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
                    let copied = std::io::copy(&mut source, &mut zip).map_err(|error| {
                        format!("cannot archive {}: {error}", entry.source.display())
                    })?;
                    workspace_files += 1;
                    workspace_bytes = workspace_bytes.saturating_add(copied);
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

fn is_symlink_mode(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
}

fn extract_project_archive(
    archive_path: &Path,
    staging_workspace: &Path,
    database_path: &Path,
    manifest: &ProjectArchiveManifest,
) -> Result<(), String> {
    let input = std::fs::File::open(archive_path).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(input)
        .map_err(|error| format!("not a valid project archive: {error}"))?;
    let mut seen = HashSet::<PathBuf>::new();
    let mut manifest_found = false;
    let mut database_found = false;
    let mut workspace_files = 0u64;
    let mut workspace_bytes = 0u64;
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
        let copied = std::io::copy(&mut file, &mut output).map_err(|error| error.to_string())?;
        workspace_files += 1;
        workspace_bytes = workspace_bytes.saturating_add(copied);
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
    id: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let _project_activity = state.begin_project_activity(&id)?;
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
    let destination_for_task = destination.clone();
    tokio::task::spawn_blocking(move || {
        write_project_archive(
            &destination_for_task,
            &database.0,
            &workspace,
            project,
            &stats,
        )
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(Some(destination.to_string_lossy().into_owned()))
}

#[tauri::command]
pub(super) async fn import_project(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<ProjectSummary>, String> {
    let Some(archive_path) = pick_archive(&app).await? else {
        return Ok(None);
    };
    let archive_for_manifest = archive_path.clone();
    let manifest = tokio::task::spawn_blocking(move || read_manifest(&archive_for_manifest))
        .await
        .map_err(|error| error.to_string())??;
    if state
        .store
        .get_project(&manifest.project.id)
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("This project is already present on this device.".into());
    }
    let Some(parent) = pick_import_parent(&app).await? else {
        return Ok(None);
    };
    let destination = unique_destination(&parent, &manifest.project.name)?;
    let staging = TempDir(parent.join(format!(".wisp-import-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&staging.0)
        .map_err(|error| format!("cannot create import staging directory: {error}"))?;
    std::fs::create_dir_all(&state.app_data).map_err(|error| error.to_string())?;
    let database = TempFile(
        state
            .app_data
            .join(format!("project-import-{}.sqlite", uuid::Uuid::new_v4())),
    );
    let archive_for_extract = archive_path.clone();
    let staging_for_extract = staging.0.clone();
    let database_for_extract = database.0.clone();
    let manifest_for_extract = manifest.clone();
    tokio::task::spawn_blocking(move || {
        extract_project_archive(
            &archive_for_extract,
            &staging_for_extract,
            &database_for_extract,
            &manifest_for_extract,
        )
    })
    .await
    .map_err(|error| error.to_string())??;

    std::fs::rename(&staging.0, &destination)
        .map_err(|error| format!("cannot place imported project: {error}"))?;
    if let Err(error) = workspace_manifest::init_workspace_layout(
        &destination,
        &manifest.project.id,
        &manifest.project.name,
    ) {
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error);
    }
    if let Err(error) = state
        .store
        .import_project_database(&database.0, &manifest.project.id, &destination)
        .await
    {
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error.to_string());
    }
    Ok(Some(
        build_project_summary(&state, &manifest.project.id).await,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        std::fs::write(&database, b"sqlite-placeholder").unwrap();
        let stats = wisp_store::ProjectTransferStats::default();
        write_project_archive(
            &archive,
            &database,
            &workspace,
            sample_manifest().project,
            &stats,
        )
        .unwrap();
        let manifest = read_manifest(&archive).unwrap();
        assert_eq!(manifest.source_os, std::env::consts::OS);
        assert_eq!(manifest.contents.workspace_files, 1);
        assert_eq!(manifest.contents.workspace_bytes, 4);
        std::fs::create_dir_all(&extracted).unwrap();
        extract_project_archive(&archive, &extracted, &extracted_database, &manifest).unwrap();
        assert_eq!(
            std::fs::read(extracted.join("figures/plot.txt")).unwrap(),
            b"plot"
        );
        assert_eq!(
            std::fs::read(extracted_database).unwrap(),
            b"sqlite-placeholder"
        );
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
