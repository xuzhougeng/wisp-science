//! Plugins: self-contained packages of skills (and MCP servers) installed
//! into this host's data directory and switched on per project.
//!
//! A package is a folder in the Claude plugin layout
//! (`.claude-plugin/plugin.json`, `skills/*/SKILL.md`, `.mcp.json`) or
//! with a `.wisp-plugin/plugin.json` of its own. Installing copies the
//! folder and records what it declares; nothing in it is run. The checks
//! and the recorded manifest follow `src-tauri/src/plugins.rs`, which
//! cannot be shared from here because it lives in the Tauri shell, and
//! the records are the store's own.
//!
//! A project that switches a plugin on gets its skills, and the MCP
//! servers it declares are started for that project's turns, as the WebView
//! starts them: in the plugin's folder, with a cleared environment, every
//! tool asking before it runs.

use std::{
    collections::{BTreeMap, HashSet},
    io::Read,
    path::{Component, Path, PathBuf},
};

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wisp_store::{PluginInstallation, Store};

const SCHEMA: &str = "wisp.plugin.v1";
const MAX_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 10_000;
const MAX_PATH_BYTES: usize = 768;
/// The largest archive a plugin comes in.
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;

/// Where a plugin came from, as it is recorded.
struct Origin {
    source: String,
    sha256: String,
    /// `unverified`, or `checksum_verified` when a published checksum
    /// matched.
    trust: &'static str,
}

/// Whether `expected` (when given) is the archive's SHA-256: the trust a
/// plugin is recorded with.
fn checked(expected: Option<&str>, actual: &str) -> Result<&'static str> {
    let Some(expected) = expected.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok("unverified");
    };
    ensure!(
        expected.len() == 64 && expected.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "A SHA-256 is 64 hexadecimal characters"
    );
    ensure!(
        expected.eq_ignore_ascii_case(actual),
        "The plugin's SHA-256 is {actual}, not {}",
        expected.to_ascii_lowercase()
    );
    Ok("checksum_verified")
}

/// Unpacks a plugin archive into `destination`, refusing what could reach
/// outside it or be too large, as the WebView's installer does.
fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    let size = std::fs::metadata(archive)
        .context("Choose a plugin archive")?
        .len();
    ensure!(
        size <= MAX_ARCHIVE_BYTES,
        "A plugin archive is at most {} MB",
        MAX_ARCHIVE_BYTES / 1024 / 1024
    );
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)
        .context("The plugin archive is not a ZIP file")?;
    ensure!(
        zip.len() <= MAX_FILES,
        "The plugin archive has more than {MAX_FILES} entries"
    );
    let mut seen = HashSet::new();
    let mut expanded = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let enclosed = entry.enclosed_name().with_context(|| {
            format!("The archive's entry '{}' leaves the package", entry.name())
        })?;
        let normalized = enclosed.to_string_lossy().replace('\\', "/");
        ensure!(
            !normalized.is_empty() && normalized.len() <= MAX_PATH_BYTES,
            "The archive has an empty or overly long path"
        );
        ensure!(
            seen.insert(normalized.clone()),
            "The archive has '{normalized}' twice"
        );
        ensure!(
            !entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000),
            "The archive has a symbolic link: {normalized}"
        );
        ensure!(
            entry.size() <= MAX_FILE_BYTES,
            "The archive's '{normalized}' is too large"
        );
        expanded = expanded.saturating_add(entry.size());
        ensure!(
            expanded <= MAX_EXPANDED_BYTES,
            "The archive is too large once unpacked"
        );
        let output = destination.join(&enclosed);
        if entry.is_dir() {
            std::fs::create_dir_all(&output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut target = std::fs::File::create(&output)?;
        std::io::copy(&mut entry, &mut target)?;
        // A program the plugin starts keeps being one.
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&output, std::fs::Permissions::from_mode(mode & 0o755))?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct McpServer {
    id: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    cwd: Option<String>,
}

/// What a package declares, as it is recorded: the same shape the WebView
/// desktop records.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Manifest {
    schema: String,
    id: String,
    display_name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    license: String,
    source_format: String,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    mcp_servers: Vec<McpServer>,
}

