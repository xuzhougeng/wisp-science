//! Install and activate self-contained feature plugins.
//!
//! The first compatibility format is the Claude plugin layout used by Motif:
//! `.claude-plugin/plugin.json`, `.mcp.json`, and `skills/*/SKILL.md`. Packages
//! are normalized into a host-owned manifest before they are persisted. Install
//! never executes package code; MCP entrypoints start only after a project-level
//! enable action.

use crate::{clear_idle_agents, AppState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, State};

const PLUGIN_SCHEMA: &str = "wisp.plugin.v1";
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 10_000;
const MAX_PATH_BYTES: usize = 768;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PluginMcpServer {
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NormalizedPluginManifest {
    pub schema: String,
    pub id: String,
    pub display_name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    pub source_format: String,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<PluginMcpServer>,
}

#[derive(Debug)]
pub(crate) struct PluginMcpLaunch {
    pub connector_id: String,
    pub display_name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub install_root: PathBuf,
}

impl NormalizedPluginManifest {
    pub(crate) fn from_installation(
        installation: &wisp_store::PluginInstallation,
    ) -> Result<Self, String> {
        serde_json::from_str(&installation.manifest_json)
            .map_err(|error| format!("invalid installed plugin manifest: {error}"))
    }

    pub(crate) fn skill_paths(&self, install_root: &Path) -> Vec<PathBuf> {
        self.skills
            .iter()
            .map(|path| install_root.join(path))
            .filter(|path| path.is_dir())
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PluginView {
    id: String,
    version: String,
    display_name: String,
    description: String,
    author: String,
    license: String,
    source_uri: String,
    archive_sha256: String,
    trust_state: String,
    enabled: bool,
    skill_count: usize,
    skill_names: Vec<String>,
    mcp_server_count: usize,
    commands: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudePluginManifest {
    name: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: serde_json::Value,
    #[serde(default)]
    license: String,
}

#[derive(Debug, Deserialize)]
struct RawMcpServer {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug)]
struct PreparedPlugin {
    package_root: PathBuf,
    manifest: NormalizedPluginManifest,
    sha256: String,
    source_uri: String,
    trust_state: String,
    staging_root: PathBuf,
}

fn validate_plugin_id(value: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value.len() <= 96
        && value.bytes().enumerate().all(|(index, byte)| match byte {
            b'a'..=b'z' | b'0'..=b'9' => true,
            b'-' => index > 0 && index + 1 < value.len(),
            _ => false,
        })
        && !value.contains("--");
    if valid {
        Ok(())
    } else {
        Err("plugin id must be lowercase kebab-case and at most 96 characters".into())
    }
}

fn validate_relative_path(value: &str, label: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.len() > MAX_PATH_BYTES {
        return Err(format!("{label} is empty or too long"));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("{label} must stay inside the plugin directory"));
    }
    Ok(path.to_path_buf())
}

fn author_name(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            value
                .as_object()
                .and_then(|object| object.get("name"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default()
}

fn find_package_root(root: &Path) -> Result<PathBuf, String> {
    let direct = [
        root.join(".wisp-plugin").join("plugin.json"),
        root.join(".claude-plugin").join("plugin.json"),
    ];
    if direct.iter().any(|path| path.is_file()) {
        return Ok(root.to_path_buf());
    }
    let mut candidates = Vec::new();
    let entries =
        std::fs::read_dir(root).map_err(|error| format!("read plugin package: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("read plugin package entry: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| format!("inspect plugin package entry: {error}"))?
            .is_dir()
        {
            let path = entry.path();
            if path.join(".wisp-plugin/plugin.json").is_file()
                || path.join(".claude-plugin/plugin.json").is_file()
            {
                candidates.push(path);
            }
        }
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(
            "plugin package has no .wisp-plugin/plugin.json or .claude-plugin/plugin.json".into(),
        ),
        _ => Err("plugin package contains more than one plugin root".into()),
    }
}

fn parse_manifest(package_root: &Path) -> Result<NormalizedPluginManifest, String> {
    let native = package_root.join(".wisp-plugin/plugin.json");
    if native.is_file() {
        let bytes = std::fs::read(&native)
            .map_err(|error| format!("read Wisp plugin manifest: {error}"))?;
        let manifest: NormalizedPluginManifest = serde_json::from_slice(&bytes)
            .map_err(|error| format!("parse Wisp plugin manifest: {error}"))?;
        validate_manifest(package_root, manifest)
    } else {
        parse_claude_plugin(package_root)
    }
}

fn parse_claude_plugin(package_root: &Path) -> Result<NormalizedPluginManifest, String> {
    let manifest_path = package_root.join(".claude-plugin/plugin.json");
    let raw: ClaudePluginManifest = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .map_err(|error| format!("read Claude plugin manifest: {error}"))?,
    )
    .map_err(|error| format!("parse Claude plugin manifest: {error}"))?;

    let mut skills = Vec::new();
    let skills_root = package_root.join("skills");
    if skills_root.is_dir() {
        for entry in std::fs::read_dir(&skills_root)
            .map_err(|error| format!("read plugin skills: {error}"))?
        {
            let entry = entry.map_err(|error| format!("read plugin skill entry: {error}"))?;
            let path = entry.path();
            if entry
                .file_type()
                .map_err(|error| format!("inspect plugin skill entry: {error}"))?
                .is_dir()
                && path.join("SKILL.md").is_file()
            {
                skills.push(
                    path.strip_prefix(package_root)
                        .map_err(|_| "plugin skill escaped package root".to_string())?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    skills.sort();

    let mcp_path = package_root.join(".mcp.json");
    let mut mcp_servers = Vec::new();
    if mcp_path.is_file() {
        let servers: BTreeMap<String, RawMcpServer> = serde_json::from_slice(
            &std::fs::read(&mcp_path).map_err(|error| format!("read .mcp.json: {error}"))?,
        )
        .map_err(|error| format!("parse .mcp.json: {error}"))?;
        for (id, server) in servers {
            if id.trim().is_empty() || server.command.trim().is_empty() {
                return Err("MCP server id and command must be non-empty".into());
            }
            mcp_servers.push(PluginMcpServer {
                id,
                command: server.command,
                args: server.args,
                env: server.env,
                cwd: server.cwd,
            });
        }
    }

    validate_manifest(
        package_root,
        NormalizedPluginManifest {
            schema: PLUGIN_SCHEMA.into(),
            id: raw.name.clone(),
            display_name: raw.display_name.unwrap_or(raw.name),
            version: raw.version,
            description: raw.description,
            author: author_name(&raw.author),
            license: raw.license,
            source_format: "claude-plugin".into(),
            skills,
            mcp_servers,
        },
    )
}

fn validate_manifest(
    package_root: &Path,
    mut manifest: NormalizedPluginManifest,
) -> Result<NormalizedPluginManifest, String> {
    if manifest.schema != PLUGIN_SCHEMA {
        return Err(format!("unsupported plugin schema '{}'", manifest.schema));
    }
    validate_plugin_id(&manifest.id)?;
    semver::Version::parse(&manifest.version)
        .map_err(|error| format!("plugin version must be semantic versioning: {error}"))?;
    manifest.display_name = manifest.display_name.trim().to_string();
    if manifest.display_name.is_empty() || manifest.display_name.chars().count() > 120 {
        return Err("plugin display name is empty or too long".into());
    }
    if manifest.description.chars().count() > 2_000 {
        return Err("plugin description is too long".into());
    }
    for skill in &manifest.skills {
        let relative = validate_relative_path(skill, "plugin skill path")?;
        if !package_root.join(relative).join("SKILL.md").is_file() {
            return Err(format!("plugin skill '{}' has no SKILL.md", skill));
        }
    }
    let mut server_ids = HashSet::new();
    for server in &manifest.mcp_servers {
        if !server_ids.insert(server.id.clone()) {
            return Err(format!("duplicate MCP server id '{}'", server.id));
        }
        if server.command.trim().is_empty() || server.command.contains(['\0', '\n', '\r']) {
            return Err(format!("invalid MCP command for server '{}'", server.id));
        }
        for argument in &server.args {
            if argument.contains(['\0', '\n', '\r']) {
                return Err(format!("invalid MCP argument for server '{}'", server.id));
            }
        }
        if let Some(cwd) = &server.cwd {
            validate_relative_path(cwd, "plugin MCP cwd")?;
        }
    }
    Ok(manifest)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("stat plugin archive: {error}"))?;
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(format!(
            "plugin archive exceeds {} MiB",
            MAX_ARCHIVE_BYTES / 1024 / 1024
        ));
    }
    let mut file = File::open(path).map_err(|error| format!("open plugin archive: {error}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("read plugin archive: {error}"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn tree_sha256(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| format!("walk plugin directory: {error}"))?;
        if entry.file_type().is_symlink() {
            return Err(format!(
                "plugin directory contains a symbolic link: {}",
                entry.path().display()
            ));
        }
        if entry.file_type().is_file() {
            files.push(entry.path().to_path_buf());
        }
    }
    if files.len() > MAX_FILES {
        return Err(format!("plugin contains more than {MAX_FILES} files"));
    }
    files.sort_by(|left, right| {
        left.strip_prefix(root)
            .unwrap_or(left)
            .cmp(right.strip_prefix(root).unwrap_or(right))
    });
    let mut hash = Sha256::new();
    let mut expanded = 0u64;
    for path in files {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "plugin file escaped source root".to_string())?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        if relative.len() > MAX_PATH_BYTES {
            return Err("plugin path is too long".into());
        }
        let size = path
            .metadata()
            .map_err(|error| format!("stat plugin file: {error}"))?
            .len();
        if size > MAX_FILE_BYTES {
            return Err(format!("plugin file '{}' is too large", relative));
        }
        expanded = expanded.saturating_add(size);
        if expanded > MAX_EXPANDED_BYTES {
            return Err("plugin expanded size exceeds safety limit".into());
        }
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        hash.update(size.to_le_bytes());
        let mut file = File::open(&path).map_err(|error| format!("open plugin file: {error}"))?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| format!("read plugin file: {error}"))?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn validate_expected_sha256(expected: Option<&str>, actual: &str) -> Result<String, String> {
    let Some(expected) = expected.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok("unverified".into());
    };
    validate_sha256_text(expected)?;
    if !expected.eq_ignore_ascii_case(actual) {
        return Err(format!(
            "plugin SHA-256 mismatch: expected {}, got {}",
            expected.to_ascii_lowercase(),
            actual
        ));
    }
    Ok("checksum_verified".into())
}

fn validate_sha256_text(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("expected SHA-256 must contain exactly 64 hexadecimal characters".into());
    }
    Ok(())
}

fn validate_plugin_url(value: &str) -> Result<url::Url, String> {
    let parsed =
        url::Url::parse(value.trim()).map_err(|error| format!("invalid plugin URL: {error}"))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err("remote plugin URL must use HTTPS".into());
    }
    Ok(parsed)
}

fn extract_zip(archive_path: &Path, destination: &Path) -> Result<(), String> {
    let file = File::open(archive_path).map_err(|error| format!("open plugin ZIP: {error}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|error| format!("read plugin ZIP: {error}"))?;
    if archive.len() > MAX_FILES {
        return Err(format!("plugin ZIP contains more than {MAX_FILES} entries"));
    }
    let mut seen = HashSet::new();
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("read plugin ZIP entry: {error}"))?;
        let Some(enclosed) = entry.enclosed_name() else {
            return Err(format!(
                "plugin ZIP entry '{}' escapes the package",
                entry.name()
            ));
        };
        let normalized = enclosed.to_string_lossy().replace('\\', "/");
        if normalized.is_empty() || normalized.len() > MAX_PATH_BYTES {
            return Err("plugin ZIP contains an empty or overly long path".into());
        }
        if !seen.insert(normalized.clone()) {
            return Err(format!("plugin ZIP contains duplicate path '{normalized}'"));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!("plugin ZIP contains symbolic link '{normalized}'"));
        }
        let size = entry.size();
        if size > MAX_FILE_BYTES {
            return Err(format!("plugin ZIP entry '{normalized}' is too large"));
        }
        expanded = expanded.saturating_add(size);
        if expanded > MAX_EXPANDED_BYTES {
            return Err("plugin ZIP expanded size exceeds safety limit".into());
        }
        let output = destination.join(&enclosed);
        if entry.is_dir() {
            std::fs::create_dir_all(&output)
                .map_err(|error| format!("create plugin directory: {error}"))?;
            continue;
        }
        let parent = output
            .parent()
            .ok_or_else(|| "plugin ZIP entry has no parent".to_string())?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create plugin directory: {error}"))?;
        let mut target = File::create(&output)
            .map_err(|error| format!("create plugin file '{}': {error}", output.display()))?;
        std::io::copy(&mut entry, &mut target)
            .map_err(|error| format!("extract plugin file '{}': {error}", output.display()))?;
        target
            .flush()
            .map_err(|error| format!("flush plugin file '{}': {error}", output.display()))?;
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|error| format!("walk plugin directory: {error}"))?;
        if entry.file_type().is_symlink() {
            return Err(format!(
                "plugin directory contains symbolic link '{}'; package a regular file instead",
                entry.path().display()
            ));
        }
        entries.push(entry.into_path());
    }
    if entries.len() > MAX_FILES + 1 {
        return Err(format!("plugin contains more than {MAX_FILES} files"));
    }
    for path in entries {
        let relative = path
            .strip_prefix(source)
            .map_err(|_| "plugin source escaped selected directory".to_string())?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let output = destination.join(relative);
        let metadata = std::fs::metadata(&path)
            .map_err(|error| format!("stat plugin source '{}': {error}", path.display()))?;
        if metadata.is_dir() {
            std::fs::create_dir_all(&output)
                .map_err(|error| format!("create plugin directory: {error}"))?;
        } else if metadata.is_file() {
            if metadata.len() > MAX_FILE_BYTES {
                return Err(format!("plugin file '{}' is too large", relative.display()));
            }
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("create plugin directory: {error}"))?;
            }
            std::fs::copy(&path, &output)
                .map_err(|error| format!("copy plugin file '{}': {error}", path.display()))?;
        }
    }
    Ok(())
}

