//! Skills from the community directory and from GitHub, as the WebView's
//! skill store gets them: a package is resolved to an exact commit, its
//! archive checked entry by entry, and installed into your own skills
//! folder only where nothing of that name is. What is downloaded is data;
//! nothing in it is run. The rules are `wisp_skills::distribution`'s,
//! shared with the WebView.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use wisp_dto::{
    CommunitySkillCatalog, SkillInstallResult, SkillInstallSource, SkillStoreCandidate,
};
use wisp_skills::distribution::{self as dist, GithubApi, StagingDir};

use crate::Server;

/// Where packages come from: GitHub's API and its archives.
#[async_trait::async_trait]
pub(crate) trait Fetch: GithubApi + Send {
    /// The repository at the source's commit, as a ZIP.
    async fn archive(&self, source: &SkillInstallSource) -> Result<Vec<u8>, String>;
    /// The community directory as it is published now.
    async fn index(&self) -> Result<Vec<u8>, String>;
}

/// GitHub itself, without following redirects.
pub(crate) struct Github(reqwest::Client);

impl Github {
    pub(crate) fn new(proxy: &str) -> Result<Self> {
        let mut client = reqwest::Client::builder()
            .user_agent("wisp-science-skill-store")
            .timeout(std::time::Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::none());
        match proxy.trim() {
            "" => {}
            "none" => client = client.no_proxy(),
            proxy => client = client.proxy(reqwest::Proxy::all(proxy)?),
        }
        Ok(Self(client.build()?))
    }

    async fn bytes(&self, url: &str, limit: usize) -> Result<Option<Vec<u8>>, String> {
        let mut response = self
            .0
            .get(url)
            .send()
            .await
            .map_err(|error| format!("GitHub network error: {error}"))?;
        match response.status().as_u16() {
            404 => return Ok(None),
            403 | 429 => return Err(
                "GitHub refused or rate-limited the request. Try again later; nothing was changed."
                    .into(),
            ),
            status if !(200..300).contains(&status) => {
                return Err(format!(
                    "GitHub answered HTTP {status}; nothing was changed."
                ))
            }
            _ => {}
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err(format!("The download is over {limit} bytes."));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("The download was cut off: {error}"))?
        {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(format!("The download is over {limit} bytes."));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
    }
}

#[async_trait::async_trait]
impl GithubApi for Github {
    async fn json(&self, path: &str) -> Result<Option<serde_json::Value>, String> {
        self.bytes(&format!("https://api.github.com{path}"), 1024 * 1024)
            .await?
            .map(|bytes| {
                serde_json::from_slice(&bytes)
                    .map_err(|error| format!("GitHub's answer could not be read: {error}"))
            })
            .transpose()
    }
}

#[async_trait::async_trait]
impl Fetch for Github {
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
        .ok_or_else(|| "That commit of the repository is not there (404).".into())
    }

    async fn index(&self) -> Result<Vec<u8>, String> {
        self.bytes(dist::INDEX_URL, 1024 * 1024)
            .await?
            .ok_or_else(|| "The community directory is not there (404).".into())
    }
}

/// The community directory: published now when `refresh` and it can be
/// read, else the one shipped with the app, saying why.
pub(crate) async fn community(fetch: &dyn Fetch, refresh: bool) -> Result<CommunitySkillCatalog> {
    let mut notice = None;
    if refresh {
        match fetch
            .index()
            .await
            .and_then(|bytes| dist::parse_catalog(&bytes))
        {
            Ok(entries) => return Ok(CommunitySkillCatalog { entries, notice }),
            Err(error) => {
                notice = Some(format!(
                    "{error} Showing the directory that ships with the app."
                ))
            }
        }
    }
    let entries = dist::parse_catalog(include_bytes!("../../../community-skills/index.json"))
        .map_err(|error| anyhow!(error))?;
    Ok(CommunitySkillCatalog { entries, notice })
}

/// Marks a candidate whose name is already used: by a skill the project
/// can use (`taken`), or by a folder in your own skills.
fn annotate(candidates: &mut [SkillStoreCandidate], taken: &[(String, String)], personal: &Path) {
    for candidate in candidates {
        let mut said: Vec<String> = taken
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case(&candidate.name))
            .map(|(name, place)| format!("{name} is already a skill here ({place})"))
            .collect();
        let valid = !candidate.name.starts_with('.')
            && !candidate.name.contains('/')
            && dist::portable_path(&candidate.name, false).is_ok();
        let existing = personal.join(&candidate.name);
        if valid && std::fs::symlink_metadata(&existing).is_ok() {
            candidate.installed_source = dist::read_origin(&existing);
            said.push(format!(
                "{} already has it; nothing there is replaced",
                existing.display()
            ));
        }
        if !said.is_empty() {
            candidate.conflict = Some(said.join("\n"));
        }
    }
}

