use super::{
    clear_idle_agents, clear_idle_agents_for_project, effective_enabled_skill_names,
    load_enabled_skill_names, load_skill_index, load_skill_tags, normalize_tags,
    project_skill_catalog, save_enabled_skill_names, save_skill_tags, skill_infos, AppState,
    SkillInfo,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, State};
use wisp_skills::{SkillCatalogRecord, SkillSource};

async fn list_skill_infos_for_project(
    state: &AppState,
    label: &str,
) -> Result<Vec<SkillInfo>, String> {
    let ap = state.require_active(label)?;
    let tags = load_skill_tags(&state.store).await;
    let (all, enabled) = project_skill_catalog(&state.store, &ap).await;
    let plugin_roots = crate::plugins::enabled_plugin_manifests(&state.store, &ap.id)
        .await
        .into_iter()
        .flat_map(|(installation, manifest)| {
            let display_name = manifest.display_name.clone();
            manifest
                .skill_paths(Path::new(&installation.install_root))
                .into_iter()
                .map(move |path| (path, display_name.clone()))
        })
        .collect::<Vec<_>>();
    let mut infos = skill_infos(&all, &tags, enabled.as_ref());
    for info in &mut infos {
        if let Some((_, display_name)) = plugin_roots
            .iter()
            .find(|(root, _)| Path::new(&info.dir).starts_with(root))
        {
            // Managed by its parent plugin; removal happens from the plugin
            // card so files and project bindings stay consistent.
            info.builtin = true;
            info.managed = true;
            info.managed_by = Some(display_name.clone());
        }
    }
    Ok(infos)
}

#[tauri::command]
pub(super) async fn list_skills(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
) -> Result<Vec<SkillInfo>, String> {
    list_skill_infos_for_project(&state, window.label()).await
}