#[derive(Deserialize)]
struct ClaudeManifest {
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

#[derive(Deserialize)]
struct RawMcpServer {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    cwd: Option<String>,
}

/// An installed plugin as Settings lists it for one project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRow {
    pub id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub license: String,
    /// The folder it was installed from.
    pub source: String,
    /// Switched on for the project asked about.
    pub enabled: bool,
    /// The skills it brings, by the names they are used under.
    pub skills: Vec<String>,
    /// The MCP servers it declares, as the commands that would start them.
    pub servers: Vec<String>,
    /// What is wrong with it as installed.
    pub problems: Vec<String>,
}

fn valid_id(id: &str) -> Result<()> {
    let valid = !id.is_empty()
        && id.len() <= 96
        && id.bytes().enumerate().all(|(index, byte)| match byte {
            b'a'..=b'z' | b'0'..=b'9' => true,
            b'-' => index > 0 && index + 1 < id.len(),
            _ => false,
        })
        && !id.contains("--");
    ensure!(
        valid,
        "plugin id must be lowercase kebab-case and at most 96 characters"
    );
    Ok(())
}

fn inside(value: &str, label: &str) -> Result<PathBuf> {
    ensure!(
        !value.is_empty() && value.len() <= MAX_PATH_BYTES,
        "{label} is empty or too long"
    );
    let path = Path::new(value);
    ensure!(
        !path.is_absolute()
            && !path.components().any(|component| matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )),
        "{label} must stay inside the plugin directory"
    );
    Ok(path.to_path_buf())
}

/// The folder that holds the manifest: the one given, or the only folder
/// in it that does.
fn package_root(root: &Path) -> Result<PathBuf> {
    let declares = |folder: &Path| {
        folder.join(".wisp-plugin/plugin.json").is_file()
            || folder.join(".claude-plugin/plugin.json").is_file()
    };
    if declares(root) {
        return Ok(root.to_path_buf());
    }
    let mut candidates = vec![];
    for entry in std::fs::read_dir(root).context("read plugin package")? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && declares(&entry.path()) {
            candidates.push(entry.path());
        }
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => bail!("plugin package has no .wisp-plugin/plugin.json or .claude-plugin/plugin.json"),
        _ => bail!("plugin package contains more than one plugin root"),
    }
}

fn parse(root: &Path) -> Result<Manifest> {
    let native = root.join(".wisp-plugin/plugin.json");
    if native.is_file() {
        let manifest = serde_json::from_slice(&std::fs::read(&native)?)
            .map_err(|error| anyhow!("parse Wisp plugin manifest: {error}"))?;
        return validate(root, manifest);
    }
    let raw: ClaudeManifest =
        serde_json::from_slice(&std::fs::read(root.join(".claude-plugin/plugin.json"))?)
            .map_err(|error| anyhow!("parse Claude plugin manifest: {error}"))?;
    let mut skills = vec![];
    let skills_root = root.join("skills");
    if skills_root.is_dir() {
        for entry in std::fs::read_dir(&skills_root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() && entry.path().join("SKILL.md").is_file() {
                skills.push(format!("skills/{}", entry.file_name().to_string_lossy()));
            }
        }
    }
    skills.sort();
    let mut mcp_servers = vec![];
    let mcp = root.join(".mcp.json");
    if mcp.is_file() {
        let servers: BTreeMap<String, RawMcpServer> = serde_json::from_slice(&std::fs::read(&mcp)?)
            .map_err(|error| anyhow!("parse .mcp.json: {error}"))?;
        for (id, server) in servers {
            ensure!(
                !id.trim().is_empty() && !server.command.trim().is_empty(),
                "MCP server id and command must be non-empty"
            );
            mcp_servers.push(McpServer {
                id,
                command: server.command,
                args: server.args,
                env: server.env,
                cwd: server.cwd,
            });
        }
    }
    let author = raw
        .author
        .as_str()
        .or_else(|| raw.author.get("name").and_then(|name| name.as_str()))
        .unwrap_or_default()
        .to_owned();
    validate(
        root,
        Manifest {
            schema: SCHEMA.into(),
            id: raw.name.clone(),
            display_name: raw.display_name.unwrap_or(raw.name),
            version: raw.version,
            description: raw.description,
            author,
            license: raw.license,
            source_format: "claude-plugin".into(),
            skills,
            mcp_servers,
        },
    )
}

