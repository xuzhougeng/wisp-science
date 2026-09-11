//! Public community directory and GitHub installs. Downloaded code is data;
//! this module never starts a process or modifies the bundled catalog.
use crate::{project_skill_catalog, AppState};
use std::path::Path;
use tauri::State;
use wisp_dto::{
    CommunitySkillCatalog, SkillInstallResult, SkillInstallSource, SkillStoreCandidate,
};
use wisp_skills::distribution::{self as dist, GithubApi, StagingDir};

struct GithubClient(reqwest::Client);
impl GithubClient {
    fn new() -> Result<Self, String> {
        Ok(Self(
            reqwest::Client::builder()
                .user_agent("wisp-science-skill-store")
                .timeout(std::time::Duration::from_secs(90))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())?,
        ))
    }

    async fn bytes(&self, url: &str, limit: usize) -> Result<Option<Vec<u8>>, String> {
        let mut response = self
            .0
            .get(url)
            .send()
            .await
            .map_err(|e| format!("GitHub network error: {e}"))?;
        match response.status().as_u16() {
            404 => return Ok(None),
            403 | 429 => return Err("GitHub access denied or API rate limit reached. Retry later; no package was changed.".into()),
            status if !(200..300).contains(&status) => return Err(format!("GitHub request failed (HTTP {status}); no package was changed.")),
            _ => {}
        }
        if response
            .content_length()
            .is_some_and(|len| len > limit as u64)
        {
            return Err(format!("Download exceeds {} bytes.", limit));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("GitHub download interrupted: {e}"))?
        {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(format!("Download exceeds {} bytes.", limit));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
    }

    async fn archive(&self, source: &SkillInstallSource) -> Result<Vec<u8>, String> {
        dist::validate_source(source)?;
        self.bytes(
            &format!(
                "https://codeload.github.com/{}/zip/{}",
                source.repository, source.commit
            ),
            dist::MAX_ARCHIVE_BYTES,
        )
        .await?
        .ok_or("Repository commit is unavailable (404).".into())
    }
}

#[async_trait::async_trait]
impl GithubApi for GithubClient {
    async fn json(&self, path: &str) -> Result<Option<serde_json::Value>, String> {
        self.bytes(&format!("https://api.github.com{path}"), 1024 * 1024)
            .await?
            .map(|bytes| {
                serde_json::from_slice(&bytes).map_err(|e| format!("Invalid GitHub response: {e}"))
            })
            .transpose()
    }
}

#[tauri::command]
pub(super) async fn list_community_skills(refresh: bool) -> Result<CommunitySkillCatalog, String> {
    // A shipped snapshot allows offline discovery. Refresh never changes local
    // installations and failure leaves the usable snapshot visible.
    let mut notice = None;
    if refresh {
        let result = async {
            let bytes = GithubClient::new()?
                .bytes(dist::INDEX_URL, 1024 * 1024)
                .await?
                .ok_or("Community index is unavailable (404).")?;
            dist::parse_catalog(&bytes)
        }
        .await;
        match result {
            Ok(entries) => return Ok(CommunitySkillCatalog { entries, notice }),
            Err(error) => {
                notice = Some(format!(
                    "{error} Showing the directory shipped with this app."
                ))
            }
        }
    }
    let entries = dist::parse_catalog(include_bytes!("../../community-skills/index.json"))?;
    Ok(CommunitySkillCatalog { entries, notice })
}