/// The packages at a GitHub address, read from its archive at the exact
/// commit the address (or `exact_ref`) names.
pub(crate) async fn preview(
    fetch: &dyn Fetch,
    address: &str,
    exact_ref: &str,
    taken: Vec<(String, String)>,
    personal: PathBuf,
) -> Result<Vec<SkillStoreCandidate>> {
    let source = dist::resolve_source(&DynApi(fetch), address, exact_ref)
        .await
        .map_err(|error| anyhow!(error))?;
    let bytes = fetch
        .archive(&source)
        .await
        .map_err(|error| anyhow!(error))?;
    tokio::task::spawn_blocking(move || {
        let staging = StagingDir::new(&std::env::temp_dir()).map_err(|error| anyhow!(error))?;
        let root = dist::extract_repository(&bytes, &staging.0).map_err(|error| anyhow!(error))?;
        let mut candidates =
            dist::inspect_repository(&root, &source).map_err(|error| anyhow!(error))?;
        annotate(&mut candidates, &taken, &personal);
        Ok(candidates)
    })
    .await?
}

/// Installs the one package `source` names into `personal`, refusing a
/// name that is taken. Returns where it went.
pub(crate) async fn install(
    fetch: &dyn Fetch,
    source: SkillInstallSource,
    taken: Vec<(String, String)>,
    personal: PathBuf,
) -> Result<SkillInstallResult> {
    dist::validate_source(&source).map_err(|error| anyhow!(error))?;
    let bytes = fetch
        .archive(&source)
        .await
        .map_err(|error| anyhow!(error))?;
    tokio::task::spawn_blocking(move || {
        let staging = StagingDir::new(&std::env::temp_dir()).map_err(|error| anyhow!(error))?;
        let root = dist::extract_repository(&bytes, &staging.0).map_err(|error| anyhow!(error))?;
        let mut candidates =
            dist::inspect_repository(&root, &source).map_err(|error| anyhow!(error))?;
        annotate(&mut candidates, &taken, &personal);
        let candidate = candidates
            .into_iter()
            .find(|candidate| candidate.source.package_path == source.package_path)
            .context("Choose one skill package to install")?;
        if let Some(conflict) = &candidate.conflict {
            anyhow::bail!("That name is taken; nothing was changed.\n{conflict}");
        }
        let destination =
            dist::install_new(&root, &personal, &candidate).map_err(|error| anyhow!(error))?;
        Ok(SkillInstallResult {
            name: candidate.name,
            directory: destination.to_string_lossy().into_owned(),
            notice: None,
        })
    })
    .await?
}

/// A `&dyn Fetch` as the resolver takes it.
struct DynApi<'a>(&'a dyn Fetch);

#[async_trait::async_trait]
impl GithubApi for DynApi<'_> {
    async fn json(&self, path: &str) -> Result<Option<serde_json::Value>, String> {
        self.0.json(path).await
    }
}

impl Server {
    fn github(&self, routes: &wisp_dto::NetworkSettings) -> Result<Github> {
        Github::new(&routes.mcp_proxy_url)
    }

    /// Your own skills folder, where store packages go.
    fn personal_skills(&self) -> Result<PathBuf> {
        self.0
            .config
            .personal_skills
            .clone()
            .context("This host keeps no skills of your own")
    }

    /// The skills `project` can use already, by name with where they are.
    async fn taken_skills(&self, project: &str) -> Result<Vec<(String, String)>> {
        Ok(self
            .skills(Some(project))
            .await?
            .into_iter()
            .map(|skill| (skill.name, skill.source.to_owned()))
            .collect())
    }

    /// The community directory of skills, as published now when `refresh`.
    pub async fn community_skills(&self, refresh: bool) -> Result<CommunitySkillCatalog> {
        let routes = crate::network::load(&self.0.store).await;
        community(&self.github(&routes)?, refresh).await
    }

    /// The skill packages at a GitHub address, for `project`.
    pub async fn preview_github_skills(
        &self,
        project: &str,
        address: &str,
        exact_ref: &str,
    ) -> Result<Vec<SkillStoreCandidate>> {
        let routes = crate::network::load(&self.0.store).await;
        self.preview_with(&self.github(&routes)?, project, address, exact_ref)
            .await
    }

    pub(crate) async fn preview_with(
        &self,
        fetch: &dyn Fetch,
        project: &str,
        address: &str,
        exact_ref: &str,
    ) -> Result<Vec<SkillStoreCandidate>> {
        let taken = self.taken_skills(project).await?;
        preview(fetch, address, exact_ref, taken, self.personal_skills()?).await
    }

    /// Installs one package into your own skills folder; every project
    /// has it from its next turn.
    pub async fn install_github_skill(
        &self,
        project: &str,
        source: SkillInstallSource,
    ) -> Result<SkillInstallResult> {
        let routes = crate::network::load(&self.0.store).await;
        self.install_with(&self.github(&routes)?, project, source)
            .await
    }

    pub(crate) async fn install_with(
        &self,
        fetch: &dyn Fetch,
        project: &str,
        source: SkillInstallSource,
    ) -> Result<SkillInstallResult> {
        let _admission = self.0.admission.read().await;
        let taken = self.taken_skills(project).await?;
        install(fetch, source, taken, self.personal_skills()?).await
    }
}
