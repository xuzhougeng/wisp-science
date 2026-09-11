//! Bounded, non-executing GitHub Skill distribution. No credentials or network
//! are needed for the parser, package validator, or atomic new-install path.
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use wisp_dto::{CommunitySkillEntry, SkillInstallSource, SkillStoreCandidate};

pub const GUIDE_URL: &str =
    "https://github.com/xuzhougeng/wisp-science/blob/main/docs/skill-authoring.md";
pub const INDEX_URL: &str =
    "https://raw.githubusercontent.com/xuzhougeng/wisp-science/main/community-skills/index.json";
pub const MAX_ARCHIVE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_EXPANDED_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 4000;
pub const ORIGIN_FILE: &str = ".wisp-source.json";

#[derive(Debug, PartialEq)]
pub struct GithubLink {
    pub repository: String,
    pub kind: Option<String>,
    /// GitHub's tree/blob URLs do not encode the ref/path boundary.
    pub tail: String,
}

pub fn portable_path(value: &str, allow_empty: bool) -> Result<(), String> {
    if value.is_empty() && allow_empty {
        return Ok(());
    }
    if value.len() > 1024
        || value.split('/').any(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with([' ', '.'])
                || part
                    .chars()
                    .any(|c| c.is_control() || "\\:<>\"|?*".contains(c))
                || [
                    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
                    "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7",
                    "LPT8", "LPT9",
                ]
                .contains(&stem.as_str())
        })
    {
        return Err(format!("Unsafe or non-portable package path: {value}"));
    }
    Ok(())
}

pub fn parse_github_link(value: &str) -> Result<GithubLink, String> {
    let url = url::Url::parse(value.trim()).map_err(|e| format!("Invalid GitHub URL: {e}"))?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
    {
        return Err("Use a public https://github.com/owner/repository URL, without credentials or query parameters.".into());
    }
    let decoded = percent_decode_path(url.path())?;
    portable_path(decoded.trim_matches('/'), false)?;
    let parts: Vec<_> = decoded.trim_matches('/').split('/').collect();
    if parts.len() < 2 {
        return Err("GitHub URL needs an owner and repository.".into());
    }
    let repo = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
    if !parts[..2].iter().all(|v| {
        v.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    }) || repo.is_empty()
    {
        return Err("Invalid GitHub owner or repository.".into());
    }
    let kind = if parts.len() > 2 {
        if parts.len() < 4 || !["tree", "blob"].contains(&parts[2]) {
            return Err("Use a repository, tree directory, or blob SKILL.md link.".into());
        }
        Some(parts[2].to_string())
    } else {
        None
    };
    let tail = parts.get(3..).unwrap_or_default().join("/");
    if kind.as_deref() == Some("blob") && !tail.ends_with("/SKILL.md") {
        return Err("A GitHub file link must point to SKILL.md.".into());
    }
    Ok(GithubLink {
        repository: format!("{}/{repo}", parts[0]),
        kind,
        tail,
    })
}

fn percent_decode_path(value: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut iter = value.bytes();
    while let Some(c) = iter.next() {
        if c == b'%' {
            let a = iter.next().and_then(|c| (c as char).to_digit(16));
            let b = iter.next().and_then(|c| (c as char).to_digit(16));
            bytes.push(match (a, b) {
                (Some(a), Some(b)) => (a * 16 + b) as u8,
                _ => return Err("Invalid URL escape.".into()),
            });
        } else {
            bytes.push(c);
        }
    }
    String::from_utf8(bytes).map_err(|_| "GitHub path is not UTF-8.".into())
}

/// Resolve the boundary only against an API-confirmed ref; never guess a branch.
pub fn package_path_for_ref(link: &GithubLink, git_ref: &str) -> Result<String, String> {
    if link.kind.is_none() {
        return Ok(String::new());
    }
    let path = if link.tail == git_ref {
        ""
    } else {
        link.tail
            .strip_prefix(&format!("{git_ref}/"))
            .ok_or("The exact ref does not match this GitHub URL.")?
    };
    let path = if link.kind.as_deref() == Some("blob") {
        if path == "SKILL.md" {
            ""
        } else {
            path.strip_suffix("/SKILL.md")
                .ok_or("File link must identify SKILL.md within the selected ref.")?
        }
    } else {
        path
    };
    portable_path(path, true)?;
    Ok(path.into())
}