fn validate(root: &Path, mut manifest: Manifest) -> Result<Manifest> {
    ensure!(
        manifest.schema == SCHEMA,
        "unsupported plugin schema '{}'",
        manifest.schema
    );
    valid_id(&manifest.id)?;
    semver::Version::parse(&manifest.version)
        .map_err(|error| anyhow!("plugin version must be semantic versioning: {error}"))?;
    manifest.display_name = manifest.display_name.trim().to_owned();
    ensure!(
        !manifest.display_name.is_empty() && manifest.display_name.chars().count() <= 120,
        "plugin display name is empty or too long"
    );
    ensure!(
        manifest.description.chars().count() <= 2_000,
        "plugin description is too long"
    );
    for skill in &manifest.skills {
        ensure!(
            root.join(inside(skill, "plugin skill path")?)
                .join("SKILL.md")
                .is_file(),
            "plugin skill '{skill}' has no SKILL.md"
        );
    }
    let mut ids = HashSet::new();
    for server in &manifest.mcp_servers {
        ensure!(
            ids.insert(server.id.clone()),
            "duplicate MCP server id '{}'",
            server.id
        );
        ensure!(
            !server.command.trim().is_empty() && !server.command.contains(['\0', '\n', '\r']),
            "invalid MCP command for server '{}'",
            server.id
        );
        ensure!(
            !server
                .args
                .iter()
                .any(|argument| argument.contains(['\0', '\n', '\r'])),
            "invalid MCP argument for server '{}'",
            server.id
        );
        if let Some(cwd) = &server.cwd {
            inside(cwd, "plugin MCP cwd")?;
        }
    }
    Ok(manifest)
}

/// The regular files under `root`, sorted by their path inside it. A
/// symbolic link anywhere refuses the package.
fn files(root: &Path) -> Result<Vec<(String, PathBuf, u64)>> {
    let mut found = vec![];
    let mut expanded = 0u64;
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.context("walk plugin directory")?;
        ensure!(
            !entry.file_type().is_symlink(),
            "plugin directory contains a symbolic link: {}",
            entry.path().display()
        );
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        ensure!(relative.len() <= MAX_PATH_BYTES, "plugin path is too long");
        let size = entry.metadata()?.len();
        ensure!(
            size <= MAX_FILE_BYTES,
            "plugin file '{relative}' is too large"
        );
        expanded = expanded.saturating_add(size);
        ensure!(
            expanded <= MAX_EXPANDED_BYTES,
            "plugin expanded size exceeds safety limit"
        );
        found.push((relative, entry.into_path(), size));
        ensure!(
            found.len() <= MAX_FILES,
            "plugin contains more than {MAX_FILES} files"
        );
    }
    found.sort();
    Ok(found)
}

/// One hash over every file's path, size and content, in path order: the
/// same a folder gets in the WebView desktop.
fn tree_sha256(files: &[(String, PathBuf, u64)]) -> Result<String> {
    let mut hash = Sha256::new();
    for (relative, path, size) in files {
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        hash.update(size.to_le_bytes());
        let mut file = std::fs::File::open(path)?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
    }
    Ok(hex::encode(hash.finalize()))
}

/// Checks the folder and copies it to `data/plugins/<id>/<version>`,
/// replacing whatever version of the plugin was there. Runs nothing.
fn install_files(source: &Path, data: &Path, origin: Option<Origin>) -> Result<PluginInstallation> {
    ensure!(source.is_dir(), "Choose the folder of a plugin");
    let listed = files(source)?;
    let origin = match origin {
        Some(origin) => origin,
        None => Origin {
            source: source.to_string_lossy().into_owned(),
            sha256: tree_sha256(&listed)?,
            trust: "unverified",
        },
    };
    let root = package_root(source)?;
    let manifest = parse(&root)?;
    let plugin_root = data.join("plugins").join(&manifest.id);
    let install_root = plugin_root.join(&manifest.version);
    // Copied beside its place first, so a failed copy leaves the
    // installed version as it was.
    let staging = data
        .join("plugin-staging")
        .join(uuid::Uuid::new_v4().to_string());
    let copied = (|| -> Result<()> {
        for (_, path, _) in &listed {
            let Ok(relative) = path.strip_prefix(&root) else {
                // Beside the package root, not part of the plugin.
                continue;
            };
            let target = staging.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(path, &target)?;
        }
        if plugin_root.exists() {
            std::fs::remove_dir_all(&plugin_root)?;
        }
        std::fs::create_dir_all(&plugin_root)?;
        std::fs::rename(&staging, &install_root)?;
        Ok(())
    })();
    if let Err(error) = copied {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error.context("install plugin files"));
    }
    let now = chrono::Utc::now().timestamp();
    Ok(PluginInstallation {
        plugin_id: manifest.id.clone(),
        version: manifest.version.clone(),
        display_name: manifest.display_name.clone(),
        description: manifest.description.clone(),
        author: manifest.author.clone(),
        license: manifest.license.clone(),
        source_uri: origin.source,
        install_root: install_root.to_string_lossy().into_owned(),
        archive_sha256: origin.sha256,
        manifest_json: serde_json::to_string(&manifest)?,
        trust_state: origin.trust.into(),
        installed_at: now,
        updated_at: now,
    })
}