fn prepare_plugin(
    source: &Path,
    expected_sha256: Option<&str>,
    app_data: &Path,
    source_uri: String,
) -> Result<PreparedPlugin, String> {
    let staging_root = app_data
        .join("plugin-staging")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&staging_root)
        .map_err(|error| format!("create plugin staging directory: {error}"))?;
    let result = (|| {
        let (sha256, trust_state, unpacked) = if source.is_dir() {
            let sha256 = tree_sha256(source)?;
            let trust_state = validate_expected_sha256(expected_sha256, &sha256)?;
            let unpacked = staging_root.join("package");
            std::fs::create_dir_all(&unpacked)
                .map_err(|error| format!("create plugin staging package: {error}"))?;
            copy_directory(source, &unpacked)?;
            (sha256, trust_state, unpacked)
        } else {
            if !source.is_file() {
                return Err("plugin source does not exist or is not a regular file".into());
            }
            let extension = source
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if !extension.eq_ignore_ascii_case("zip") {
                return Err("plugin archive must be a .zip file".into());
            }
            let sha256 = sha256_file(source)?;
            // Authenticate a release archive before parsing or extracting any
            // of its package contents.
            let trust_state = validate_expected_sha256(expected_sha256, &sha256)?;
            let unpacked = staging_root.join("package");
            std::fs::create_dir_all(&unpacked)
                .map_err(|error| format!("create plugin staging package: {error}"))?;
            extract_zip(source, &unpacked)?;
            (sha256, trust_state, unpacked)
        };
        let package_root = find_package_root(&unpacked)?;
        let manifest = parse_manifest(&package_root)?;
        Ok(PreparedPlugin {
            package_root,
            manifest,
            sha256,
            source_uri,
            trust_state,
            staging_root: staging_root.clone(),
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging_root);
    }
    result
}

fn install_prepared(
    prepared: &PreparedPlugin,
    app_data: &Path,
) -> Result<(PathBuf, wisp_store::PluginInstallation), String> {
    let install_root = app_data
        .join("plugins")
        .join(&prepared.manifest.id)
        .join(&prepared.manifest.version);
    if install_root.exists() {
        return Err(format!(
            "plugin '{} {}' is already installed",
            prepared.manifest.id, prepared.manifest.version
        ));
    }
    let parent = install_root
        .parent()
        .ok_or_else(|| "plugin install directory has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create plugin install directory: {error}"))?;
    std::fs::rename(&prepared.package_root, &install_root)
        .map_err(|error| format!("commit plugin installation: {error}"))?;
    let now = chrono::Utc::now().timestamp();
    let installation = wisp_store::PluginInstallation {
        plugin_id: prepared.manifest.id.clone(),
        version: prepared.manifest.version.clone(),
        display_name: prepared.manifest.display_name.clone(),
        description: prepared.manifest.description.clone(),
        author: prepared.manifest.author.clone(),
        license: prepared.manifest.license.clone(),
        source_uri: prepared.source_uri.clone(),
        install_root: install_root.to_string_lossy().to_string(),
        archive_sha256: prepared.sha256.clone(),
        manifest_json: serde_json::to_string(&prepared.manifest)
            .map_err(|error| format!("serialize normalized plugin manifest: {error}"))?,
        trust_state: prepared.trust_state.clone(),
        installed_at: now,
        updated_at: now,
    };
    Ok((install_root, installation))
}