pub fn validate_source(source: &SkillInstallSource) -> Result<(), String> {
    let link = parse_github_link(&format!("https://github.com/{}", source.repository))?;
    if link.kind.is_some() || link.repository != source.repository {
        return Err("Invalid repository identity.".into());
    }
    if source.commit.len() != 40 || !source.commit.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Installation requires the full resolved 40-character commit SHA.".into());
    }
    if source.git_ref.trim().is_empty() {
        return Err("Missing resolved ref.".into());
    }
    let original = parse_github_link(&source.source_url)?;
    if original.repository != source.repository {
        return Err("Source URL and repository disagree.".into());
    }
    portable_path(&source.package_path, true)
}

pub struct StagingDir(pub PathBuf);
impl StagingDir {
    pub fn new(parent: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let path = parent.join(format!("skill-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for StagingDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Extract a GitHub zipball to an exclusively owned staging directory. Every
/// entry is checked, including entries outside the selected Skill package.
pub fn extract_repository(bytes: &[u8], destination: &Path) -> Result<PathBuf, String> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("Repository archive exceeds 32 MiB.".into());
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("Invalid ZIP: {e}"))?;
    if archive.len() > MAX_ENTRIES {
        return Err("Repository archive exceeds 4,000 entries.".into());
    }
    let mut seen = HashSet::new();
    let mut spelling = HashMap::new();
    let mut root = None::<String>;
    let mut expanded = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().trim_end_matches('/').to_string();
        portable_path(&name, false)?;
        if name.split('/').count() > 64 {
            return Err("Repository path exceeds 64 levels.".into());
        }
        if !seen.insert(name.to_lowercase()) {
            return Err(format!("Duplicate or case-colliding ZIP path: {name}"));
        }
        // Explicit and implicit directory names must agree on case too;
        // otherwise Linux and Windows would install different package trees.
        let mut prefix = String::new();
        for component in name.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if let Some(previous) = spelling.insert(prefix.to_lowercase(), prefix.clone()) {
                if previous != prefix {
                    return Err(format!("Case-colliding ZIP directory: {prefix}"));
                }
            }
        }
        let top = name.split('/').next().unwrap().to_string();
        if root.as_ref().is_some_and(|r| r != &top) {
            return Err("GitHub archive must have one repository root.".into());
        }
        root = Some(top);
        let mode = entry.unix_mode().unwrap_or(0) & 0o170000;
        if mode != 0 && mode != 0o100000 && mode != 0o040000 {
            return Err(format!("Symbolic link or special ZIP entry: {name}"));
        }
        if expanded.saturating_add(entry.size()) > MAX_EXPANDED_BYTES {
            return Err("Repository expanded size exceeds 128 MiB.".into());
        }
        if entry.size() > MAX_FILE_BYTES {
            return Err(format!("File exceeds 8 MiB: {name}"));
        }
        let path = destination.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let copied = std::io::copy(&mut entry.by_ref().take(MAX_FILE_BYTES + 1), &mut output)
            .map_err(|e| format!("Extract {name}: {e}"))?;
        expanded += copied;
        if copied > MAX_FILE_BYTES || expanded > MAX_EXPANDED_BYTES {
            return Err(
                "Repository expanded size exceeds the 8 MiB file / 128 MiB total limit.".into(),
            );
        }
        output.flush().map_err(|e| e.to_string())?;
    }
    root.map(|r| destination.join(r))
        .filter(|p| p.is_dir())
        .ok_or("Archive has no repository directory.".into())
}

