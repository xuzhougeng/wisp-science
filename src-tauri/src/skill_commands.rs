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
    window: tauri::WebviewWindow,
) -> Result<Vec<SkillInfo>, String> {
    list_skill_infos_for_project(&state, window.label()).await
}

#[tauri::command]
pub(super) async fn reload_skills(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
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
    let ap = state.active(label);
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
    window: tauri::WebviewWindow,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    update_skills_enabled(&state, window.label(), vec![name], enabled).await
}

#[tauri::command]
pub(super) async fn set_skills_enabled(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
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

fn user_skills_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|h| h.join(".wisp").join("skills"))
        .ok_or_else(|| "no home directory".to_string())
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

#[tauri::command]
pub(super) async fn install_skill(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    src_path: String,
) -> Result<String, String> {
    let src = PathBuf::from(&src_path);
    let skills_dir = user_skills_dir()?;
    // ZIP extraction, recursive copy, and the atomic directory swap run off
    // the async runtime. Existing user-added skills are replaced so importing
    // an updated package is a normal upgrade path.
    let skill_name = tokio::task::spawn_blocking(move || install_skill_source(&src, &skills_dir))
        .await
        .map_err(|e| format!("{e}"))??;
    reload_host_skill_index(&state, window.label());
    let ap = state.active(window.label());
    if let Some(mut enabled) = load_enabled_skill_names(&state.store, &ap.id).await {
        enabled.insert(skill_name.clone());
        save_enabled_skill_names(&state.store, &ap.id, &enabled).await?;
    }
    clear_idle_agents(&state).await;
    Ok(skill_name)
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
    window: tauri::WebviewWindow,
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
    let ap = state.active(window.label());
    if let Some(mut enabled) = load_enabled_skill_names(&state.store, &ap.id).await {
        enabled.remove(&name);
        let _ = save_enabled_skill_names(&state.store, &ap.id, &enabled).await;
    }
    let mut tags = load_skill_tags(&state.store).await;
    tags.remove(&name);
    let _ = save_skill_tags(&state.store, &tags).await;
    reload_host_skill_index(&state, window.label());
    clear_idle_agents(&state).await;
    Ok(())
}

fn reload_host_skill_index(state: &AppState, label: &str) {
    let mut ap = state.active(label);
    ap.skills = Arc::new(load_skill_index(&ap.root));
    state.set_active(label, ap);
}

pub(super) fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
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