fn plugin_view(
    installation: wisp_store::PluginInstallation,
    enabled: bool,
) -> Result<PluginView, String> {
    let manifest = NormalizedPluginManifest::from_installation(&installation)?;
    let commands = manifest
        .mcp_servers
        .iter()
        .map(|server| {
            if server.args.is_empty() {
                server.command.clone()
            } else {
                format!("{} {}", server.command, server.args.join(" "))
            }
        })
        .collect();
    Ok(PluginView {
        id: installation.plugin_id,
        version: installation.version,
        display_name: installation.display_name,
        description: installation.description,
        author: installation.author,
        license: installation.license,
        source_uri: installation.source_uri,
        archive_sha256: installation.archive_sha256,
        trust_state: installation.trust_state,
        enabled,
        skill_count: manifest.skills.len(),
        skill_names: manifest
            .skills
            .iter()
            .filter_map(|path| path.trim_end_matches('/').rsplit('/').next())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect(),
        mcp_server_count: manifest.mcp_servers.len(),
        commands,
    })
}

pub(crate) async fn enabled_plugin_manifests(
    store: &wisp_store::Store,
    project_id: &str,
) -> Vec<(wisp_store::PluginInstallation, NormalizedPluginManifest)> {
    let installations = store
        .list_enabled_plugin_installations(project_id)
        .await
        .unwrap_or_default();
    installations
        .into_iter()
        .filter_map(|installation| {
            NormalizedPluginManifest::from_installation(&installation)
                .ok()
                .map(|manifest| (installation, manifest))
        })
        .collect()
}