pub fn inspect_repository(
    root: &Path,
    source: &SkillInstallSource,
) -> Result<Vec<SkillStoreCandidate>, String> {
    validate_source(source)?;
    let selected = root.join(&source.package_path);
    if !selected.is_dir() {
        return Err(format!(
            "Package directory not found at commit {}: {}",
            source.commit, source.package_path
        ));
    }
    let mut candidates = Vec::new();
    let mut preview_bytes = 0usize;
    for entry in walkdir::WalkDir::new(&selected)
        .sort_by_file_name()
        .max_depth(64)
    {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().is_file() || entry.file_name() != "SKILL.md" {
            continue;
        }
        if candidates.len() >= 100 {
            return Err(
                "More than 100 Skills found. Provide a narrower Skill directory URL.".into(),
            );
        }
        let dir = entry.path().parent().unwrap();
        if selected.join("SKILL.md").is_file() && dir != selected {
            continue;
        }
        let mut source = source.clone();
        source.package_path = dir
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let markdown = std::fs::read_to_string(entry.path())
            .map_err(|e| format!("Read {}: {e}", entry.path().display()))?;
        if markdown.len() > 1024 * 1024 {
            return Err("SKILL.md preview exceeds 1 MiB. Select a smaller package.".into());
        }
        preview_bytes += markdown.len();
        if preview_bytes > 4 * 1024 * 1024 {
            return Err(
                "Combined SKILL.md previews exceed 4 MiB. Select a narrower package directory."
                    .into(),
            );
        }
        let fallback = if source.package_path.is_empty() {
            source.repository.rsplit('/').next().unwrap().to_string()
        } else {
            dir.file_name().unwrap().to_string_lossy().into_owned()
        };
        let mut candidate = SkillStoreCandidate { name: fallback.clone(), description: String::new(), tags: vec![], source, markdown: markdown.clone(), format_errors: vec![], resource_errors: vec![], warnings: vec!["Dependencies have not been probed or configured; review SKILL.md and the directory dependency declarations before use.".into(), "Runtime behavior has not been verified. Format validation is not a runtime compatibility endorsement.".into()], conflict: None, installed_source: None };
        match crate::manifest::parse_skill_document(&markdown, fallback) {
            Ok((manifest, _)) => {
                candidate.name = manifest.name.unwrap();
                candidate.description = manifest.description;
                candidate.tags = manifest.tags.0;
                if candidate.description.is_empty() {
                    candidate
                        .format_errors
                        .push("SKILL.md description is empty.".into());
                }
                if candidate.name.contains('/')
                    || candidate.name.starts_with('.')
                    || portable_path(&candidate.name, false).is_err()
                {
                    candidate
                        .format_errors
                        .push("Skill name must be one portable, non-hidden directory name.".into());
                }
            }
            Err(e) => candidate.format_errors.push(e),
        }
        // Check explicit Markdown relative resource links and inline-code
        // references. Dynamic paths / free prose still need author review.
        let links = regex::Regex::new(
            r"\]\(([^\s)]+)(?:\s+[^)]*)?\)|`((?:references|scripts|assets)/[^`]+)`",
        )
        .unwrap();
        let mut fence = None::<char>;
        let resource_prose = markdown
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                    let marker = trimmed.chars().next().unwrap();
                    if fence == Some(marker) {
                        fence = None;
                    } else if fence.is_none() {
                        fence = Some(marker);
                    }
                    return false;
                }
                fence.is_none()
            })
            .collect::<Vec<_>>()
            .join("\n");
        for capture in links.captures_iter(&resource_prose) {
            let value = capture.get(1).or_else(|| capture.get(2)).unwrap().as_str();
            if value.contains("://") || value.starts_with('#') || value.starts_with("mailto:") {
                continue;
            }
            let value = value.split('#').next().unwrap();
            if value.is_empty() {
                continue;
            }
            let value = percent_decode_path(value.strip_prefix("./").unwrap_or(value))?;
            if portable_path(&value, false).is_err() || !dir.join(&value).exists() {
                candidate
                    .resource_errors
                    .push(format!("Missing or unsafe package resource: {value}"));
            }
        }
        // GitHub ZIPs contain LFS pointer text and omit submodule contents.
        // Neither is a complete distributable package.
        for resource in walkdir::WalkDir::new(dir) {
            let resource = resource.map_err(|e| e.to_string())?;
            if !resource.file_type().is_file() {
                continue;
            }
            let mut prefix = [0u8; 128];
            let len = std::fs::File::open(resource.path())
                .and_then(|mut f| f.read(&mut prefix))
                .map_err(|e| e.to_string())?;
            if prefix[..len].starts_with(b"version https://git-lfs.github.com/spec/v1") {
                candidate.resource_errors.push(format!(
                    "Git LFS object is not included in the package: {}",
                    resource.path().strip_prefix(dir).unwrap().display()
                ));
            }
        }
        if let Ok(modules) = std::fs::read_to_string(root.join(".gitmodules")) {
            for line in modules.lines() {
                if let Some(("path", path)) = line
                    .trim()
                    .split_once('=')
                    .map(|(k, v)| (k.trim(), v.trim()))
                {
                    let path = path.trim_matches('"');
                    if Path::new(path).starts_with(&candidate.source.package_path) {
                        candidate
                            .resource_errors
                            .push(format!("Git submodule resources are not included: {path}"));
                    }
                }
            }
        }
        candidate.resource_errors.sort();
        candidate.resource_errors.dedup();
        candidates.push(candidate);
        // An explicit Skill package includes its resources, not nested Skills.
        if selected.join("SKILL.md").is_file() {
            break;
        }
    }
    if candidates.is_empty() {
        return Err("No SKILL.md found in the selected repository directory.".into());
    }
    Ok(candidates)
}