#[tauri::command]
pub(super) async fn list_skill_files(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    name: String,
) -> Result<Vec<String>, String> {
    let ap = state.require_active(window.label())?;
    let (catalog, _) = project_skill_catalog(&state.store, &ap).await;
    let root = catalog
        .get(&name)
        .ok_or("Skill not found in this project.")?
        .dir
        .clone();
    tokio::task::spawn_blocking(move || wisp_skills::files::list_skill_files(&root))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(super) async fn read_skill_file(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    name: String,
    path: String,
) -> Result<wisp_dto::SkillFileContent, String> {
    let ap = state.require_active(window.label())?;
    let (catalog, _) = project_skill_catalog(&state.store, &ap).await;
    let root = catalog
        .get(&name)
        .ok_or("Skill not found in this project.")?
        .dir
        .clone();
    tokio::task::spawn_blocking(move || {
        let content = wisp_skills::files::read_skill_file(&root, &path)?;
        Ok(wisp_dto::SkillFileContent { path, content })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(super) async fn reload_skills(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
) -> Result<Vec<SkillInfo>, String> {
    let label = window.label();
    let mut project = state.require_active(label)?;
    let previous_names = project
        .skills
        .all()
        .iter()
        .map(|skill| skill.name.clone())
        .collect::<HashSet<_>>();
    let mut enabled = effective_enabled_skill_names(&state.store, &project).await;
    project.skills = Arc::new(load_skill_index(&project.root));

    enabled = enabled_names_after_reload(
        enabled,
        &previous_names,
        project.skills.all().iter().map(|skill| skill.name.as_str()),
    );
    if let Some(names) = &enabled {
        save_enabled_skill_names(&state.store, &project.id, names).await?;
    }

    let project_id = project.id.clone();
    state.set_active(label, project);
    clear_idle_agents_for_project(&state, &project_id).await;
    list_skill_infos_for_project(&state, label).await
}

fn enabled_names_after_reload<'a>(
    enabled: Option<HashSet<String>>,
    previous_names: &HashSet<String>,
    discovered_names: impl IntoIterator<Item = &'a str>,
) -> Option<HashSet<String>> {
    enabled.map(|mut names| {
        names.extend(
            discovered_names
                .into_iter()
                .filter(|name| !previous_names.contains(*name))
                .map(str::to_string),
        );
        names
    })
}

#[tauri::command]
pub(super) async fn set_skill_tags(
    state: State<'_, AppState>,
    name: String,
    tags: Vec<String>,
) -> Result<(), String> {
    let mut all_tags = load_skill_tags(&state.store).await;
    let tags = normalize_tags(tags);
    if tags.is_empty() {
        all_tags.remove(&name);
    } else {
        all_tags.insert(name, tags);
    }
    save_skill_tags(&state.store, &all_tags).await?;
    clear_idle_agents(&state).await;
    Ok(())
}

async fn update_skills_enabled(
    state: &AppState,
    label: &str,
    names: Vec<String>,
    enabled: bool,
) -> Result<(), String> {
    let ap = state.require_active(label)?;
    let mut current = effective_enabled_skill_names(&state.store, &ap)
        .await
        .unwrap_or_else(|| ap.skills.all().iter().map(|s| s.name.clone()).collect());
    let known = ap
        .skills
        .all()
        .iter()
        .map(|s| s.name.as_str())
        .collect::<HashSet<_>>();
    for name in names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && known.contains(n.as_str()))
    {
        if enabled {
            current.insert(name);
        } else {
            current.remove(&name);
        }
    }
    save_enabled_skill_names(&state.store, &ap.id, &current).await?;
    clear_idle_agents_for_project(state, &ap.id).await;
    Ok(())
}

#[tauri::command]
pub(super) async fn set_skill_enabled(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    update_skills_enabled(&state, window.label(), vec![name], enabled).await
}

#[tauri::command]
pub(super) async fn set_skills_enabled(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    names: Vec<String>,
    enabled: bool,
) -> Result<(), String> {
    update_skills_enabled(&state, window.label(), names, enabled).await
}

#[tauri::command]
pub(super) async fn pick_skill_source(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    // Folder picking is offered via a second button in the UI.
    app.dialog()
        .file()
        .add_filter("Wisp skill", &["md", "zip"])
        .pick_file(move |p| {
            let _ = tx.send(p);
        });
    let picked = rx.await.map_err(|e| format!("{e}"))?;
    Ok(picked.map(|fp| fp.to_string()))
}

pub(super) fn user_skills_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|h| h.join(".wisp").join("skills"))
        .ok_or_else(|| "no home directory".to_string())
}

/// Project-scoped Skills live inside the project folder and move or disappear
/// with it.
pub(super) fn project_skills_dir(root: &Path) -> PathBuf {
    root.join(".wisp").join("skills")
}

/// Discovery precedence, lowest first; mirrors the order in `skill_sources`.
fn scope_rank(scope: SkillSource) -> u8 {
    match scope {
        SkillSource::Bundled => 0,
        SkillSource::Project => 1,
        SkillSource::Global => 2,
        SkillSource::Extra => 3,
        SkillSource::Plugin => 4,
        SkillSource::Custom => 5,
    }
}

fn record_dir(record: &SkillCatalogRecord) -> PathBuf {
    record.path.parent().map(PathBuf::from).unwrap_or_default()
}

fn named_records<'a>(
    records: &'a [SkillCatalogRecord],
    name: &'a str,
) -> impl Iterator<Item = &'a SkillCatalogRecord> {
    records
        .iter()
        .filter(move |record| record.parse_error.is_none() && record.name == name)
}

fn skill_copy(record: &SkillCatalogRecord, effective_hash: Option<&str>) -> wisp_dto::SkillCopy {
    wisp_dto::SkillCopy {
        scope: record.scope.as_str().to_string(),
        dir: record_dir(record).to_string_lossy().to_string(),
        effective: record.effective,
        same_as_effective: record.effective
            || (effective_hash.is_some() && record.skill_md_sha256.as_deref() == effective_hash),
    }
}