fn expand_plugin_root(value: &str, install_root: &Path) -> Result<String, String> {
    let root = install_root.to_string_lossy();
    let expanded = value
        .replace("${CLAUDE_PLUGIN_ROOT}", &root)
        .replace("${WISP_PLUGIN_ROOT}", &root);
    if expanded.contains("${") || expanded.contains('\0') {
        return Err(format!("unsupported variable in plugin value '{value}'"));
    }
    Ok(expanded)
}

fn plugin_mcp_launch(
    installation: &wisp_store::PluginInstallation,
    manifest: &NormalizedPluginManifest,
    server: &PluginMcpServer,
) -> Result<PluginMcpLaunch, String> {
    let install_root = PathBuf::from(&installation.install_root)
        .canonicalize()
        .map_err(|error| format!("resolve plugin install directory: {error}"))?;
    let command_value = expand_plugin_root(&server.command, &install_root)?;
    let command = if command_value.contains(['/', '\\']) {
        let path = PathBuf::from(&command_value)
            .canonicalize()
            .map_err(|error| format!("resolve plugin MCP command '{command_value}': {error}"))?;
        if !path.starts_with(&install_root) || !path.is_file() {
            return Err(
                "plugin MCP command path must be a file inside the plugin directory".into(),
            );
        }
        path
    } else {
        which::which(&command_value)
            .map_err(|_| format!("plugin MCP executable '{command_value}' was not found in PATH"))?
    };
    let args = server
        .args
        .iter()
        .map(|argument| expand_plugin_root(argument, &install_root))
        .collect::<Result<Vec<_>, _>>()?;
    let mut env = BTreeMap::new();
    for (key, value) in &server.env {
        let valid_key = !key.is_empty()
            && key.len() <= 128
            && key.bytes().enumerate().all(|(index, byte)| {
                byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
            });
        if !valid_key {
            return Err(format!("plugin MCP environment key '{key}' is invalid"));
        }
        env.insert(key.clone(), expand_plugin_root(value, &install_root)?);
    }
    let cwd = if let Some(relative) = &server.cwd {
        let relative = validate_relative_path(relative, "plugin MCP cwd")?;
        let path = install_root
            .join(relative)
            .canonicalize()
            .map_err(|error| {
                format!(
                    "resolve plugin MCP working directory '{}': {error}",
                    server.id
                )
            })?;
        if !path.starts_with(&install_root) || !path.is_dir() {
            return Err(
                "plugin MCP working directory must stay inside the plugin directory".into(),
            );
        }
        path
    } else {
        install_root.clone()
    };
    Ok(PluginMcpLaunch {
        connector_id: format!("plugin:{}:{}", manifest.id, server.id),
        display_name: format!("{} / {}", manifest.display_name, server.id),
        command,
        args,
        env,
        cwd,
        install_root,
    })
}