fn manifest_of(installation: &PluginInstallation) -> Result<Manifest> {
    serde_json::from_str(&installation.manifest_json)
        .map_err(|error| anyhow!("invalid installed plugin manifest: {error}"))
}

/// The folders of an installed plugin's skills that are still there.
fn skill_folders(installation: &PluginInstallation) -> Vec<PathBuf> {
    let Ok(manifest) = manifest_of(installation) else {
        return vec![];
    };
    manifest
        .skills
        .iter()
        .filter_map(|skill| inside(skill, "plugin skill path").ok())
        .map(|skill| Path::new(&installation.install_root).join(skill))
        .filter(|folder| folder.is_dir())
        .collect()
}

fn row(installation: PluginInstallation, enabled: bool) -> PluginRow {
    let mut problems = vec![];
    let mut skills = vec![];
    let mut servers = vec![];
    match manifest_of(&installation) {
        Ok(manifest) => {
            for skill in &manifest.skills {
                let file = Path::new(&installation.install_root)
                    .join(skill)
                    .join("SKILL.md");
                match wisp_skills::parse_skill_file(&file) {
                    Ok(skill) => skills.push(skill.name),
                    Err(error) => problems.push(format!("{skill}: {error}")),
                }
            }
            // A server that could not be started says why.
            for server in &manifest.mcp_servers {
                if let Err(error) = launch(&installation, &manifest, server) {
                    problems.push(format!("MCP '{}': {error:#}", server.id));
                }
            }
            servers = manifest
                .mcp_servers
                .iter()
                .map(|server| {
                    std::iter::once(server.command.as_str())
                        .chain(server.args.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect();
        }
        Err(error) => problems.push(error.to_string()),
    }
    PluginRow {
        id: installation.plugin_id,
        version: installation.version,
        name: installation.display_name,
        description: installation.description,
        author: installation.author,
        license: installation.license,
        source: installation.source_uri,
        enabled,
        skills,
        servers,
        problems,
    }
}

/// How one of a plugin's MCP servers is started: a program in the plugin's
/// folder or on the PATH, with `${CLAUDE_PLUGIN_ROOT}` (or
/// `${WISP_PLUGIN_ROOT}`) in what it is given standing for that folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServerLaunch {
    /// `plugin:<plugin>:<server>`, as the WebView names it.
    pub id: String,
    /// What its tools are named after.
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub root: PathBuf,
}

fn expand_root(value: &str, root: &Path) -> Result<String> {
    let root = root.to_string_lossy();
    let expanded = value
        .replace("${CLAUDE_PLUGIN_ROOT}", &root)
        .replace("${WISP_PLUGIN_ROOT}", &root);
    ensure!(
        !expanded.contains("${") && !expanded.contains('\0'),
        "unsupported variable in plugin value '{value}'"
    );
    Ok(expanded)
}

fn launch(
    installation: &PluginInstallation,
    manifest: &Manifest,
    server: &McpServer,
) -> Result<ServerLaunch> {
    let root = dunce::canonicalize(&installation.install_root)
        .context("resolve plugin install directory")?;
    let command = expand_root(&server.command, &root)?;
    let command = if command.contains(['/', '\\']) {
        let path = dunce::canonicalize(&command)
            .with_context(|| format!("resolve plugin MCP command '{command}'"))?;
        ensure!(
            path.starts_with(&root) && path.is_file(),
            "plugin MCP command path must be a file inside the plugin directory"
        );
        path
    } else {
        which::which(&command)
            .map_err(|_| anyhow!("plugin MCP executable '{command}' was not found in PATH"))?
    };
    let args = server
        .args
        .iter()
        .map(|argument| expand_root(argument, &root))
        .collect::<Result<Vec<_>>>()?;
    let mut env = BTreeMap::new();
    for (key, value) in &server.env {
        ensure!(
            crate::mcp::valid_env_name(key),
            "plugin MCP environment key '{key}' is invalid"
        );
        env.insert(key.clone(), expand_root(value, &root)?);
    }
    let cwd = match &server.cwd {
        Some(relative) => {
            let relative = inside(relative, "plugin MCP cwd")?;
            let path = dunce::canonicalize(root.join(relative))
                .with_context(|| format!("resolve plugin MCP working directory '{}'", server.id))?;
            ensure!(
                path.starts_with(&root) && path.is_dir(),
                "plugin MCP working directory must stay inside the plugin directory"
            );
            path
        }
        None => root.clone(),
    };
    Ok(ServerLaunch {
        id: format!("plugin:{}:{}", manifest.id, server.id),
        name: format!("{} {}", manifest.id, server.id),
        command,
        args,
        env,
        cwd,
        root,
    })
}

/// How to start the MCP servers of the plugins `project` has switched on,
/// and why any of them cannot be, by plugin.
pub(crate) async fn enabled_launches(
    store: &Store,
    project: &str,
) -> (Vec<ServerLaunch>, Vec<String>) {
    let mut launches = vec![];
    let mut problems = vec![];
    for installation in store
        .list_enabled_plugin_installations(project)
        .await
        .unwrap_or_default()
    {
        let Ok(manifest) = manifest_of(&installation) else {
            continue;
        };
        for server in &manifest.mcp_servers {
            match launch(&installation, &manifest, server) {
                Ok(found) => launches.push(found),
                Err(error) => problems.push(format!(
                    "Plugin '{}' MCP '{}': {error:#}",
                    manifest.display_name, server.id
                )),
            }
        }
    }
    (launches, problems)
}

/// The skill folders of the plugins `project` has switched on.
pub(crate) async fn enabled_skill_folders(store: &Store, project: &str) -> Vec<PathBuf> {
    store
        .list_enabled_plugin_installations(project)
        .await
        .unwrap_or_default()
        .iter()
        .flat_map(skill_folders)
        .collect()
}

pub(crate) async fn list(store: &Store, project: Option<&str>) -> Result<Vec<PluginRow>> {
    let bindings = match project {
        Some(project) => store.list_project_plugins(project).await?,
        None => vec![],
    };
    let enabled: HashSet<(String, String)> = bindings
        .into_iter()
        .filter(|binding| binding.enabled)
        .map(|binding| (binding.plugin_id, binding.version))
        .collect();
    let mut rows: Vec<_> = store
        .list_plugin_installations()
        .await?
        .into_iter()
        .map(|installation| {
            let on =
                enabled.contains(&(installation.plugin_id.clone(), installation.version.clone()));
            row(installation, on)
        })
        .collect();
    rows.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    Ok(rows)
}

/// Installs the plugin in `folder`; a version already installed is
/// replaced. A project that had it on keeps it on only at the version it
/// chose, so a new version has to be switched on again.
pub(crate) async fn install(store: &Store, data: &Path, folder: &Path) -> Result<PluginRow> {
    let source = folder.to_string_lossy().into_owned();
    install_checked(store, data, folder, None, source).await
}

/// Installs from a folder, or from a `.zip` checked against `expected`
/// (its SHA-256, when given) before anything in it is read. `source` is
/// what the record says it came from.
async fn install_checked(
    store: &Store,
    data: &Path,
    path: &Path,
    expected: Option<String>,
    source: String,
) -> Result<PluginRow> {
    let (path, data) = (path.to_path_buf(), data.to_path_buf());
    let installation = tokio::task::spawn_blocking(move || -> Result<PluginInstallation> {
        if path.is_dir() {
            ensure!(expected.is_none(), "A checksum is for an archive");
            return install_files(&path, &data, None);
        }
        ensure!(
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("zip")),
            "Choose a plugin folder or a .zip archive"
        );
        let sha256 = hex::encode(Sha256::digest(std::fs::read(&path)?));
        let trust = checked(expected.as_deref(), &sha256)?;
        let unpacked = data
            .join("plugin-downloads")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&unpacked)?;
        let installed = extract_zip(&path, &unpacked).and_then(|()| {
            install_files(
                &unpacked,
                &data,
                Some(Origin {
                    source,
                    sha256,
                    trust,
                }),
            )
        });
        let _ = std::fs::remove_dir_all(&unpacked);
        installed
    })
    .await??;
    // One version of a plugin is kept: its files were replaced above.
    for other in store.list_plugin_installations().await? {
        if other.plugin_id == installation.plugin_id && other.version != installation.version {
            store
                .delete_plugin_installation(&other.plugin_id, &other.version)
                .await?;
        }
    }
    store.replace_plugin_installation(&installation).await?;
    Ok(row(installation, false))
}