/// Every discovered copy of `name`, active copy first and the rest in
/// discovery precedence, so the UI can show which one wins.
pub(super) fn skill_copies(records: &[SkillCatalogRecord], name: &str) -> Vec<wisp_dto::SkillCopy> {
    let mut named = named_records(records, name).collect::<Vec<_>>();
    named.sort_by_key(|record| (!record.effective, scope_rank(record.scope)));
    let effective_hash = named
        .iter()
        .find(|record| record.effective)
        .and_then(|record| record.skill_md_sha256.as_deref());
    named
        .into_iter()
        .map(|record| skill_copy(record, effective_hash))
        .collect()
}

/// Build the receipt for a package installed at `dir`, naming the copy that
/// still wins discovery when the new one is shadowed.
fn install_receipt(
    records: &[SkillCatalogRecord],
    name: &str,
    scope: SkillSource,
    dir: &Path,
) -> wisp_dto::SkillInstallReceipt {
    let installed_effective = named_records(records, name)
        .find(|record| record_dir(record) == dir)
        .is_none_or(|record| record.effective);
    let shadowed_by = (!installed_effective)
        .then(|| {
            skill_copies(records, name)
                .into_iter()
                .find(|copy| copy.effective)
        })
        .flatten();
    wisp_dto::SkillInstallReceipt {
        name: name.to_string(),
        scope: scope.as_str().to_string(),
        dir: dir.to_string_lossy().to_string(),
        shadowed_by,
    }
}

#[tauri::command]
pub(super) async fn skill_install_locations(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
) -> Result<wisp_dto::SkillInstallLocations, String> {
    let ap = state.require_active(window.label())?;
    Ok(wisp_dto::SkillInstallLocations {
        global_dir: user_skills_dir()?.to_string_lossy().to_string(),
        project_dir: project_skills_dir(&ap.root).to_string_lossy().to_string(),
    })
}

/// Reject skill names that could escape the skills directory. A valid name is a
/// single path component: no separators, no `..`, non-empty.
pub(super) fn validate_skill_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("skill name is empty".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!("invalid skill name '{name}'"));
    }
    // Must be exactly one path component (defends against platform-specific tricks).
    if std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        != Some(name)
    {
        return Err(format!("invalid skill name '{name}'"));
    }
    Ok(())
}

fn install_scope(scope: Option<&str>) -> Result<SkillSource, String> {
    match scope.unwrap_or("global") {
        "global" => Ok(SkillSource::Global),
        "project" => Ok(SkillSource::Project),
        other => Err(format!("unsupported skill install scope '{other}'")),
    }
}

#[tauri::command]
pub(super) async fn install_skill(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    src_path: String,
    scope: Option<String>,
) -> Result<wisp_dto::SkillInstallReceipt, String> {
    let scope = install_scope(scope.as_deref())?;
    let src = PathBuf::from(&src_path);
    let skills_dir = match scope {
        SkillSource::Project => project_skills_dir(&state.require_active(window.label())?.root),
        _ => user_skills_dir()?,
    };
    let target_dir = skills_dir.clone();
    // ZIP extraction, recursive copy, and the atomic directory swap run off
    // the async runtime. Existing user-added skills are replaced so importing
    // an updated package is a normal upgrade path.
    let skill_name = tokio::task::spawn_blocking(move || install_skill_source(&src, &skills_dir))
        .await
        .map_err(|e| format!("{e}"))??;
    reload_host_skill_index(&state, window.label())?;
    let ap = state.require_active(window.label())?;
    if let Some(mut enabled) = load_enabled_skill_names(&state.store, &ap.id).await {
        enabled.insert(skill_name.clone());
        save_enabled_skill_names(&state.store, &ap.id, &enabled).await?;
    }
    clear_idle_agents(&state).await;
    Ok(install_receipt(
        ap.skills.catalog_records(),
        &skill_name,
        scope,
        &target_dir.join(&skill_name),
    ))
}