pub(crate) async fn enabled_plugin_mcp_launches(
    store: &wisp_store::Store,
    project_id: &str,
) -> (Vec<PluginMcpLaunch>, Vec<String>) {
    let mut launches = Vec::new();
    let mut errors = Vec::new();
    for (installation, manifest) in enabled_plugin_manifests(store, project_id).await {
        for server in &manifest.mcp_servers {
            match plugin_mcp_launch(&installation, &manifest, server) {
                Ok(launch) => launches.push(launch),
                Err(error) => errors.push(format!(
                    "Plugin '{}' MCP '{}': {error}",
                    manifest.display_name, server.id
                )),
            }
        }
    }
    (launches, errors)
}

#[tauri::command]
pub(super) async fn list_plugins(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<PluginView>, String> {
    let project = state.active(window.label());
    let bindings = state
        .store
        .list_project_plugins(&project.id)
        .await
        .map_err(|error| error.to_string())?;
    let enabled: HashSet<(String, String)> = bindings
        .into_iter()
        .filter(|binding| binding.enabled)
        .map(|binding| (binding.plugin_id, binding.version))
        .collect();
    state
        .store
        .list_plugin_installations()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|installation| {
            let is_enabled =
                enabled.contains(&(installation.plugin_id.clone(), installation.version.clone()));
            plugin_view(installation, is_enabled)
        })
        .collect()
}