/// New installs only. Staging is outside the discovery root, so interruption
/// cannot expose a partially installed SKILL.md. Never replace a user package.
pub fn install_new(
    root: &Path,
    skills_dir: &Path,
    candidate: &SkillStoreCandidate,
) -> Result<PathBuf, String> {
    validate_source(&candidate.source)?;
    portable_path(&candidate.name, false)?;
    if candidate.name.contains('/')
        || candidate.name.starts_with('.')
        || !candidate.format_errors.is_empty()
        || !candidate.resource_errors.is_empty()
        || candidate.conflict.is_some()
    {
        return Err("Resolve validation errors or name conflicts before installing.".into());
    }
    let parent = skills_dir
        .parent()
        .ok_or("Skills directory needs a parent.")?;
    let staging = StagingDir::new(&parent.join("skill-store-staging"))?;
    let package = root.join(&candidate.source.package_path);
    for entry in walkdir::WalkDir::new(&package) {
        let entry = entry.map_err(|e| e.to_string())?;
        let relative = entry
            .path()
            .strip_prefix(&package)
            .map_err(|e| e.to_string())?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        if entry.file_type().is_symlink() {
            return Err("Package contains a symbolic link.".into());
        }
        let output = staging.0.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
        } else if entry.file_type().is_file() {
            std::fs::copy(entry.path(), output).map_err(|e| e.to_string())?;
        } else {
            return Err("Package contains a special file.".into());
        }
    }
    if !staging.0.join("SKILL.md").is_file() {
        return Err("Package has no SKILL.md.".into());
    }
    // Store provenance only after validating source; package-supplied origin
    // metadata can never claim a different installation identity.
    std::fs::write(
        staging.0.join(ORIGIN_FILE),
        serde_json::to_vec_pretty(&candidate.source).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(skills_dir).map_err(|e| e.to_string())?;
    let destination = skills_dir.join(&candidate.name);
    if std::fs::symlink_metadata(&destination).is_ok() {
        return Err(format!(
            "Keep existing package: {}. Store installation never overwrites files.",
            destination.display()
        ));
    }
    std::fs::rename(&staging.0, &destination)
        .map_err(|e| format!("Install commit failed; existing Skills were preserved: {e}"))?;
    Ok(destination)
}

pub fn read_origin(dir: &Path) -> Option<SkillInstallSource> {
    let path = dir.join(ORIGIN_FILE);
    if std::fs::symlink_metadata(&path)
        .ok()?
        .file_type()
        .is_symlink()
        || std::fs::metadata(&path).ok()?.len() > 16 * 1024
    {
        return None;
    }
    let source: SkillInstallSource = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    validate_source(&source).ok()?;
    Some(source)
}

pub fn parse_catalog(bytes: &[u8]) -> Result<Vec<CommunitySkillEntry>, String> {
    if bytes.len() > 1024 * 1024 {
        return Err("Community index exceeds 1 MiB.".into());
    }
    let entries: Vec<CommunitySkillEntry> =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid community index: {e}"))?;
    if entries.len() > 1000 {
        return Err("Community index exceeds 1,000 entries.".into());
    }
    let mut names = HashSet::new();
    for entry in &entries {
        if !names.insert(&entry.name) {
            return Err(format!("Duplicate community entry: {}", entry.name));
        }
        for field in [
            &entry.name,
            &entry.description,
            &entry.author,
            &entry.license,
            &entry.git_ref,
            &entry.responsibilities,
            &entry.when_to_use,
            &entry.inputs,
            &entry.outputs,
            &entry.out_of_scope,
            &entry.operation_boundary,
            &entry.supported_wisp,
            &entry.known_limits,
        ] {
            if field.trim().is_empty() {
                return Err(format!(
                    "Community entry '{}' has an empty required field.",
                    entry.name
                ));
            }
        }
        let link = parse_github_link(&format!("https://github.com/{}", entry.repository))?;
        if link.kind.is_some() {
            return Err("Catalog repository must be owner/repo.".into());
        }
        portable_path(&entry.package_path, true)?;
        let feedback = url::Url::parse(&entry.feedback_url).map_err(|e| e.to_string())?;
        if feedback.scheme() != "https" {
            return Err("Feedback URL must use HTTPS.".into());
        }
    }
    Ok(entries)
}

/// The transport returns None only for HTTP 404. Rate limits, network failures,
/// malformed responses and truncated data are errors, never guessed defaults.
#[async_trait::async_trait]
pub trait GithubApi: Sync {
    async fn json(&self, path: &str) -> Result<Option<serde_json::Value>, String>;
}

pub fn commit_api_path(repository: &str, git_ref: &str) -> String {
    let mut url = url::Url::parse(&format!(
        "https://api.github.com/repos/{repository}/commits/"
    ))
    .unwrap();
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .push(git_ref);
    url.path().to_string()
}