/// Copy a project Skill into the global directory. The project copy, the
/// enabled set, and tag overrides are untouched: both copies share the public
/// name those settings are keyed by.
#[tauri::command]
pub(super) async fn promote_skill_to_global(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    name: String,
) -> Result<wisp_dto::SkillInstallReceipt, String> {
    validate_skill_name(&name)?;
    let ap = state.require_active(window.label())?;
    let records = ap.skills.catalog_records();
    let source = named_records(records, &name)
        .find(|record| record.scope == SkillSource::Project)
        .map(record_dir)
        .ok_or("skill_not_in_project")?;
    if let Some(existing) =
        named_records(records, &name).find(|record| record.scope == SkillSource::Global)
    {
        return Err(format!(
            "skill_global_exists:{}",
            record_dir(existing).display()
        ));
    }
    let dest = user_skills_dir()?.join(&name);
    let target = dest.clone();
    tokio::task::spawn_blocking(move || promote_skill_dir(&source, &target))
        .await
        .map_err(|e| format!("{e}"))??;
    reload_host_skill_index(&state, window.label())?;
    clear_idle_agents(&state).await;
    let ap = state.require_active(window.label())?;
    Ok(install_receipt(
        ap.skills.catalog_records(),
        &name,
        SkillSource::Global,
        &dest,
    ))
}

/// Never overwrite: an occupied global folder is a conflict the user resolves.
fn promote_skill_dir(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        return Err(format!("skill_global_exists:{}", to.display()));
    }
    install_skill_dir(from, to)
        .map(|_| ())
        .map_err(|error| format!("promote skill: {error}"))
}

/// Delete the project copy of a Skill only when another copy keeps the name
/// available: a global copy, or any other copy with identical SKILL.md.
#[tauri::command]
pub(super) async fn remove_project_skill_copy(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    name: String,
) -> Result<(), String> {
    let ap = state.require_active(window.label())?;
    let dir = removable_project_copy(
        ap.skills.catalog_records(),
        &name,
        &project_skills_dir(&ap.root),
    )?;
    tokio::fs::remove_dir_all(&dir)
        .await
        .map_err(|e| format!("{e}"))?;
    reload_host_skill_index(&state, window.label())?;
    clear_idle_agents_for_project(&state, &ap.id).await;
    Ok(())
}