/// Downloads a plugin archive over HTTPS and installs it once its SHA-256
/// is the one given, as the WebView does for an address.
pub(crate) async fn install_from_address(
    store: &Store,
    data: &Path,
    address: &str,
    expected: &str,
    client: reqwest::Client,
) -> Result<PluginRow> {
    let url = url::Url::parse(address.trim()).context("Enter the plugin's address")?;
    ensure!(
        url.scheme() == "https" && url.host_str().is_some(),
        "A plugin's address uses HTTPS"
    );
    // The checksum's form is looked at before anything is downloaded.
    let expected = expected.trim();
    ensure!(
        expected.len() == 64 && expected.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "A SHA-256 is 64 hexadecimal characters"
    );
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .context("Could not download the plugin")?
        .error_for_status()
        .context("Could not download the plugin")?;
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_ARCHIVE_BYTES),
        "The plugin archive is too large"
    );
    let folder = data.join("plugin-downloads");
    tokio::fs::create_dir_all(&folder).await?;
    let archive = folder.join(format!("{}.zip", uuid::Uuid::new_v4()));
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Could not download the plugin")?
    {
        bytes.extend_from_slice(&chunk);
        ensure!(
            bytes.len() as u64 <= MAX_ARCHIVE_BYTES,
            "The plugin archive is too large"
        );
    }
    tokio::fs::write(&archive, bytes).await?;
    let installed = install_checked(
        store,
        data,
        &archive,
        Some(expected.to_owned()),
        url.to_string(),
    )
    .await;
    let _ = tokio::fs::remove_file(&archive).await;
    installed
}