#[tauri::command]
pub(super) async fn pick_plugin_source(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Wisp or Claude plugin", &["zip"])
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    receiver
        .await
        .map_err(|error| error.to_string())
        .map(|path| path.map(|path| path.to_string()))
}

#[tauri::command]
pub(super) async fn install_plugin(
    state: State<'_, AppState>,
    src_path: String,
    expected_sha256: Option<String>,
) -> Result<PluginView, String> {
    let source = PathBuf::from(&src_path);
    let source_uri = source
        .canonicalize()
        .unwrap_or_else(|_| source.clone())
        .to_string_lossy()
        .to_string();
    install_plugin_path(&state, source, expected_sha256, source_uri).await
}

async fn install_plugin_path(
    state: &AppState,
    source: PathBuf,
    expected_sha256: Option<String>,
    source_uri: String,
) -> Result<PluginView, String> {
    let app_data = state.app_data.clone();
    let expected = expected_sha256.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        prepare_plugin(&source, expected.as_deref(), &app_data, source_uri)
    })
    .await
    .map_err(|error| format!("plugin installer task failed: {error}"))??;
    let app_data = state.app_data.clone();
    let prepared_for_commit = prepared;
    let (install_root, installation, staging_root) = tokio::task::spawn_blocking(move || {
        let (install_root, installation) = install_prepared(&prepared_for_commit, &app_data)?;
        Ok::<_, String>((install_root, installation, prepared_for_commit.staging_root))
    })
    .await
    .map_err(|error| format!("plugin commit task failed: {error}"))??;
    if let Err(error) = state.store.upsert_plugin_installation(&installation).await {
        let _ = tokio::fs::remove_dir_all(&install_root).await;
        let _ = tokio::fs::remove_dir_all(&staging_root).await;
        return Err(format!("save plugin installation: {error}"));
    }
    let _ = tokio::fs::remove_dir_all(&staging_root).await;
    plugin_view(installation, false)
}

#[tauri::command]
pub(super) async fn install_plugin_url(
    state: State<'_, AppState>,
    source_url: String,
    expected_sha256: String,
) -> Result<PluginView, String> {
    let parsed = validate_plugin_url(&source_url)?;
    validate_sha256_text(expected_sha256.trim())?;
    let downloads = state.app_data.join("plugin-downloads");
    tokio::fs::create_dir_all(&downloads)
        .await
        .map_err(|error| format!("create plugin download directory: {error}"))?;
    let download = downloads.join(format!("{}.zip", uuid::Uuid::new_v4()));
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() == "https" && attempt.previous().len() < 6 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|error| format!("build plugin downloader: {error}"))?;
    let response = client
        .get(parsed.clone())
        .send()
        .await
        .map_err(|error| format!("download plugin: {error}"))?
        .error_for_status()
        .map_err(|error| format!("download plugin: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
    {
        return Err("plugin download exceeds the archive size limit".into());
    }
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&download)
        .await
        .map_err(|error| format!("create plugin download: {error}"))?;
    let download_result = async {
        let mut received = 0u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| format!("read plugin download: {error}"))?;
            received = received.saturating_add(chunk.len() as u64);
            if received > MAX_ARCHIVE_BYTES {
                return Err("plugin download exceeds the archive size limit".to_string());
            }
            file.write_all(&chunk)
                .await
                .map_err(|error| format!("write plugin download: {error}"))?;
        }
        file.flush()
            .await
            .map_err(|error| format!("flush plugin download: {error}"))?;
        Ok::<_, String>(())
    }
    .await;
    if let Err(error) = download_result {
        drop(file);
        let _ = tokio::fs::remove_file(&download).await;
        return Err(error);
    }
    drop(file);
    let result = install_plugin_path(
        &state,
        download.clone(),
        Some(expected_sha256),
        parsed.to_string(),
    )
    .await;
    let _ = tokio::fs::remove_file(download).await;
    result
}