/// Names of the Skill packages under a project's `.wisp/skills`, including
/// ones that fail to parse: deleting the project folder deletes them all.
fn project_skill_package_names(root: &Path) -> Vec<String> {
    let index =
        wisp_skills::SkillIndex::load_scoped(&[(project_skills_dir(root), SkillSource::Project)]);
    let mut names = index
        .catalog_records()
        .iter()
        .map(|record| record.name.clone())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

/// Project Skills that "Delete project and local data" would remove. Looked up
/// by id because deletion starts from the projects list, where the project is
/// usually not the active one. An unknown project has nothing to warn about.
#[tauri::command]
pub(super) async fn list_project_skill_packages(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<String>, String> {
    let Some((_, workspace_dir)) = state
        .store
        .get_project(&id)
        .await
        .map_err(|e| format!("{e}"))?
    else {
        return Ok(vec![]);
    };
    if workspace_dir.trim().is_empty() {
        return Ok(vec![]);
    }
    tokio::task::spawn_blocking(move || project_skill_package_names(Path::new(&workspace_dir)))
        .await
        .map_err(|e| format!("{e}"))
}

fn removable_project_copy(
    records: &[SkillCatalogRecord],
    name: &str,
    project_dir: &Path,
) -> Result<PathBuf, String> {
    let project = named_records(records, name)
        .find(|record| record.scope == SkillSource::Project)
        .ok_or("skill_not_in_project")?;
    let dir = record_dir(project);
    if !dir.starts_with(project_dir) || dir == project_dir {
        return Err("skill_not_in_project".into());
    }
    let kept = named_records(records, name).any(|record| {
        record.scope != SkillSource::Project
            && (record.scope == SkillSource::Global
                || (project.skill_md_sha256.is_some()
                    && record.skill_md_sha256 == project.skill_md_sha256))
    });
    if !kept {
        return Err("skill_project_copy_is_only_copy".into());
    }
    Ok(dir)
}

struct SkillImportTempDir(PathBuf);

impl Drop for SkillImportTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn install_skill_source(src: &Path, skills_dir: &Path) -> Result<String, String> {
    let (_temp_dir, skill_dir, skill_md) = if src.is_dir() {
        let md = src.join("SKILL.md");
        if !md.is_file() {
            return Err("selected folder has no SKILL.md".into());
        }
        (None, src.to_path_buf(), md)
    } else if src.file_name().is_some_and(|name| name == "SKILL.md") {
        (
            None,
            src.parent().map(PathBuf::from).unwrap_or_default(),
            src.to_path_buf(),
        )
    } else if src
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        let temp_root =
            std::env::temp_dir().join(format!("wisp-skill-import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp_root)
            .map_err(|error| format!("create skill ZIP staging directory: {error}"))?;
        let temp_dir = SkillImportTempDir(temp_root.clone());
        let fallback = src
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| validate_skill_name(name).is_ok())
            .unwrap_or("skill");
        let unpacked = temp_root.join(fallback);
        std::fs::create_dir(&unpacked)
            .map_err(|error| format!("create skill ZIP extraction directory: {error}"))?;
        crate::plugins::extract_zip(src, &unpacked)?;
        let skill_dir = find_archived_skill_dir(&unpacked)?;
        let skill_md = skill_dir.join("SKILL.md");
        (Some(temp_dir), skill_dir, skill_md)
    } else {
        return Err("select a skill folder, a SKILL.md file, or a ZIP archive".into());
    };

    let skill = wisp_skills::parse_skill_file(&skill_md)?;
    if skill.description.trim().is_empty() {
        return Err("SKILL.md is missing a description".into());
    }
    validate_skill_name(&skill.name)?;
    let dest = skills_dir.join(&skill.name);
    install_skill_dir(&skill_dir, &dest).map_err(|error| format!("install skill: {error}"))?;
    Ok(skill.name)
}

fn find_archived_skill_dir(root: &Path) -> Result<PathBuf, String> {
    if root.join("SKILL.md").is_file() {
        return Ok(root.to_path_buf());
    }
    let mut candidates = Vec::new();
    for entry in
        std::fs::read_dir(root).map_err(|error| format!("read skill ZIP contents: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read skill ZIP entry: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| format!("inspect skill ZIP entry: {error}"))?
            .is_dir()
            && entry.path().join("SKILL.md").is_file()
        {
            candidates.push(entry.path());
        }
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err("skill ZIP has no SKILL.md at its root or in a top-level folder".into()),
        _ => Err("skill ZIP contains more than one skill folder".into()),
    }
}

#[tauri::command]
pub(super) async fn remove_skill(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    name: String,
) -> Result<(), String> {
    validate_skill_name(&name)?;
    let dir = user_skills_dir()?.join(&name);
    if !dir.is_dir() {
        return Err("only user-added skills can be removed".into());
    }
    tokio::fs::remove_dir_all(&dir)
        .await
        .map_err(|e| format!("{e}"))?;
    let ap = state.require_active(window.label())?;
    if let Some(mut enabled) = load_enabled_skill_names(&state.store, &ap.id).await {
        enabled.remove(&name);
        let _ = save_enabled_skill_names(&state.store, &ap.id, &enabled).await;
    }
    let mut tags = load_skill_tags(&state.store).await;
    tags.remove(&name);
    let _ = save_skill_tags(&state.store, &tags).await;
    reload_host_skill_index(&state, window.label())?;
    clear_idle_agents(&state).await;
    Ok(())
}

fn reload_host_skill_index(state: &AppState, label: &str) -> Result<(), String> {
    let mut ap = state.require_active(label)?;
    ap.skills = Arc::new(load_skill_index(&ap.root));
    state.set_active(label, ap);
    Ok(())
}

pub(super) fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            // A link could point outside the skill folder (e.g. ~/.ssh/id_ed25519)
            // and end up readable by the model once installed.
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "skill folder contains a symlink: {}",
                    entry.path().display()
                ),
            ));
        }
        if kind.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