pub(crate) async fn set_enabled(
    store: &Store,
    project: &str,
    id: &str,
    version: &str,
    enabled: bool,
) -> Result<()> {
    let installation = store
        .get_plugin_installation(id, version)
        .await?
        .with_context(|| format!("plugin '{id} {version}' is not installed"))?;
    manifest_of(&installation)?;
    if enabled {
        // Tools a plugin brings are asked about, as the WebView records.
        store
            .set_project_plugin(project, id, version, true, r#"{"tools":"ask"}"#)
            .await
    } else {
        store.set_project_plugin_enabled(project, id, false).await?;
        Ok(())
    }
}

/// Removes an installed plugin and its files. Only files under this
/// host's own plugin directory are ever removed.
pub(crate) async fn remove(store: &Store, data: &Path, id: &str, version: &str) -> Result<()> {
    valid_id(id)?;
    let installation = store
        .get_plugin_installation(id, version)
        .await?
        .context("plugin installation not found")?;
    let expected = data.join("plugins").join(id).join(version);
    let installed = PathBuf::from(&installation.install_root);
    store.delete_plugin_installation(id, version).await?;
    // A plugin another desktop installed keeps its files where that
    // desktop put them; only the record goes.
    if installed == expected && installed.exists() {
        tokio::fs::remove_dir_all(&installed).await?;
        if let Some(parent) = installed.parent() {
            let _ = tokio::fs::remove_dir(parent).await;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn write_plugin(folder: &Path, name: &str, version: &str) {
        std::fs::create_dir_all(folder.join(".claude-plugin")).unwrap();
        std::fs::write(
            folder.join(".claude-plugin/plugin.json"),
            format!(
                r#"{{"name":"{name}","displayName":"Motif tools","version":"{version}",
                    "description":"Find motifs","author":{{"name":"Lab"}},"license":"MIT"}}"#
            ),
        )
        .unwrap();
        let skill = folder.join("skills/find-motifs");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: find-motifs\ndescription: Find sequence motifs in a FASTA file.\n---\n\nRun the scan.\n",
        )
        .unwrap();
        std::fs::write(
            folder.join(".mcp.json"),
            r#"{"motif":{"command":"uvx","args":["motif-mcp","--stdio"]}}"#,
        )
        .unwrap();
    }

    #[test]
    fn a_package_is_read_from_the_claude_layout_and_checked() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("motif");
        write_plugin(&folder, "motif-tools", "1.2.0");
        let manifest = parse(&package_root(&folder).unwrap()).unwrap();
        assert_eq!(manifest.id, "motif-tools");
        assert_eq!(manifest.display_name, "Motif tools");
        assert_eq!(manifest.author, "Lab");
        assert_eq!(manifest.skills, ["skills/find-motifs"]);
        assert_eq!(manifest.mcp_servers[0].command, "uvx");
        // A folder that only holds the package is as good as the package.
        assert_eq!(package_root(temp.path()).unwrap(), folder);

        for (name, version) in [
            ("Motif Tools", "1.0.0"),
            ("motif--tools", "1.0.0"),
            ("motif", "one"),
        ] {
            write_plugin(&folder, name, version);
            assert!(parse(&folder).is_err(), "{name} {version}");
        }
        write_plugin(&folder, "motif-tools", "1.2.0");
        std::fs::write(
            folder.join(".mcp.json"),
            r#"{"motif":{"command":"uvx","cwd":"../outside"}}"#,
        )
        .unwrap();
        assert!(parse(&folder).is_err());
        let empty = temp.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(package_root(&empty).is_err());
    }

    #[test]
    fn installing_copies_the_package_and_replaces_the_version_before() {
        let temp = tempfile::tempdir().unwrap();
        let (folder, data) = (temp.path().join("motif"), temp.path().join("data"));
        write_plugin(&folder, "motif-tools", "1.2.0");
        let first = install_files(&folder, &data, None).unwrap();
        let root = data.join("plugins/motif-tools/1.2.0");
        assert_eq!(Path::new(&first.install_root), root);
        assert!(root.join("skills/find-motifs/SKILL.md").is_file());
        assert_eq!(first.trust_state, "unverified");
        assert_eq!(first.archive_sha256.len(), 64);
        assert_eq!(skill_folders(&first), [root.join("skills/find-motifs")]);
        // The same files give the same hash; a changed file another.
        assert_eq!(
            install_files(&folder, &data, None).unwrap().archive_sha256,
            first.archive_sha256
        );

        write_plugin(&folder, "motif-tools", "1.3.0");
        let second = install_files(&folder, &data, None).unwrap();
        assert_ne!(second.archive_sha256, first.archive_sha256);
        assert!(!root.exists());
        assert!(data.join("plugins/motif-tools/1.3.0/.mcp.json").is_file());
        assert!(!data
            .join("plugin-staging")
            .read_dir()
            .unwrap()
            .any(|_| true));

        let shown = row(second, true);
        assert_eq!(shown.skills, ["find-motifs"]);
        assert_eq!(shown.servers, ["uvx motif-mcp --stdio"]);
        assert!(shown.enabled && shown.problems.is_empty());

        // A package that is refused leaves what is installed alone.
        std::fs::remove_file(folder.join(".claude-plugin/plugin.json")).unwrap();
        assert!(install_files(&folder, &data, None).is_err());
        assert!(data.join("plugins/motif-tools/1.3.0").is_dir());
    }

    #[cfg(unix)]
    /// A zip of `folder`, with its files under `prefix`, and any extra
    /// entries as `(name, mode)`.
    fn zip_of(folder: &Path, prefix: &str, extra: &[(&str, u32)]) -> Vec<u8> {
        use std::io::Write;
        let mut bytes = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut bytes);
        for entry in walkdir_files(folder) {
            let name = format!(
                "{prefix}{}",
                entry
                    .strip_prefix(folder)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&std::fs::read(&entry).unwrap()).unwrap();
        }
        for (name, mode) in extra {
            // A mode of a link is written as a link.
            if mode & 0o170000 == 0o120000 {
                zip.add_symlink(
                    *name,
                    "/etc/passwd",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
                continue;
            }
            zip.start_file(
                *name,
                zip::write::SimpleFileOptions::default().unix_permissions(*mode),
            )
            .unwrap();
            zip.write_all(b"x").unwrap();
        }
        zip.finish().unwrap();
        bytes.into_inner()
    }

    fn walkdir_files(folder: &Path) -> Vec<PathBuf> {
        let mut out = vec![];
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.extend(walkdir_files(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn a_plugin_is_installed_from_a_zip_and_a_checksum_must_match() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("motif");
        write_plugin(&folder, "motif-tools", "2.0.0");
        let archive = temp.path().join("motif.zip");
        let bytes = zip_of(&folder, "motif-tools-2.0.0/", &[]);
        std::fs::write(&archive, &bytes).unwrap();
        let sha = hex::encode(Sha256::digest(&bytes));
        let data = temp.path().join("data");
        // A checksum that does not match stops it before anything is read.
        let wrong = "0".repeat(64);
        let refused = install_checked_blocking(&data, &archive, Some(wrong), "motif.zip");
        assert!(refused.unwrap_err().to_string().contains("SHA-256"));
        assert!(!data.join("plugins/motif-tools").exists());
        let installed =
            install_checked_blocking(&data, &archive, Some(sha.to_uppercase()), "https://x/m.zip")
                .unwrap();
        assert_eq!(installed.trust_state, "checksum_verified");
        assert_eq!(installed.archive_sha256, sha);
        assert_eq!(installed.source_uri, "https://x/m.zip");
        assert!(data
            .join("plugins/motif-tools/2.0.0/skills/find-motifs/SKILL.md")
            .is_file());
        // Without one it is unverified; what it was unpacked into is gone.
        let unverified = install_checked_blocking(&data, &archive, None, "motif.zip").unwrap();
        assert_eq!(unverified.trust_state, "unverified");
        assert_eq!(
            std::fs::read_dir(data.join("plugin-downloads"))
                .unwrap()
                .count(),
            0
        );
        assert!(checked(Some("not-hex"), &sha).is_err());
    }

    #[test]
    fn an_archive_that_reaches_out_or_links_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("motif");
        write_plugin(&folder, "motif-tools", "2.0.0");
        for (extra, what) in [
            (("../escape.txt", 0o644), "leaves"),
            (("motif-tools/link", 0o120777), "symbolic link"),
        ] {
            let archive = temp.path().join("bad.zip");
            std::fs::write(&archive, zip_of(&folder, "", &[extra])).unwrap();
            let error = extract_zip(&archive, &temp.path().join("out"))
                .unwrap_err()
                .to_string();
            assert!(error.contains(what), "{error}");
        }
        assert!(!temp.path().join("escape.txt").exists());
    }

    fn install_checked_blocking(
        data: &Path,
        archive: &Path,
        expected: Option<String>,
        source: &str,
    ) -> Result<PluginInstallation> {
        let sha256 = hex::encode(Sha256::digest(std::fs::read(archive)?));
        let trust = checked(expected.as_deref(), &sha256)?;
        let unpacked = data.join("plugin-downloads").join("test");
        std::fs::create_dir_all(&unpacked)?;
        let installed = extract_zip(archive, &unpacked).and_then(|()| {
            install_files(
                &unpacked,
                data,
                Some(Origin {
                    source: source.into(),
                    sha256,
                    trust,
                }),
            )
        });
        let _ = std::fs::remove_dir_all(&unpacked);
        installed
    }

    #[tokio::test]
    async fn an_address_must_be_https_with_a_checksum_before_anything_is_fetched() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("wisp.sqlite")).await.unwrap();
        let client = reqwest::Client::new();
        let sha = "a".repeat(64);
        for (address, sum, said) in [
            ("http://example.test/p.zip", sha.as_str(), "HTTPS"),
            ("not an address", sha.as_str(), "address"),
            ("https://example.test/p.zip", "abc", "64 hexadecimal"),
        ] {
            let error = install_from_address(&store, temp.path(), address, sum, client.clone())
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(said), "{address}: {error}");
        }
    }

    #[test]
    fn a_package_with_a_symbolic_link_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let (folder, data) = (temp.path().join("motif"), temp.path().join("data"));
        write_plugin(&folder, "motif-tools", "1.2.0");
        std::os::unix::fs::symlink(temp.path(), folder.join("outside")).unwrap();
        let refused = install_files(&folder, &data, None).unwrap_err().to_string();
        assert!(refused.contains("symbolic link"), "{refused}");
        assert!(!data.join("plugins").exists());
    }
}