#[tauri::command]
pub(super) async fn set_plugin_enabled(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    plugin_id: String,
    version: String,
    enabled: bool,
) -> Result<(), String> {
    let project = state.active(window.label());
    let installation = state
        .store
        .get_plugin_installation(&plugin_id, &version)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("plugin '{plugin_id} {version}' is not installed"))?;
    NormalizedPluginManifest::from_installation(&installation)?;
    if enabled {
        state
            .store
            .set_project_plugin(
                &project.id,
                &plugin_id,
                &version,
                true,
                r#"{"tools":"ask"}"#,
            )
            .await
            .map_err(|error| error.to_string())?;
    } else {
        let binding = state
            .store
            .list_project_plugins(&project.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|binding| binding.plugin_id == plugin_id && binding.enabled)
            .ok_or_else(|| format!("plugin '{plugin_id}' is not enabled for this project"))?;
        if binding.version != version {
            return Err(format!(
                "plugin '{plugin_id} {}' is enabled instead of version '{version}'",
                binding.version
            ));
        }
        state
            .store
            .set_project_plugin_enabled(&project.id, &plugin_id, false)
            .await
            .map_err(|error| error.to_string())?;
    }
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
pub(super) async fn remove_plugin(
    state: State<'_, AppState>,
    plugin_id: String,
    version: String,
) -> Result<(), String> {
    validate_plugin_id(&plugin_id)?;
    let installation = state
        .store
        .get_plugin_installation(&plugin_id, &version)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "plugin installation not found".to_string())?;
    let expected_root = state
        .app_data
        .join("plugins")
        .join(&plugin_id)
        .join(&version);
    let installed_root = PathBuf::from(&installation.install_root);
    if installed_root != expected_root
        || !installed_root.starts_with(state.app_data.join("plugins"))
    {
        return Err("refusing to remove plugin outside the managed plugin directory".into());
    }
    clear_idle_agents(&state).await;
    let quarantine = state
        .app_data
        .join("plugin-staging")
        .join(format!("remove-{}", uuid::Uuid::new_v4()));
    if installed_root.exists() {
        if let Some(parent) = quarantine.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| format!("prepare plugin removal: {error}"))?;
        }
        tokio::fs::rename(&installed_root, &quarantine)
            .await
            .map_err(|error| format!("quarantine plugin files: {error}"))?;
    }
    if let Err(error) = state
        .store
        .delete_plugin_installation(&plugin_id, &version)
        .await
    {
        if quarantine.exists() {
            let _ = tokio::fs::rename(&quarantine, &installed_root).await;
        }
        return Err(error.to_string());
    }
    if quarantine.exists() {
        tokio::fs::remove_dir_all(&quarantine)
            .await
            .map_err(|error| format!("remove plugin files: {error}"))?;
    }
    if let Some(parent) = installed_root.parent() {
        let _ = tokio::fs::remove_dir(parent).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use wisp_tools::{Tool, ToolEnv, ToolEvent};

    struct AcceptanceEnv {
        root: PathBuf,
        events: Mutex<Vec<ToolEvent>>,
    }

    #[async_trait::async_trait]
    impl ToolEnv for AcceptanceEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn emit(&self, event: ToolEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn fixture(root: &Path) {
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::create_dir_all(root.join("skills/motif/scripts")).unwrap();
        std::fs::create_dir_all(root.join("server")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"motif","displayName":"Motif","version":"0.2.1","description":"Workbench","author":{"name":"Test"},"license":"MIT"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join(".mcp.json"),
            r#"{"motif":{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/server/server.mjs"]}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("skills/motif/SKILL.md"),
            "---\nname: motif\ndescription: Test\n---\n# Motif",
        )
        .unwrap();
        std::fs::write(root.join("server/server.mjs"), "// fixture").unwrap();
    }

    #[test]
    fn claude_plugin_is_normalized_without_executing_it() {
        let root = std::env::temp_dir().join(format!("wisp-plugin-{}", uuid::Uuid::new_v4()));
        fixture(&root);
        let manifest = parse_manifest(&root).unwrap();
        assert_eq!(manifest.id, "motif");
        assert_eq!(manifest.skills, vec!["skills/motif"]);
        assert_eq!(manifest.mcp_servers[0].command, "node");
        assert_eq!(manifest.source_format, "claude-plugin");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn zip_extraction_rejects_parent_traversal() {
        let root = std::env::temp_dir().join(format!("wisp-plugin-zip-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let archive_path = root.join("bad.zip");
        let file = File::create(&archive_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("../escape", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"bad").unwrap();
        archive.finish().unwrap();
        let output = root.join("out");
        std::fs::create_dir_all(&output).unwrap();
        assert!(extract_zip(&archive_path, &output).is_err());
        assert!(!root.join("escape").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn checksum_mismatch_is_rejected() {
        assert!(validate_expected_sha256(Some(&"0".repeat(64)), &"a".repeat(64)).is_err());
        assert_eq!(
            validate_expected_sha256(Some(&"A".repeat(64)), &"a".repeat(64)).unwrap(),
            "checksum_verified"
        );
    }

    #[test]
    fn remote_install_requires_https_and_checksum_shape() {
        assert!(validate_plugin_url("http://example.test/plugin.zip").is_err());
        assert_eq!(
            validate_plugin_url("https://example.test/plugin.zip")
                .unwrap()
                .scheme(),
            "https"
        );
        assert!(validate_sha256_text("abc").is_err());
        assert!(validate_sha256_text(&"a".repeat(64)).is_ok());
    }

    /// Opt-in real-package acceptance. Normal CI skips when the two variables
    /// are absent; release verification points them at Motif's built ZIP and
    /// published archive checksum.
    #[tokio::test]
    async fn motif_release_bundle_acceptance() {
        let Ok(zip_path) = std::env::var("WISP_MOTIF_PLUGIN_ZIP") else {
            return;
        };
        let expected = std::env::var("WISP_MOTIF_PLUGIN_SHA256")
            .expect("WISP_MOTIF_PLUGIN_SHA256 accompanies WISP_MOTIF_PLUGIN_ZIP");
        let root = std::env::temp_dir().join(format!("wisp-motif-e2e-{}", uuid::Uuid::new_v4()));
        let app_data = root.join("app-data");
        let project_root = root.join("project");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::create_dir_all(&project_root).unwrap();

        let prepared = prepare_plugin(
            Path::new(&zip_path),
            Some(&expected),
            &app_data,
            zip_path.clone(),
        )
        .unwrap();
        assert_eq!(prepared.manifest.id, "motif-for-claude-science");
        assert_eq!(prepared.trust_state, "checksum_verified");
        let (_, installation) = install_prepared(&prepared, &app_data).unwrap();

        let store = wisp_store::Store::open(&app_data.join("wisp.sqlite"))
            .await
            .unwrap();
        store
            .create_project("motif-project", "Motif", &project_root.to_string_lossy())
            .await
            .unwrap();
        store
            .upsert_plugin_installation(&installation)
            .await
            .unwrap();
        store
            .set_project_plugin(
                "motif-project",
                &installation.plugin_id,
                &installation.version,
                true,
                r#"{"tools":"ask"}"#,
            )
            .await
            .unwrap();
        let (launches, errors) = enabled_plugin_mcp_launches(&store, "motif-project").await;
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(launches.len(), 1);

        let client = Arc::new(crate::connect_plugin_mcp(&launches[0]).await.unwrap());
        let tools = client.tools_list().await.unwrap();
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["motif_open_workbench", "motif_create_workbench_artifact"]
        );
        let open = tools
            .iter()
            .find(|tool| tool.name == "motif_open_workbench")
            .unwrap()
            .clone();
        assert_eq!(open.ui_resource_uri(), Some("ui://motif/workbench.html"));
        let env = AcceptanceEnv {
            root: project_root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let open_result = wisp_mcp::McpTool::new(open, client.clone())
            .run(&serde_json::json!({}), &env)
            .await;
        assert!(open_result.success, "{}", open_result.content);
        assert!(env.events.lock().unwrap().iter().any(|event| matches!(
            event,
            ToolEvent::Presentation { kind, payload }
                if kind == "mcp_app"
                    && payload.pointer("/resource/uri").and_then(serde_json::Value::as_str)
                        == Some("ui://motif/workbench.html")
        )));

        let artifact = tools
            .into_iter()
            .find(|tool| tool.name == "motif_create_workbench_artifact")
            .unwrap();
        let artifact_result = wisp_mcp::McpTool::new(artifact, client.clone())
            .run(
                &serde_json::json!({
                    "content": ">MOTIFDEMO\nACGTACGTACGT",
                    "filename": "motif-demo.fasta",
                    "title": "Wisp Motif acceptance",
                    "outputFilename": "motif-demo-workbench.html"
                }),
                &env,
            )
            .await;
        assert!(artifact_result.success, "{}", artifact_result.content);
        let artifact_path = env
            .events
            .lock()
            .unwrap()
            .iter()
            .find_map(|event| match event {
                ToolEvent::FileChanged { path } => Some(path.clone()),
                _ => None,
            })
            .expect("Motif fallback materialized an HTML artifact");
        let html = std::fs::read_to_string(project_root.join(artifact_path)).unwrap();
        assert!(html.contains("Motif"));

        drop(client);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