/// Install a skill directory, replacing an existing user-installed copy.
///
/// The new tree is copied to a sibling staging directory first. Only after the
/// copy succeeds do we move the old tree aside and atomically put the staged
/// tree in its place. This prevents a failed update from leaving a partially
/// copied skill behind.
fn install_skill_dir(from: &Path, to: &Path) -> Result<bool, String> {
    let parent = to
        .parent()
        .ok_or_else(|| "skill destination has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;

    if to.exists() {
        if !to.is_dir() {
            return Err(format!(
                "skill destination '{}' is not a directory",
                to.display()
            ));
        }
        if same_file::is_same_file(from, to).unwrap_or(false) {
            return Ok(true);
        }
    }

    let name = to
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("skill");
    let token = uuid::Uuid::new_v4();
    let staging = parent.join(format!(".{name}-install-{token}"));
    let backup = parent.join(format!(".{name}-backup-{token}"));

    if let Err(error) = copy_dir_recursive(from, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error.to_string());
    }

    let replaced = to.exists();
    if replaced {
        if let Err(error) = std::fs::rename(to, &backup) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error.to_string());
        }
    }

    if let Err(error) = std::fs::rename(&staging, to) {
        let restore_error = if replaced {
            std::fs::rename(&backup, to).err()
        } else {
            None
        };
        let _ = std::fs::remove_dir_all(&staging);
        return match restore_error {
            Some(restore_error) => Err(format!(
                "{error}; restoring the previous skill also failed: {restore_error}"
            )),
            None => Err(error.to_string()),
        };
    }

    if replaced {
        if let Err(error) = std::fs::remove_dir_all(&backup) {
            tracing::warn!(
                path = %backup.display(),
                %error,
                "could not remove replaced skill backup"
            );
        }
    }
    Ok(replaced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("wisp-skill-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn installing_same_named_skill_replaces_the_existing_tree() {
        let temp = TestDir::new();
        let source = temp.0.join("source");
        let destination = temp.0.join("installed").join("example");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(source.join("SKILL.md"), "new instructions").unwrap();
        std::fs::write(source.join("new.txt"), "new resource").unwrap();
        std::fs::write(destination.join("SKILL.md"), "old instructions").unwrap();
        std::fs::write(destination.join("stale.txt"), "stale resource").unwrap();

        assert_eq!(install_skill_dir(&source, &destination), Ok(true));
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            "new instructions"
        );
        assert!(destination.join("new.txt").is_file());
        assert!(!destination.join("stale.txt").exists());
    }

    #[test]
    fn failed_skill_copy_keeps_the_existing_tree() {
        let temp = TestDir::new();
        let source = temp.0.join("missing-source");
        let destination = temp.0.join("installed").join("example");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("SKILL.md"), "old instructions").unwrap();

        assert!(install_skill_dir(&source, &destination).is_err());
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            "old instructions"
        );
    }

    #[test]
    fn zip_skill_import_installs_a_single_top_level_skill_folder() {
        let temp = TestDir::new();
        let archive_path = temp.0.join("ggtree-visualization.zip");
        let file = std::fs::File::create(&archive_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "ggtree-visualization/SKILL.md",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive
            .write_all(b"---\nname: ggtree-visualization\ndescription: Draw trees\n---\n# Skill")
            .unwrap();
        archive
            .start_file(
                "ggtree-visualization/assets/template.R",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(b"plot(tree)").unwrap();
        archive.finish().unwrap();

        let installed = temp.0.join("installed");
        assert_eq!(
            install_skill_source(&archive_path, &installed),
            Ok("ggtree-visualization".into())
        );
        assert!(installed.join("ggtree-visualization/SKILL.md").is_file());
        assert_eq!(
            std::fs::read_to_string(installed.join("ggtree-visualization/assets/template.R"))
                .unwrap(),
            "plot(tree)"
        );
    }

    #[test]
    fn zip_skill_import_rejects_multiple_skill_folders() {
        let temp = TestDir::new();
        let archive_path = temp.0.join("skills.zip");
        let file = std::fs::File::create(&archive_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        for name in ["one", "two"] {
            archive
                .start_file(
                    format!("{name}/SKILL.md"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            archive
                .write_all(format!("---\nname: {name}\ndescription: Test\n---\n").as_bytes())
                .unwrap();
        }
        archive.finish().unwrap();

        let error = install_skill_source(&archive_path, &temp.0.join("installed")).unwrap_err();
        assert_eq!(error, "skill ZIP contains more than one skill folder");
    }

    fn write_skill(base: &Path, dir: &str, name: &str, body: &str) -> PathBuf {
        let dir = base.join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Test skill\n---\n{body}"),
        )
        .unwrap();
        dir
    }

    fn scoped_index(temp: &TestDir) -> (wisp_skills::SkillIndex, PathBuf, PathBuf, PathBuf) {
        let bundled = temp.0.join("bundled");
        let project = temp.0.join("project").join(".wisp").join("skills");
        let global = temp.0.join("home").join(".wisp").join("skills");
        let index = wisp_skills::SkillIndex::load_scoped(&[
            (bundled.clone(), SkillSource::Bundled),
            (project.clone(), SkillSource::Project),
            (global.clone(), SkillSource::Global),
        ]);
        (index, bundled, project, global)
    }

    #[test]
    fn skill_copies_list_the_active_copy_first_and_flag_identical_duplicates() {
        let temp = TestDir::new();
        let (_, bundled, project, global) = scoped_index(&temp);
        write_skill(&bundled, "search", "search", "same");
        write_skill(&project, "search", "search", "same");
        write_skill(&global, "search", "search", "edited");
        let (index, ..) = scoped_index(&temp);

        let copies = skill_copies(index.catalog_records(), "search");
        let scopes = copies.iter().map(|c| c.scope.as_str()).collect::<Vec<_>>();
        assert_eq!(scopes, ["bundled", "project", "global"]);
        assert!(copies[0].effective && copies[0].same_as_effective);
        assert!(!copies[1].effective && copies[1].same_as_effective);
        assert!(!copies[2].effective && !copies[2].same_as_effective);
        assert!(copies[1].dir.ends_with("search"));
    }

    #[test]
    fn install_receipt_names_the_copy_that_shadows_a_global_install() {
        let temp = TestDir::new();
        let (_, _, project, global) = scoped_index(&temp);
        let project_dir = write_skill(&project, "notes", "notes", "project");
        let global_dir = write_skill(&global, "notes", "notes", "global");
        let only_global = write_skill(&global, "solo", "solo", "global");
        let (index, ..) = scoped_index(&temp);

        let shadowed = install_receipt(
            index.catalog_records(),
            "notes",
            SkillSource::Global,
            &global_dir,
        );
        assert_eq!(shadowed.scope, "global");
        let winner = shadowed.shadowed_by.expect("project copy wins");
        assert_eq!(winner.scope, "project");
        assert_eq!(PathBuf::from(winner.dir), project_dir);

        let active = install_receipt(
            index.catalog_records(),
            "solo",
            SkillSource::Global,
            &only_global,
        );
        assert_eq!(active.shadowed_by, None);
    }

    #[test]
    fn install_scope_defaults_to_global_and_rejects_unknown_scopes() {
        assert_eq!(install_scope(None), Ok(SkillSource::Global));
        assert_eq!(install_scope(Some("project")), Ok(SkillSource::Project));
        assert!(install_scope(Some("bundled")).is_err());
    }

    #[test]
    fn project_install_writes_under_the_project_skills_directory() {
        let temp = TestDir::new();
        let source = write_skill(&temp.0, "source", "local-only", "body");
        let project_root = temp.0.join("project");
        let target = project_skills_dir(&project_root);

        assert_eq!(
            install_skill_source(&source, &target),
            Ok("local-only".into())
        );
        assert!(project_root
            .join(".wisp/skills/local-only/SKILL.md")
            .is_file());
    }

    #[test]
    fn promoting_never_overwrites_an_existing_global_folder() {
        let temp = TestDir::new();
        let source = write_skill(&temp.0, "project/notes", "notes", "project");
        std::fs::write(source.join("helper.py"), "print(1)").unwrap();
        let global = temp.0.join("global");
        let existing = write_skill(&global, "notes", "notes", "keep me");

        let error = promote_skill_dir(&source, &existing).unwrap_err();
        assert!(error.starts_with("skill_global_exists:"));
        assert!(std::fs::read_to_string(existing.join("SKILL.md"))
            .unwrap()
            .contains("keep me"));

        let dest = global.join("fresh");
        promote_skill_dir(&source, &dest).unwrap();
        assert!(dest.join("helper.py").is_file());
        assert!(source.join("SKILL.md").is_file(), "project copy is kept");
    }

    #[test]
    fn project_copy_is_removable_only_when_another_copy_keeps_the_name() {
        let temp = TestDir::new();
        let (_, bundled, project, global) = scoped_index(&temp);
        let promoted = write_skill(&project, "promoted", "promoted", "v1");
        write_skill(&global, "promoted", "promoted", "v2");
        let identical = write_skill(&project, "dup", "dup", "same");
        write_skill(&bundled, "dup", "dup", "same");
        write_skill(&project, "edited", "edited", "mine");
        write_skill(&bundled, "edited", "edited", "upstream");
        write_skill(&project, "unique", "unique", "only");
        let (index, ..) = scoped_index(&temp);
        let records = index.catalog_records();

        assert_eq!(
            removable_project_copy(records, "promoted", &project),
            Ok(promoted)
        );
        assert_eq!(
            removable_project_copy(records, "dup", &project),
            Ok(identical)
        );
        assert_eq!(
            removable_project_copy(records, "edited", &project),
            Err("skill_project_copy_is_only_copy".into())
        );
        assert_eq!(
            removable_project_copy(records, "unique", &project),
            Err("skill_project_copy_is_only_copy".into())
        );
        assert_eq!(
            removable_project_copy(records, "missing", &project),
            Err("skill_not_in_project".into())
        );
    }

    #[test]
    fn project_skill_packages_lists_every_package_that_project_deletion_removes() {
        let temp = TestDir::new();
        assert!(project_skill_package_names(&temp.0).is_empty());

        let skills = project_skills_dir(&temp.0);
        write_skill(&skills, "stats", "session-stats", "body");
        write_skill(&skills, "search", "nature-academic-search", "body");
        std::fs::create_dir_all(skills.join("broken")).unwrap();
        std::fs::write(skills.join("broken/SKILL.md"), "no frontmatter").unwrap();
        std::fs::create_dir_all(skills.join("empty-folder")).unwrap();

        assert_eq!(
            project_skill_package_names(&temp.0),
            ["broken", "nature-academic-search", "session-stats"]
        );
    }

    #[test]
    fn reload_enables_new_skills_without_reviving_disabled_existing_skills() {
        let previous = HashSet::from(["enabled".into(), "disabled".into()]);
        let enabled = Some(HashSet::from(["enabled".into()]));

        let updated = enabled_names_after_reload(
            enabled,
            &previous,
            ["enabled", "disabled", "new-project-skill"],
        )
        .unwrap();

        assert!(updated.contains("enabled"));
        assert!(!updated.contains("disabled"));
        assert!(updated.contains("new-project-skill"));
    }
}