async fn annotate_conflicts(
    state: &AppState,
    label: &str,
    candidates: &mut [SkillStoreCandidate],
) -> Result<(), String> {
    let mut project = state.require_active(label)?;
    // Re-read files before confirmation/install so a stale UI cannot overwrite
    // a newly added local package or miss bundled/project shadowing.
    project.skills = std::sync::Arc::new(crate::load_skill_index(&project.root));
    let (catalog, _) = project_skill_catalog(&state.store, &project).await;
    let global = crate::skill_commands::user_skills_dir()?;
    for candidate in candidates {
        let conflicts: Vec<_> = catalog
            .catalog_records()
            .iter()
            .filter(|record| record.name.eq_ignore_ascii_case(&candidate.name))
            .collect();
        if !conflicts.is_empty() {
            candidate.conflict = Some(
                conflicts
                    .iter()
                    .map(|r| {
                        format!(
                            "{}: {} ({}){}",
                            r.scope.as_str(),
                            r.name,
                            r.path.display(),
                            if r.effective {
                                " — effective source"
                            } else {
                                " — shadowed or invalid"
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        // Invalid frontmatter names are previewable errors, never paths to read.
        if candidate.name.starts_with('.')
            || candidate.name.contains('/')
            || dist::portable_path(&candidate.name, false).is_err()
        {
            continue;
        }
        if std::fs::symlink_metadata(global.join(&candidate.name)).is_ok() {
            candidate.installed_source = dist::read_origin(&global.join(&candidate.name));
            let existing = format!("Existing global package at {}. Keep it or manage it in Installed Skills; the store does not replace local files.", global.join(&candidate.name).display());
            candidate.conflict = Some(match candidate.conflict.take() {
                Some(conflict) => format!("{conflict}\n{existing}"),
                None => existing,
            });
        }
    }
    Ok(())
}

#[tauri::command]
pub(super) async fn preview_github_skills(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    source_url: String,
    exact_ref: String,
) -> Result<Vec<SkillStoreCandidate>, String> {
    state.require_active(window.label())?;
    let client = GithubClient::new()?;
    let source = dist::resolve_source(&client, &source_url, &exact_ref).await?;
    let bytes = client.archive(&source).await?;
    let mut candidates = tokio::task::spawn_blocking(move || {
        let staging = StagingDir::new(&std::env::temp_dir())?;
        let root = dist::extract_repository(&bytes, &staging.0)?;
        dist::inspect_repository(&root, &source)
    })
    .await
    .map_err(|e| e.to_string())??;
    annotate_conflicts(&state, window.label(), &mut candidates).await?;
    Ok(candidates)
}

#[tauri::command]
pub(super) async fn install_github_skill(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    source: SkillInstallSource,
) -> Result<SkillInstallResult, String> {
    state.require_active(window.label())?;
    dist::validate_source(&source)?;
    let client = GithubClient::new()?;
    let bytes = client.archive(&source).await?;
    let selected_path = source.package_path.clone();
    let (staging, root, mut candidates) = tokio::task::spawn_blocking(move || {
        let staging = StagingDir::new(&std::env::temp_dir())?;
        let root = dist::extract_repository(&bytes, &staging.0)?;
        let candidates = dist::inspect_repository(&root, &source)?;
        Ok::<_, String>((staging, root, candidates))
    })
    .await
    .map_err(|e| e.to_string())??;
    annotate_conflicts(&state, window.label(), &mut candidates).await?;
    let candidate = candidates
        .into_iter()
        .find(|c| c.source.package_path == selected_path)
        .ok_or("Select one specific Skill package before installation.")?;
    if let Some(conflict) = &candidate.conflict {
        return Err(format!(
            "Name conflict; existing files preserved.\n{conflict}"
        ));
    }
    let skills_dir = crate::skill_commands::user_skills_dir()?;
    let name = candidate.name.clone();
    let destination = tokio::task::spawn_blocking(move || {
        let _staging = staging;
        dist::install_new(&root, &skills_dir, &candidate)
    })
    .await
    .map_err(|e| e.to_string())??;
    // A refresh failure is distinct from an install failure: retain and report
    // the complete package, so Retry cannot accidentally replace it.
    let notice = crate::skill_commands::reload_skills(state, window)
        .await
        .err()
        .map(|e| format!("Package installed. Reload Skills to refresh this project: {e}"));
    Ok(SkillInstallResult {
        name,
        directory: destination.to_string_lossy().into_owned(),
        notice,
    })
}

#[tauri::command]
pub(super) async fn get_skill_install_source(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    name: String,
) -> Result<Option<SkillInstallSource>, String> {
    let project = state.require_active(window.label())?;
    let (catalog, _) = project_skill_catalog(&state.store, &project).await;
    let skill = catalog.get(&name).ok_or("Skill not found.")?;
    Ok(dist::read_origin(Path::new(&skill.dir)))
}