pub async fn resolve_source(
    api: &impl GithubApi,
    source_url: &str,
    exact_ref: &str,
) -> Result<SkillInstallSource, String> {
    let link = parse_github_link(source_url)?;
    let refs = if !exact_ref.trim().is_empty() {
        vec![exact_ref.trim().to_string()]
    } else if link.kind.is_none() {
        let repo = api
            .json(&format!("/repos/{}", link.repository))
            .await?
            .ok_or("Repository is unavailable (404).")?;
        vec![repo
            .get("default_branch")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .ok_or("GitHub did not return a default branch; provide the exact ref.")?
            .to_string()]
    } else {
        let parts: Vec<_> = link.tail.split('/').collect();
        if parts.len() > 16 {
            return Err("Provide the exact ref to disambiguate this long GitHub URL.".into());
        }
        (1..=parts.len())
            .map(|i| parts[..i].join("/"))
            .filter(|r| package_path_for_ref(&link, r).is_ok())
            .collect()
    };
    let mut resolved = Vec::new();
    for git_ref in refs {
        if let Some(value) = api
            .json(&commit_api_path(&link.repository, &git_ref))
            .await?
        {
            let commit = value
                .get("sha")
                .and_then(|v| v.as_str())
                .ok_or("GitHub commit response has no SHA.")?;
            let source = SkillInstallSource {
                repository: link.repository.clone(),
                source_url: source_url.trim().to_string(),
                package_path: package_path_for_ref(&link, &git_ref)?,
                git_ref,
                commit: commit.into(),
            };
            validate_source(&source)?;
            resolved.push(source);
        }
    }
    match resolved.len() {
        1 => Ok(resolved.remove(0)),
        0 => Err("No matching repository ref (404). Check the URL or provide the exact branch, tag, or commit.".into()),
        _ => Err(format!("Ambiguous ref/path boundary. Enter the exact ref: {}", resolved.iter().map(|s| s.git_ref.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SkillIndex, SkillSource};
    use std::collections::{BTreeMap, HashMap};

    fn source(path: &str) -> SkillInstallSource {
        SkillInstallSource {
            repository: "author/research".into(),
            source_url: "https://github.com/author/research".into(),
            git_ref: "release/v1".into(),
            commit: "a".repeat(40),
            package_path: path.into(),
        }
    }
    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, bytes) in files {
            writer
                .start_file(*path, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }
    const SKILL: &[u8] = b"---\nname: sample\ndescription: Example\n---\nRead [guide](references/guide.md) and `scripts/run.py`.";
    fn package() -> Vec<u8> {
        zip(&[
            ("repo/skills/sample/SKILL.md", SKILL),
            ("repo/skills/sample/references/guide.md", b"reference"),
            (
                "repo/skills/sample/scripts/run.py",
                b"raise RuntimeError('must never execute')",
            ),
            (
                "repo/skills/second/SKILL.md",
                b"---\nname: second\ndescription: Another skill\n---\nBody",
            ),
        ])
    }
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, String> {
        use sha2::Digest;
        walkdir::WalkDir::new(root)
            .into_iter()
            .map(Result::unwrap)
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                (
                    e.path().strip_prefix(root).unwrap().to_path_buf(),
                    hex::encode(sha2::Sha256::digest(std::fs::read(e.path()).unwrap())),
                )
            })
            .collect()
    }

    #[test]
    fn github_links_keep_slash_refs_and_file_package_resources() {
        for kind in ["tree", "blob"] {
            let suffix = if kind == "blob" { "/SKILL.md" } else { "" };
            let link = parse_github_link(&format!(
                "https://github.com/author/research/{kind}/release%2Fv1/skills/sample{suffix}"
            ))
            .unwrap();
            assert_eq!(
                package_path_for_ref(&link, "release/v1").unwrap(),
                "skills/sample"
            );
            assert!(package_path_for_ref(&link, "main").is_err());
        }
        let root_file =
            parse_github_link("https://github.com/author/research/blob/main/SKILL.md").unwrap();
        assert_eq!(package_path_for_ref(&root_file, "main").unwrap(), "");
        for url in [
            "http://github.com/a/b",
            "https://github.com.evil.test/a/b",
            "https://token@github.com/a/b",
            "https://github.com/a/b/issues/1",
            "https://github.com/a/b/blob/main/README.md",
            "https://github.com/a/b?token=secret",
            "https://github.com/a/b/tree/main/a%5Cb",
        ] {
            assert!(parse_github_link(url).is_err(), "{url}");
        }
    }

    struct MockGithub(HashMap<String, serde_json::Value>);
    #[async_trait::async_trait]
    impl GithubApi for MockGithub {
        async fn json(&self, path: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(self.0.get(path).cloned())
        }
    }
    #[tokio::test]
    async fn mock_github_resolves_reported_default_branch_tags_and_pinned_commit() {
        let api = MockGithub(HashMap::from([
            (
                "/repos/author/research".into(),
                serde_json::json!({"default_branch": "trunk"}),
            ),
            (
                commit_api_path("author/research", "trunk"),
                serde_json::json!({"sha": "b".repeat(40)}),
            ),
            (
                commit_api_path("author/research", "release/v1"),
                serde_json::json!({"sha": "a".repeat(40)}),
            ),
            (
                commit_api_path("author/research", &"a".repeat(40)),
                serde_json::json!({"sha": "a".repeat(40)}),
            ),
        ]));
        let default = resolve_source(&api, "https://github.com/author/research", "")
            .await
            .unwrap();
        assert_eq!(default.git_ref, "trunk");
        assert_eq!(default.commit, "b".repeat(40));
        let file = resolve_source(
            &api,
            "https://github.com/author/research/blob/release/v1/skills/sample/SKILL.md",
            "",
        )
        .await
        .unwrap();
        assert_eq!(file.git_ref, "release/v1");
        assert_eq!(file.package_path, "skills/sample");
        assert_eq!(
            resolve_source(&api, "https://github.com/author/research", &"a".repeat(40))
                .await
                .unwrap()
                .commit,
            "a".repeat(40)
        );
        assert!(resolve_source(&api, "https://github.com/missing/repo", "")
            .await
            .unwrap_err()
            .contains("404"));
    }
    #[tokio::test]
    async fn ambiguous_refs_require_explicit_choice_and_api_failures_do_not_guess() {
        let api = MockGithub(HashMap::from([
            (
                commit_api_path("author/research", "release"),
                serde_json::json!({"sha": "b".repeat(40)}),
            ),
            (
                commit_api_path("author/research", "release/v1"),
                serde_json::json!({"sha": "a".repeat(40)}),
            ),
        ]));
        let url = "https://github.com/author/research/tree/release/v1/skills/sample";
        assert!(resolve_source(&api, url, "")
            .await
            .unwrap_err()
            .contains("Ambiguous"));
        assert_eq!(
            resolve_source(&api, url, "release/v1")
                .await
                .unwrap()
                .package_path,
            "skills/sample"
        );
        struct RateLimit;
        #[async_trait::async_trait]
        impl GithubApi for RateLimit {
            async fn json(&self, _: &str) -> Result<Option<serde_json::Value>, String> {
                Err("GitHub rate limit".into())
            }
        }
        assert_eq!(
            resolve_source(&RateLimit, url, "").await.unwrap_err(),
            "GitHub rate limit"
        );
    }

    #[test]
    fn complete_package_installs_only_selected_skill_and_never_executes_scripts() {
        let temp = StagingDir::new(&std::env::temp_dir()).unwrap();
        let root = extract_repository(&package(), &temp.0.join("unpack")).unwrap();
        let all = inspect_repository(&root, &source("")).unwrap();
        assert_eq!(all.len(), 2);
        let candidates = inspect_repository(&root, &source("skills/sample")).unwrap();
        let candidate = &candidates[0];
        assert!(candidate.format_errors.is_empty());
        assert!(candidate.resource_errors.is_empty());
        let global = temp.0.join("user/skills");
        let destination = install_new(&root, &global, candidate).unwrap();
        assert!(destination.join("scripts/run.py").is_file());
        assert_eq!(
            std::fs::read_to_string(destination.join("references/guide.md")).unwrap(),
            "reference"
        );
        assert!(!global.join("second").exists());
        assert_eq!(read_origin(&destination), Some(candidate.source.clone()));
        assert_eq!(
            SkillIndex::load_scoped(&[(global, SkillSource::Global)])
                .all()
                .len(),
            1
        );
    }

    #[test]
    fn lfs_and_submodules_are_reported_as_incomplete_resources() {
        let temp = StagingDir::new(&std::env::temp_dir()).unwrap();
        let archive = zip(&[
            ("repo/skills/sample/SKILL.md", b"---\nname: sample\ndescription: Example\n---\nBody"),
            ("repo/skills/sample/assets/data.bin", b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 10000"),
            ("repo/.gitmodules", b"[submodule \"data\"]\npath = skills/sample/data\nurl = https://example.test/data.git"),
        ]);
        let root = extract_repository(&archive, &temp.0).unwrap();
        let candidate = inspect_repository(&root, &source("skills/sample"))
            .unwrap()
            .remove(0);
        assert!(candidate
            .resource_errors
            .iter()
            .any(|e| e.contains("Git LFS")));
        assert!(candidate
            .resource_errors
            .iter()
            .any(|e| e.contains("submodule")));
        assert!(install_new(&root, &temp.0.join("user/skills"), &candidate).is_err());
    }

    #[test]
    fn repeat_install_and_local_modifications_are_preserved() {
        let temp = StagingDir::new(&std::env::temp_dir()).unwrap();
        let root = extract_repository(&package(), &temp.0.join("unpack")).unwrap();
        let candidate = inspect_repository(&root, &source("skills/sample"))
            .unwrap()
            .remove(0);
        let global = temp.0.join("user/skills");
        let destination = install_new(&root, &global, &candidate).unwrap();
        std::fs::write(destination.join("references/guide.md"), "user edits").unwrap();
        let before = snapshot(&global);
        assert!(install_new(&root, &global, &candidate)
            .unwrap_err()
            .contains("existing package"));
        assert_eq!(snapshot(&global), before);
    }

    #[test]
    fn validation_errors_are_separate_and_prevent_installation() {
        let temp = StagingDir::new(&std::env::temp_dir()).unwrap();
        let archive = zip(&[
            ("repo/missing/SKILL.md", SKILL),
            (
                "repo/invalid/SKILL.md",
                b"---\nname: bad\nwisp:\n  schema_version: 1\n  roles: [oracle]\n---\nBody",
            ),
        ]);
        let root = extract_repository(&archive, &temp.0.join("unpack")).unwrap();
        let all = inspect_repository(&root, &source("")).unwrap();
        let bad = all.iter().find(|c| c.name == "invalid").unwrap();
        assert!(bad.format_errors[0].contains("wisp.roles"));
        let missing = all.iter().find(|c| c.name == "sample").unwrap();
        assert!(missing.format_errors.is_empty());
        assert_eq!(missing.resource_errors.len(), 2);
        for candidate in &all {
            assert!(install_new(&root, &temp.0.join("user/skills"), candidate).is_err());
        }
        assert!(!temp.0.join("user/skills").exists());
    }

    #[test]
    fn zip_rejects_traversal_symlinks_case_aliases_windows_paths_and_size_limits() {
        for name in [
            "repo/../../escaped",
            "repo/../escaped",
            "repo/a\\b",
            "/repo/absolute",
            "repo/C:/file",
            "repo/CON.txt",
            "repo/trailing.",
        ] {
            let temp = StagingDir::new(&std::env::temp_dir()).unwrap();
            assert!(
                extract_repository(&zip(&[(name, b"bad")]), &temp.0).is_err(),
                "{name}"
            );
        }
        let temp = StagingDir::new(&std::env::temp_dir()).unwrap();
        assert!(extract_repository(
            &zip(&[("repo/a", b"one"), ("repo/A", b"two")]),
            &temp.0.join("collision")
        )
        .is_err());
        assert!(extract_repository(
            &zip(&[("repo/dir/a", b"one"), ("repo/Dir/b", b"two")]),
            &temp.0.join("directory-case")
        )
        .is_err());
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .add_symlink(
                "repo/link",
                "../../target",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        assert!(extract_repository(
            &writer.finish().unwrap().into_inner(),
            &temp.0.join("symlink")
        )
        .unwrap_err()
        .contains("Symbolic"));
        assert!(extract_repository(
            &zip(&[("repo/big", &vec![b'x'; MAX_FILE_BYTES as usize + 1])]),
            &temp.0.join("large")
        )
        .unwrap_err()
        .contains("8 MiB"));
        assert!(
            extract_repository(&vec![0; MAX_ARCHIVE_BYTES + 1], &temp.0.join("archive"))
                .unwrap_err()
                .contains("32 MiB")
        );
        assert!(extract_repository(&package()[..40], &temp.0.join("truncated")).is_err());
    }

    #[test]
    fn legacy_root_skill_uses_repository_name_not_archive_commit_suffix() {
        let temp = StagingDir::new(&std::env::temp_dir()).unwrap();
        let root = extract_repository(
            &zip(&[(
                "research-0123456789/SKILL.md",
                b"---\ndescription: Legacy root package\n---\nBody",
            )]),
            &temp.0,
        )
        .unwrap();
        let candidate = inspect_repository(&root, &source("")).unwrap().remove(0);
        assert_eq!(candidate.name, "research");
        assert!(candidate.format_errors.is_empty());
    }

    #[test]
    fn example_links_in_fenced_code_are_not_required_resources() {
        let temp = StagingDir::new(&std::env::temp_dir()).unwrap();
        let root = extract_repository(&zip(&[("repo/SKILL.md", b"---\nname: sample\ndescription: Example\n---\n```markdown\n[example](not-a-resource.md)\n```\nNo package resources required.")]), &temp.0).unwrap();
        assert!(inspect_repository(&root, &source("")).unwrap()[0]
            .resource_errors
            .is_empty());
    }

    #[test]
    fn missing_parent_resource_and_encoded_traversal_are_rejected() {
        let temp = StagingDir::new(&std::env::temp_dir()).unwrap();
        let root = extract_repository(&zip(&[("repo/SKILL.md", b"---\nname: sample\ndescription: Example\n---\n[bad](../outside.md) [encoded](%2e%2e/outside.md)")]), &temp.0).unwrap();
        let candidate = inspect_repository(&root, &source("")).unwrap().remove(0);
        assert!(!candidate.resource_errors.is_empty());
    }

    #[test]
    fn shipped_directory_and_authoring_examples_match_current_parser() {
        let entries =
            parse_catalog(include_bytes!("../../../community-skills/index.json")).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for entry in entries {
            // Third-party entries are schema-checked above; maintainers review
            // their external packages without making CI depend on GitHub.
            if entry.repository != "xuzhougeng/wisp-science" {
                continue;
            }
            let source = SkillInstallSource {
                repository: entry.repository.clone(),
                source_url: format!("https://github.com/{}", entry.repository),
                git_ref: entry.git_ref,
                commit: "a".repeat(40),
                package_path: entry.package_path,
            };
            let candidates = inspect_repository(&repo, &source).unwrap();
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0].name, entry.name);
            assert!(candidates[0].format_errors.is_empty());
            assert!(candidates[0].resource_errors.is_empty());
        }
        let guide = include_str!("../../../docs/skill-authoring.md");
        for block in guide.split("```skill\n").skip(1) {
            crate::manifest::parse_skill_document(
                block.split("```").next().unwrap(),
                "example".into(),
            )
            .unwrap();
        }
    }

    #[test]
    fn simulated_app_upgrade_downgrade_and_interruption_preserve_user_packages_and_choices() {
        for layout in ["Windows/resources", "macOS/App.app/Contents/Resources"] {
            let temp = StagingDir::new(&std::env::temp_dir()).unwrap();
            let root = extract_repository(&package(), &temp.0.join("unpack")).unwrap();
            let candidate = inspect_repository(&root, &source("skills/sample"))
                .unwrap()
                .remove(0);
            let global = temp.0.join("user/.wisp/skills");
            let dest = install_new(&root, &global, &candidate).unwrap();
            std::fs::write(dest.join("notes.txt"), "local modification").unwrap();
            let project = temp.0.join("project/.wisp/skills");
            std::fs::create_dir_all(project.join("project-skill")).unwrap();
            std::fs::write(
                project.join("project-skill/SKILL.md"),
                b"---\nname: project-skill\ndescription: project\n---\nBody",
            )
            .unwrap();
            let user_before = snapshot(&global);
            let project_before = snapshot(&project);
            let tags = BTreeMap::from([("sample".into(), vec!["personal".into()])]);
            let enabled = HashSet::from(["project-skill".into()]);
            let bundled = temp.0.join(layout).join("skills");
            for installed_bundle in [
                Some("old-bundled"),
                Some("sample"),
                None,
                Some("old-bundled"),
            ] {
                if bundled.exists() {
                    std::fs::remove_dir_all(&bundled).unwrap();
                }
                if let Some(name) = installed_bundle {
                    std::fs::create_dir_all(bundled.join(name)).unwrap();
                    std::fs::write(
                        bundled.join(name).join("SKILL.md"),
                        format!("---\nname: {name}\ndescription: bundled\n---\nBody"),
                    )
                    .unwrap();
                }
                // Even a fully copied abandoned staging package is undiscovered.
                let interrupted =
                    StagingDir::new(&temp.0.join("user/.wisp/skill-store-staging")).unwrap();
                std::fs::write(interrupted.0.join("SKILL.md"), SKILL).unwrap();
                let index = SkillIndex::load_scoped(&[
                    (bundled.clone(), SkillSource::Bundled),
                    (project.clone(), SkillSource::Project),
                    (global.clone(), SkillSource::Global),
                ])
                .with_tag_overrides(&tags);
                assert_eq!(index.get("sample").unwrap().tags, ["personal"]);
                assert!(index
                    .filtered_by_names(Some(&enabled))
                    .get("sample")
                    .is_none());
                assert_eq!(
                    index
                        .catalog_records()
                        .iter()
                        .filter(|r| r.name == "sample")
                        .count(),
                    if installed_bundle == Some("sample") {
                        2
                    } else {
                        1
                    }
                );
                assert_eq!(snapshot(&global), user_before);
                assert_eq!(snapshot(&project), project_before);
                assert_eq!(read_origin(&dest).unwrap().commit, "a".repeat(40));
            }
        }
    }
}
