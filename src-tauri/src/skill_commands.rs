use super::{
    clear_idle_agents, effective_enabled_skill_names, load_enabled_skill_names, load_skill_tags,
    normalize_tags, save_enabled_skill_names, save_skill_tags, skill_infos, skill_paths, AppState,
    SkillInfo,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, State};
use wisp_skills::SkillIndex;

#[tauri::command]
pub(super) async fn list_skills(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<SkillInfo>, String> {
    let ap = state.active(window.label());
    let tags = load_skill_tags(&state.store).await;
    let enabled = effective_enabled_skill_names(&state.store, &ap).await;
    Ok(skill_infos(&ap.skills, &tags, enabled.as_ref()))
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
    save_skill_tags(&state.store, &all_tags).await
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
    clear_idle_agents(state).await;
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
    // Let the user pick a SKILL.md; folder picking is offered via a second button
    // in the UI that calls pick_directory (existing command).
    app.dialog()
        .file()
        .add_filter("SKILL.md", &["md"])
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
    // Resolve the skill's source dir + the SKILL.md path.
    let (skill_dir, skill_md) = if src.is_dir() {
        let md = src.join("SKILL.md");
        if !md.is_file() {
            return Err("selected folder has no SKILL.md".into());
        }
        (src.clone(), md)
    } else if src.file_name().map(|n| n == "SKILL.md").unwrap_or(false) {
        (
            src.parent().map(PathBuf::from).unwrap_or_default(),
            src.clone(),
        )
    } else {
        return Err("select a skill folder or a SKILL.md file".into());
    };
    // Parse name from frontmatter (fall back to dir name), validate description.
    let skill = wisp_skills::parse_skill_file(&skill_md)?;
    if skill.description.trim().is_empty() {
        return Err("SKILL.md is missing a description".into());
    }
    validate_skill_name(&skill.name)?;
    let dest = user_skills_dir()?.join(&skill.name);
    if dest.exists() {
        return Err(format!("a skill named '{}' already exists", skill.name));
    }
    {
        // Recursive copy off the async runtime: a skill folder can be large.
        let (skill_dir, dest) = (skill_dir.clone(), dest.clone());
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(dest.parent().unwrap())?;
            copy_dir_recursive(&skill_dir, &dest)
        })
        .await
        .map_err(|e| format!("{e}"))?
        .map_err(|e| format!("{e}"))?;
    }
    reload_skills(&state, window.label());
    let ap = state.active(window.label());
    if let Some(mut enabled) = load_enabled_skill_names(&state.store, &ap.id).await {
        enabled.insert(skill.name.clone());
        save_enabled_skill_names(&state.store, &ap.id, &enabled).await?;
    }
    clear_idle_agents(&state).await;
    Ok(skill.name)
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
    reload_skills(&state, window.label());
    clear_idle_agents(&state).await;
    Ok(())
}

fn reload_skills(state: &AppState, label: &str) {
    let mut ap = state.active(label);
    ap.skills = Arc::new(SkillIndex::load(&skill_paths(&ap.root)));
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
