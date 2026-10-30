//! Wisp-managed toolchain: uv, Python, R, and Node installed under the app
//! data directory instead of the system, with no administrator rights and no
//! change to the user's shell profile or `PATH`.
//!
//! A pinned, checksum-verified pixi binary installs each tool from conda-forge
//! as its own pixi global environment, with `PIXI_HOME` pointing at the
//! toolchain root so the user's own `~/.pixi` is never read or changed. uv
//! then creates Wisp's default Python environment at the location detection
//! already prefers. Project-specific environments stay with uv (or pixi) in
//! the project. Nothing here runs at startup: installation starts only from an
//! explicit request in setup or Settings.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};
use wisp_dto::{NetworkSettings, ToolchainStatus};
use wisp_runtime::{find_in_conda_prefix, PythonEnv};

pub(crate) const EVENT: &str = "toolchain-install";
const PIXI_VERSION: &str = "0.83.0";
/// Release archives for each supported host, with the SHA-256 published in
/// that release's `sha256.sum`. Pinning both keeps the CLI the commands below
/// were written against and refuses a swapped or truncated download.
const PIXI_ASSETS: &[PixiAsset] = &[
    PixiAsset {
        os: "macos",
        arch: "aarch64",
        triple: "aarch64-apple-darwin",
        archive: Archive::TarGz,
        sha256: "b46066fbaf931284a3f4a61d68e0eb352295f4b3ce4330315a3de3dc5da5736c",
    },
    PixiAsset {
        os: "macos",
        arch: "x86_64",
        triple: "x86_64-apple-darwin",
        archive: Archive::TarGz,
        sha256: "145e26ba860b5a08a2904ca4354cdfdcec1332a8f1d09ea08f1ec663c08e3ac8",
    },
    PixiAsset {
        os: "linux",
        arch: "aarch64",
        triple: "aarch64-unknown-linux-musl",
        archive: Archive::TarGz,
        sha256: "d08403d8f048626ec93ba7b7c56e3b747a5f3a95c4e6e0c815ceb264f5f0b9ce",
    },
    PixiAsset {
        os: "linux",
        arch: "x86_64",
        triple: "x86_64-unknown-linux-musl",
        archive: Archive::TarGz,
        sha256: "064f371e692aaa3400ac581c89da2843c91437f20c2d9155395a5d0037f33551",
    },
    PixiAsset {
        os: "windows",
        arch: "aarch64",
        triple: "aarch64-pc-windows-msvc",
        archive: Archive::Zip,
        sha256: "9ed3a4f7c1fdd2a36936e6d44b1492bfd47d84a57bfbd9be3ae4f338bd3cc92c",
    },
    PixiAsset {
        os: "windows",
        arch: "x86_64",
        triple: "x86_64-pc-windows-msvc",
        archive: Archive::Zip,
        sha256: "77bed9f9d2795c94f9ab5744353892eb82bd3b1eaa09ecbbe7573e22ef70a2f4",
    },
];
const CONDA_FORGE: &str = "https://conda.anaconda.org/conda-forge";
/// The pixi archive is ~20 MB and its binary under 100 MB; anything far
/// larger is not the release asset.
const MAX_DOWNLOAD_BYTES: usize = 256 * 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;
const LOG_LINES: usize = 40;
const CANCELLED: &str = "cancelled";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Archive {
    TarGz,
    Zip,
}

#[derive(Debug)]
pub(crate) struct PixiAsset {
    os: &'static str,
    arch: &'static str,
    triple: &'static str,
    archive: Archive,
    sha256: &'static str,
}

impl PixiAsset {
    fn url(&self) -> String {
        let extension = match self.archive {
            Archive::TarGz => "tar.gz",
            Archive::Zip => "zip",
        };
        format!(
            "https://github.com/prefix-dev/pixi/releases/download/v{PIXI_VERSION}/pixi-{}.{extension}",
            self.triple
        )
    }
}

fn pixi_asset(os: &str, arch: &str) -> Option<&'static PixiAsset> {
    PIXI_ASSETS
        .iter()
        .find(|asset| asset.os == os && asset.arch == arch)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tool {
    Uv,
    Python,
    R,
    Node,
}

impl Tool {
    const ALL: [Tool; 4] = [Tool::Uv, Tool::Python, Tool::R, Tool::Node];

    fn id(self) -> &'static str {
        match self {
            Tool::Uv => "uv",
            Tool::Python => "python",
            Tool::R => "r",
            Tool::Node => "node",
        }
    }

    fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.id() == id)
    }

    /// The pixi global environment that holds this tool, and its conda-forge
    /// specs. R carries jsonlite because the persistent `r` tool needs it.
    fn environment(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Tool::Uv => ("uv", &["uv"]),
            Tool::Python => ("python", &["python=3.12"]),
            Tool::R => ("r", &["r-base", "r-jsonlite"]),
            Tool::Node => ("nodejs", &["nodejs=24"]),
        }
    }
}

/// Where the managed tools live. Pure paths: nothing is created here.
pub(crate) struct Layout {
    root: PathBuf,
    venv: PathBuf,
}

impl Layout {
    pub(crate) fn new(app_data: &Path) -> Self {
        Self {
            root: app_data.join("toolchain"),
            venv: PythonEnv::managed(app_data).venv,
        }
    }

    fn pixi(&self) -> PathBuf {
        self.root.join("bin").join(executable("pixi"))
    }

    fn env(&self, tool: Tool) -> PathBuf {
        self.root.join("envs").join(tool.environment().0)
    }

    fn venv_python(&self) -> PathBuf {
        PythonEnv {
            venv: self.venv.clone(),
        }
        .python()
    }

    fn has_env(&self, tool: Tool) -> bool {
        self.env(tool).join("conda-meta").is_dir()
    }

    fn installed(&self) -> Vec<Tool> {
        Tool::ALL
            .into_iter()
            .filter(|tool| {
                self.has_env(*tool) && (*tool != Tool::Python || self.venv_python().is_file())
            })
            .collect()
    }

    /// Directories put ahead of `PATH` for locally launched code, most
    /// preferred first: Wisp's Python environment (so `python` and `pip` mean
    /// it, not the bare base interpreter), pixi's exposed tools, then the
    /// Node prefix where `npm install -g` puts command-line tools.
    pub(crate) fn path_dirs(&self) -> Vec<PathBuf> {
        let mut directories = Vec::new();
        if self.has_env(Tool::Python) {
            if let Some(bin) = self.venv_python().parent().filter(|bin| bin.is_dir()) {
                directories.push(bin.to_path_buf());
            }
        }
        let bin = self.root.join("bin");
        if bin.is_dir() {
            directories.push(bin);
        }
        if self.has_env(Tool::Node) {
            let prefix = self.env(Tool::Node);
            directories.push(if cfg!(target_os = "windows") {
                prefix
            } else {
                prefix.join("bin")
            });
        }
        directories
    }

    /// A managed executable for detection when neither a saved path nor PATH
    /// provides one.
    pub(crate) fn find(&self, name: &str) -> Option<PathBuf> {
        let directories = std::env::join_paths(self.path_dirs()).ok()?;
        which::which_in(name, Some(directories), &self.root).ok()
    }
}

fn executable(name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// `Rscript` inside an R prefix. On Windows R keeps its own layout under
/// `lib\R\bin`; `direct_rscript` later resolves the architecture binary.
fn rscript_in(prefix: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        let bin = prefix.join("lib").join("R").join("bin");
        [bin.join("x64").join("Rscript.exe"), bin.join("Rscript.exe")]
            .into_iter()
            .find(|path| path.is_file())
            .or_else(|| find_in_conda_prefix(prefix, "Rscript"))
    } else {
        find_in_conda_prefix(prefix, "Rscript").or_else(|| {
            Some(prefix.join("lib").join("R").join("bin").join("Rscript")).filter(|p| p.is_file())
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Pixi,
    Install(Tool),
    Venv,
    Packages,
    Save,
}

impl Step {
    fn label(&self) -> String {
        match self {
            Step::Pixi => "pixi".into(),
            Step::Install(tool) => format!("install:{}", tool.id()),
            Step::Venv => "venv".into(),
            Step::Packages => "packages".into(),
            Step::Save => "save".into(),
        }
    }
}

/// The tools a request installs: Python brings uv, which creates and fills
/// Wisp's Python environment.
fn normalize(tools: &[Tool]) -> Vec<Tool> {
    let mut tools = tools.iter().copied().collect::<BTreeSet<_>>();
    if tools.contains(&Tool::Python) {
        tools.insert(Tool::Uv);
    }
    tools.into_iter().collect()
}

fn plan(tools: &[Tool]) -> Vec<Step> {
    let mut steps = vec![Step::Pixi];
    steps.extend(tools.iter().copied().map(Step::Install));
    if tools.contains(&Tool::Python) {
        steps.extend([Step::Venv, Step::Packages]);
    }
    steps.push(Step::Save);
    steps
}

fn toml_string(value: &str) -> String {
    // A JSON string literal is a valid TOML basic string.
    serde_json::to_string(value).expect("strings always serialize")
}

/// pixi's configuration in `PIXI_HOME`, which outranks the user's own files.
/// The Network page's Conda mirror stands in for conda-forge, and a custom CA
/// bundle needs the system trust store, where `SSL_CERT_FILE` is honored.
fn pixi_config(network: &NetworkSettings) -> String {
    let mut config =
        String::from("# Written by Wisp Science for its managed toolchain; edits are replaced.\n");
    if !network.ca_bundle_path.trim().is_empty() {
        config.push_str("tls-root-certs = \"system\"\n");
    }
    let mirror = network.conda_mirror_url.trim().trim_end_matches('/');
    if !mirror.is_empty() {
        config.push_str(&format!(
            "\n[mirrors]\n{} = [{}]\n",
            toml_string(CONDA_FORGE),
            toml_string(mirror)
        ));
    }
    config
}

/// Environment for pixi and uv. Proxies follow the Network page's Code
/// requests row; the venv must use the interpreter pixi installed, never a
/// separate uv download.
fn child_env(layout: &Layout, network: &NetworkSettings) -> Vec<(String, String)> {
    let mut envs = vec![
        (
            "PIXI_HOME".to_owned(),
            layout.root.to_string_lossy().into_owned(),
        ),
        ("NO_COLOR".to_owned(), "1".to_owned()),
        ("UV_PYTHON_DOWNLOADS".to_owned(), "never".to_owned()),
    ];
    envs.extend(wisp_tools::network::proxy_env(&network.command_proxy_url));
    let index = network.pip_index_url.trim();
    if !index.is_empty() {
        envs.push(("UV_DEFAULT_INDEX".to_owned(), index.to_owned()));
    }
    let ca_bundle = network.ca_bundle_path.trim();
    if !ca_bundle.is_empty() {
        envs.push(("SSL_CERT_FILE".to_owned(), ca_bundle.to_owned()));
    }
    envs
}

fn verify_sha256(bytes: &[u8], expected: &str) -> Result<(), String> {
    use sha2::Digest;
    let actual = hex::encode(sha2::Sha256::digest(bytes));
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "pixi download failed verification (sha256 {actual}, expected {expected}). Retry, or check whether a proxy rewrote the download."
        ))
    }
}

/// The single `pixi` executable from a release archive.
fn extract_pixi(bytes: &[u8], archive: Archive) -> Result<Vec<u8>, String> {
    let invalid = |error: std::io::Error| format!("pixi archive could not be read: {error}");
    let mut binary = Vec::new();
    match archive {
        Archive::TarGz => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
            for entry in tar.entries().map_err(invalid)? {
                let entry = entry.map_err(invalid)?;
                let name = entry.path().map_err(invalid)?.into_owned();
                if entry.header().entry_type().is_file()
                    && name.file_name().and_then(|name| name.to_str()) == Some("pixi")
                {
                    entry
                        .take(MAX_BINARY_BYTES)
                        .read_to_end(&mut binary)
                        .map_err(invalid)?;
                    return Ok(binary);
                }
            }
        }
        Archive::Zip => {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
                .map_err(|error| format!("pixi archive could not be read: {error}"))?;
            for index in 0..zip.len() {
                let file = zip
                    .by_index(index)
                    .map_err(|error| format!("pixi archive could not be read: {error}"))?;
                let name = file.enclosed_name();
                if file.is_file()
                    && name
                        .as_deref()
                        .and_then(|name| name.file_name())
                        .and_then(|name| name.to_str())
                        == Some("pixi.exe")
                {
                    file.take(MAX_BINARY_BYTES)
                        .read_to_end(&mut binary)
                        .map_err(invalid)?;
                    return Ok(binary);
                }
            }
        }
    }
    Err("pixi archive does not contain the pixi executable".into())
}

/// Terminal output as one log line: the last carriage-return frame of a
/// progress display, without color codes.
fn clean_line(line: &str) -> Option<String> {
    static ANSI: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let ansi = ANSI.get_or_init(|| regex::Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").unwrap());
    let frame = line.rsplit('\r').find(|frame| !frame.trim().is_empty())?;
    let text = ansi.replace_all(frame, "");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

pub(crate) enum Progress {
    Step {
        index: usize,
        total: usize,
        label: String,
    },
    Line(String),
}

type Report<'a> = &'a (dyn Fn(Progress) + Send + Sync);

/// The two side effects an install has besides files: running pixi/uv and
/// downloading pixi. Tests substitute both.
#[async_trait::async_trait]
pub(crate) trait Host: Send + Sync {
    /// Run to completion, passing each output line on; non-zero exit is an error.
    async fn run(
        &self,
        program: &Path,
        args: &[String],
        envs: &[(String, String)],
        line: &(dyn Fn(String) + Send + Sync),
    ) -> Result<(), String>;

    async fn download(
        &self,
        url: &str,
        progress: &(dyn Fn(usize, Option<u64>) + Send + Sync),
    ) -> Result<Vec<u8>, String>;
}

pub(crate) struct Installer<'a> {
    pub layout: &'a Layout,
    pub network: &'a NetworkSettings,
    pub host: &'a dyn Host,
    /// The pixi release for this host, or `None` where Wisp has none.
    pub pixi: Option<&'a PixiAsset>,
    /// The bundled list of common analysis packages, when present.
    pub requirements: Option<PathBuf>,
    pub report: Report<'a>,
}

impl Installer<'_> {
    /// Install `tools` (already normalized) and return the executable paths to
    /// save on the Local execution context.
    pub(crate) async fn run(&self, tools: &[Tool]) -> Result<BTreeMap<String, String>, String> {
        let steps = plan(tools);
        let envs = child_env(self.layout, self.network);
        for (index, step) in steps.iter().enumerate() {
            (self.report)(Progress::Step {
                index: index + 1,
                total: steps.len(),
                label: step.label(),
            });
            match step {
                Step::Pixi => self.ensure_pixi().await?,
                Step::Install(tool) => {
                    let (environment, specs) = tool.environment();
                    let mut args = ["global", "install", "--environment", environment]
                        .map(String::from)
                        .to_vec();
                    args.extend(specs.iter().map(|spec| spec.to_string()));
                    self.command(&self.layout.pixi(), &args, &envs).await?;
                }
                Step::Venv => self.create_venv(&envs).await?,
                Step::Packages => self.install_packages(&envs).await?,
                Step::Save => {}
            }
        }
        self.installed_paths(tools)
    }

    async fn command(
        &self,
        program: &Path,
        args: &[String],
        envs: &[(String, String)],
    ) -> Result<(), String> {
        let name = program
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.line(format!("$ {name} {}", args.join(" ")));
        let line = |text: String| self.line(text);
        self.host.run(program, args, envs, &line).await
    }

    fn line(&self, text: String) {
        if let Some(text) = clean_line(&text) {
            (self.report)(Progress::Line(text));
        }
    }

    async fn ensure_pixi(&self) -> Result<(), String> {
        let io = |error: std::io::Error| {
            format!("Could not prepare {}: {error}", self.layout.root.display())
        };
        let root = &self.layout.root;
        std::fs::create_dir_all(root.join("bin")).map_err(io)?;
        std::fs::create_dir_all(root.join("manifests")).map_err(io)?;
        // pixi falls back to the user's own global manifest when this one is
        // missing; creating it keeps every install inside the toolchain.
        let manifest = root.join("manifests").join("pixi-global.toml");
        if !manifest.is_file() {
            std::fs::write(&manifest, "version = 1\n").map_err(io)?;
        }
        std::fs::write(root.join("config.toml"), pixi_config(self.network)).map_err(io)?;
        let mirror = self.network.conda_mirror_url.trim();
        if !mirror.is_empty() {
            self.line(format!("conda-forge → {mirror}"));
        }

        let pixi = self.layout.pixi();
        let marker = root.join("bin").join(".pixi-version");
        if pixi.is_file() && std::fs::read_to_string(&marker).ok().as_deref() == Some(PIXI_VERSION)
        {
            self.line(format!("pixi {PIXI_VERSION}: {}", pixi.display()));
            return Ok(());
        }
        let asset = self.pixi.ok_or_else(|| {
            format!(
                "Wisp cannot install tools on {}/{} yet. Install them yourself and use Edit paths.",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
        let url = asset.url();
        self.line(format!("Downloading {url}"));
        let reported = std::sync::atomic::AtomicUsize::new(0);
        let progress = |received: usize, total: Option<u64>| {
            let Some(total) = total.filter(|total| *total > 0) else {
                return;
            };
            let percent = (received as u64 * 100 / total).min(100) as usize;
            // Five lines over the whole download is enough to show it moves.
            let bucket = percent / 20;
            if reported.fetch_max(bucket, std::sync::atomic::Ordering::Relaxed) < bucket {
                self.line(format!(
                    "pixi {percent}% ({:.1} / {:.1} MB)",
                    received as f64 / 1e6,
                    total as f64 / 1e6
                ));
            }
        };
        let bytes = self.host.download(&url, &progress).await.map_err(|error| {
            if error == CANCELLED {
                error
            } else {
                format!("Could not download pixi from GitHub: {error}. Check the Code requests proxy in Settings → Network, then retry.")
            }
        })?;
        verify_sha256(&bytes, asset.sha256)?;
        let binary = extract_pixi(&bytes, asset.archive)?;
        let staged = root
            .join("bin")
            .join(format!(".{}.part", executable("pixi")));
        std::fs::write(&staged, binary).map_err(io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
                .map_err(io)?;
        }
        std::fs::rename(&staged, &pixi).map_err(io)?;
        std::fs::write(&marker, PIXI_VERSION).map_err(io)?;
        self.line(format!("pixi {PIXI_VERSION}: {}", pixi.display()));
        Ok(())
    }

    fn uv(&self) -> Result<PathBuf, String> {
        find_in_conda_prefix(&self.layout.env(Tool::Uv), "uv").ok_or_else(|| {
            format!(
                "uv is not in {}; install uv again.",
                self.layout.env(Tool::Uv).display()
            )
        })
    }

    async fn create_venv(&self, envs: &[(String, String)]) -> Result<(), String> {
        let venv = &self.layout.venv;
        // Keep an existing environment and the packages the user put in it.
        if self.layout.venv_python().is_file() && venv.join("pyvenv.cfg").is_file() {
            self.line(format!("Wisp Python environment: {}", venv.display()));
            return Ok(());
        }
        let base = find_in_conda_prefix(&self.layout.env(Tool::Python), "python")
            .ok_or_else(|| "Python was installed but its interpreter was not found.".to_owned())?;
        if let Some(parent) = venv.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let args = [
            "venv".to_owned(),
            "--seed".to_owned(),
            "--python".to_owned(),
            base.to_string_lossy().into_owned(),
            venv.to_string_lossy().into_owned(),
        ];
        self.command(&self.uv()?, &args, envs).await
    }

    async fn install_packages(&self, envs: &[(String, String)]) -> Result<(), String> {
        let Some(requirements) = self.requirements.as_ref().filter(|path| path.is_file()) else {
            self.line("No bundled package list; skipped.".into());
            return Ok(());
        };
        let args = [
            "pip".to_owned(),
            "install".to_owned(),
            "--python".to_owned(),
            self.layout.venv_python().to_string_lossy().into_owned(),
            "-r".to_owned(),
            requirements.to_string_lossy().into_owned(),
        ];
        self.command(&self.uv()?, &args, envs).await
    }

    fn installed_paths(&self, tools: &[Tool]) -> Result<BTreeMap<String, String>, String> {
        let missing = |what: &str, tool: Tool| {
            format!(
                "{what} was installed but not found in {}.",
                self.layout.env(tool).display()
            )
        };
        let mut paths = BTreeMap::new();
        let mut save = |key: &str, path: PathBuf| {
            paths.insert(key.to_owned(), path.to_string_lossy().into_owned());
        };
        for tool in tools {
            let prefix = self.layout.env(*tool);
            match tool {
                Tool::Uv => save(
                    "uv_executable",
                    find_in_conda_prefix(&prefix, "uv").ok_or_else(|| missing("uv", *tool))?,
                ),
                Tool::Python => {
                    let python = self.layout.venv_python();
                    if !python.is_file() {
                        return Err(missing("Python", *tool));
                    }
                    save("python_executable", python);
                }
                Tool::R => save(
                    "rscript_executable",
                    rscript_in(&prefix).ok_or_else(|| missing("Rscript", *tool))?,
                ),
                Tool::Node => {
                    save(
                        "node_executable",
                        find_in_conda_prefix(&prefix, "node")
                            .ok_or_else(|| missing("node", *tool))?,
                    );
                    if let Some(npm) = find_in_conda_prefix(&prefix, "npm") {
                        save("npm_executable", npm);
                    }
                }
            }
        }
        Ok(paths)
    }
}

/// The real host: child processes and an HTTP download, both stoppable.
struct ProcessHost {
    client: reqwest::Client,
    cancel: tokio::sync::watch::Receiver<bool>,
}

async fn cancelled(mut cancel: tokio::sync::watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[async_trait::async_trait]
impl Host for ProcessHost {
    async fn run(
        &self,
        program: &Path,
        args: &[String],
        envs: &[(String, String)],
        line: &(dyn Fn(String) + Send + Sync),
    ) -> Result<(), String> {
        use tokio::io::AsyncBufReadExt;
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .envs(envs.iter().cloned())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        wisp_tools::process::hide_console_async(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start {}: {error}", program.display()))?;
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<String>();
        let mut readers = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            let sender = sender.clone();
            readers.push(tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stdout).lines();
                while let Ok(Some(text)) = lines.next_line().await {
                    let _ = sender.send(text);
                }
            }));
        }
        if let Some(stderr) = child.stderr.take() {
            let sender = sender.clone();
            readers.push(tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(text)) = lines.next_line().await {
                    let _ = sender.send(text);
                }
            }));
        }
        drop(sender);
        let stop = cancelled(self.cancel.clone());
        tokio::pin!(stop);
        let status = loop {
            tokio::select! {
                Some(text) = receiver.recv() => line(text),
                status = child.wait() => break status,
                _ = &mut stop => {
                    let _ = child.kill().await;
                    return Err(CANCELLED.into());
                }
            }
        };
        for reader in readers {
            let _ = reader.await;
        }
        while let Ok(text) = receiver.try_recv() {
            line(text);
        }
        let status = status.map_err(|error| error.to_string())?;
        if status.success() {
            Ok(())
        } else {
            let name = program
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            Err(format!(
                "{name} {} failed ({status}); see the log for details.",
                args.first().map(String::as_str).unwrap_or_default()
            ))
        }
    }

    async fn download(
        &self,
        url: &str,
        progress: &(dyn Fn(usize, Option<u64>) + Send + Sync),
    ) -> Result<Vec<u8>, String> {
        let stop = cancelled(self.cancel.clone());
        tokio::pin!(stop);
        let mut response = tokio::select! {
            response = self.client.get(url).send() => response
                .and_then(|response| response.error_for_status())
                .map_err(|error| error.to_string())?,
            _ = &mut stop => return Err(CANCELLED.into()),
        };
        let total = response.content_length();
        let mut bytes = Vec::new();
        loop {
            tokio::select! {
                chunk = response.chunk() => match chunk.map_err(|error| error.to_string())? {
                    Some(chunk) => {
                        bytes.extend_from_slice(&chunk);
                        if bytes.len() > MAX_DOWNLOAD_BYTES {
                            return Err("download is larger than the pixi release".into());
                        }
                        progress(bytes.len(), total);
                    }
                    None => return Ok(bytes),
                },
                _ = &mut stop => return Err(CANCELLED.into()),
            }
        }
    }
}

/// A client on the Code requests route, trusting the configured CA bundle.
/// No overall timeout: a slow link may need minutes for the archive.
fn download_client(network: &NetworkSettings) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60));
    match network.command_proxy_url.trim() {
        "" => {}
        "none" => builder = builder.no_proxy(),
        proxy => {
            builder = builder
                .proxy(reqwest::Proxy::all(proxy).map_err(|error| format!("Code proxy: {error}"))?)
        }
    }
    let ca_bundle = network.ca_bundle_path.trim();
    if !ca_bundle.is_empty() {
        let pem =
            std::fs::read(ca_bundle).map_err(|error| format!("CA bundle {ca_bundle}: {error}"))?;
        for certificate in reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|error| format!("CA bundle {ca_bundle}: {error}"))?
        {
            builder = builder.add_root_certificate(certificate);
        }
    }
    builder.build().map_err(|error| error.to_string())
}

#[derive(Default)]
struct Job {
    status: ToolchainStatus,
    cancel: Option<tokio::sync::watch::Sender<bool>>,
}

static JOB: LazyLock<Mutex<Job>> = LazyLock::new(Default::default);

fn snapshot(layout: &Layout) -> ToolchainStatus {
    let mut status = JOB.lock().unwrap().status.clone();
    status.root = layout.root.to_string_lossy().into_owned();
    status.installed = layout
        .installed()
        .into_iter()
        .map(|tool| tool.id().to_owned())
        .collect();
    if status.state.is_empty() {
        status.state = "idle".into();
    }
    status
}

fn publish(app: &tauri::AppHandle, layout: &Layout) {
    let _ = app.emit(EVENT, snapshot(layout));
}

fn update(app: &tauri::AppHandle, layout: &Layout, change: impl FnOnce(&mut ToolchainStatus)) {
    change(&mut JOB.lock().unwrap().status);
    publish(app, layout);
}

fn push_line(status: &mut ToolchainStatus, line: String) {
    status.log.push(line);
    let excess = status.log.len().saturating_sub(LOG_LINES);
    status.log.drain(..excess);
}

#[tauri::command]
pub(crate) fn get_toolchain_status(state: tauri::State<'_, crate::AppState>) -> ToolchainStatus {
    snapshot(&Layout::new(&state.app_data))
}

#[tauri::command]
pub(crate) async fn install_toolchain(
    app: tauri::AppHandle,
    tools: Vec<String>,
) -> Result<ToolchainStatus, String> {
    let requested = tools
        .iter()
        .map(|id| Tool::parse(id).ok_or_else(|| format!("Unknown tool: {id}")))
        .collect::<Result<Vec<_>, _>>()?;
    let tools = normalize(&requested);
    if tools.is_empty() {
        return Err("Choose at least one tool to install.".into());
    }
    let state = app.state::<crate::AppState>();
    let layout = Layout::new(&state.app_data);
    let network = crate::network::load(&state.store).await?;
    let client = download_client(&network)?;
    let (cancel, cancel_rx) = tokio::sync::watch::channel(false);
    {
        let mut job = JOB.lock().unwrap();
        if job.status.state == "running" {
            return Err("An installation is already running.".into());
        }
        job.cancel = Some(cancel);
        job.status = ToolchainStatus {
            state: "running".into(),
            tools: tools.iter().map(|tool| tool.id().to_owned()).collect(),
            total: plan(&tools).len(),
            ..Default::default()
        };
    }
    publish(&app, &layout);
    let initial = snapshot(&layout);
    tracing::info!(target: "wisp", tools = ?initial.tools, "installing managed toolchain");
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let app = handle;
        let host = ProcessHost {
            client,
            cancel: cancel_rx,
        };
        let report = |progress: Progress| {
            update(&app, &layout, |status| match progress {
                Progress::Step {
                    index,
                    total,
                    label,
                } => {
                    status.step = index;
                    status.total = total;
                    status.label = label;
                }
                Progress::Line(line) => push_line(status, line),
            })
        };
        let installer = Installer {
            layout: &layout,
            network: &network,
            host: &host,
            pixi: pixi_asset(std::env::consts::OS, std::env::consts::ARCH),
            requirements: wisp_paths::python_requirements_path(),
            report: &report,
        };
        let mut result = installer.run(&tools).await;
        if let Ok(paths) = &result {
            let state = app.state::<crate::AppState>();
            result = state
                .store
                .save_local_environment_paths(paths)
                .await
                .map(|()| paths.clone())
                .map_err(|error| format!("Installed, but the paths could not be saved: {error}"));
        }
        wisp_tools::local_env::set_managed_path(layout.path_dirs());
        // Refresh the Local paths every environment panel shows.
        crate::app_commands::detect_local_environment(app.clone()).await;
        {
            let mut job = JOB.lock().unwrap();
            job.cancel = None;
            match &result {
                Ok(_) => job.status.state = "done".into(),
                Err(error) if error == CANCELLED => job.status.state = "cancelled".into(),
                Err(error) => {
                    job.status.state = "failed".into();
                    job.status.error = Some(error.clone());
                }
            }
        }
        match &result {
            Ok(_) => tracing::info!(target: "wisp", "managed toolchain installed"),
            Err(error) => {
                tracing::warn!(target: "wisp", %error, "managed toolchain install stopped")
            }
        }
        publish(&app, &layout);
    });
    Ok(initial)
}

/// Stop the running installation. pixi and uv leave finished environments
/// usable and the next install resumes from them.
#[tauri::command]
pub(crate) fn cancel_toolchain_install() -> bool {
    JOB.lock()
        .unwrap()
        .cancel
        .as_ref()
        .is_some_and(|cancel| cancel.send(true).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("wisp-toolchain-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn python_brings_uv_and_its_environment_steps() {
        let tools = normalize(&[Tool::Node, Tool::Python, Tool::Python]);
        assert_eq!(tools, [Tool::Uv, Tool::Python, Tool::Node]);
        assert_eq!(
            plan(&tools).iter().map(Step::label).collect::<Vec<_>>(),
            [
                "pixi",
                "install:uv",
                "install:python",
                "install:node",
                "venv",
                "packages",
                "save"
            ]
        );
        // R alone needs neither uv nor a Python environment.
        assert_eq!(
            plan(&normalize(&[Tool::R]))
                .iter()
                .map(Step::label)
                .collect::<Vec<_>>(),
            ["pixi", "install:r", "save"]
        );
        assert_eq!(Tool::parse("r"), Some(Tool::R));
        assert_eq!(Tool::parse("conda"), None);
    }

    #[test]
    fn every_supported_host_has_a_pinned_verified_archive() {
        for (os, arch) in [
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("linux", "aarch64"),
            ("linux", "x86_64"),
            ("windows", "aarch64"),
            ("windows", "x86_64"),
        ] {
            let asset = pixi_asset(os, arch).unwrap();
            assert_eq!(asset.sha256.len(), 64, "{os}/{arch}");
            assert!(asset.url().contains(&format!("/v{PIXI_VERSION}/")));
            assert_eq!(asset.url().ends_with(".zip"), os == "windows");
        }
        assert!(pixi_asset("freebsd", "x86_64").is_none());
    }

    #[test]
    fn pixi_config_maps_conda_forge_to_the_network_mirror() {
        assert!(!pixi_config(&NetworkSettings::default()).contains("[mirrors]"));
        let config = pixi_config(&NetworkSettings {
            conda_mirror_url: "https://mirrors.tuna.tsinghua.edu.cn/anaconda/cloud/conda-forge/"
                .into(),
            ca_bundle_path: "/etc/ssl/lab.pem".into(),
            ..Default::default()
        });
        assert!(config.contains(
            "[mirrors]\n\"https://conda.anaconda.org/conda-forge\" = [\"https://mirrors.tuna.tsinghua.edu.cn/anaconda/cloud/conda-forge\"]\n"
        ));
        assert!(config.contains("tls-root-certs = \"system\""));
        // Quotes in a value stay inside the TOML string.
        assert!(pixi_config(&NetworkSettings {
            conda_mirror_url: "https://m.test/a\"b".into(),
            ..Default::default()
        })
        .contains(r#"["https://m.test/a\"b"]"#));
    }

    #[test]
    fn child_environment_isolates_pixi_and_follows_network_settings() {
        let root = temp_root();
        let layout = Layout::new(&root);
        let envs = child_env(
            &layout,
            &NetworkSettings {
                command_proxy_url: "http://127.0.0.1:7890".into(),
                pip_index_url: "https://pypi.tuna.tsinghua.edu.cn/simple".into(),
                ..Default::default()
            },
        )
        .into_iter()
        .collect::<BTreeMap<_, _>>();
        assert_eq!(
            envs["PIXI_HOME"],
            root.join("toolchain").to_string_lossy().as_ref()
        );
        assert_eq!(envs["UV_PYTHON_DOWNLOADS"], "never");
        assert_eq!(envs["HTTPS_PROXY"], "http://127.0.0.1:7890");
        assert_eq!(
            envs["UV_DEFAULT_INDEX"],
            "https://pypi.tuna.tsinghua.edu.cn/simple"
        );
        assert!(!envs.contains_key("SSL_CERT_FILE"));
        let plain = child_env(&layout, &NetworkSettings::default());
        assert!(plain
            .iter()
            .all(|(key, _)| key != "HTTPS_PROXY" && key != "UV_DEFAULT_INDEX"));
        assert!(!root.exists(), "building the environment creates nothing");
    }

    fn tar_gz(name: &str, body: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, name, body).unwrap();
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn release_archives_yield_only_the_pixi_binary() {
        assert_eq!(
            extract_pixi(&tar_gz("pixi", b"binary"), Archive::TarGz).unwrap(),
            b"binary"
        );
        assert!(extract_pixi(&tar_gz("README", b"text"), Archive::TarGz).is_err());

        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("pixi.exe", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, b"windows binary").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert_eq!(
            extract_pixi(&bytes, Archive::Zip).unwrap(),
            b"windows binary"
        );
        assert!(extract_pixi(b"not an archive", Archive::Zip).is_err());
    }

    #[test]
    fn checksum_mismatch_is_refused() {
        use sha2::Digest;
        let body = b"pixi";
        let good = hex::encode(sha2::Sha256::digest(body));
        assert!(verify_sha256(body, &good).is_ok());
        assert!(verify_sha256(body, &good.to_uppercase()).is_ok());
        assert!(verify_sha256(b"tampered", &good).is_err());
    }

    #[test]
    fn progress_frames_become_plain_log_lines() {
        assert_eq!(
            clean_line("\x1b[32m✔\x1b[0m The python environment has been installed.").as_deref(),
            Some("✔ The python environment has been installed.")
        );
        assert_eq!(clean_line("10%\r55%\r  \r").as_deref(), Some("55%"));
        assert_eq!(clean_line("   "), None);
    }

    /// Simulates pixi and uv by creating the files they would produce, and
    /// records every command so the sequence can be checked without network.
    struct FakeHost {
        layout_root: PathBuf,
        venv: PathBuf,
        archive: Vec<u8>,
        commands: Mutex<Vec<String>>,
        fail_on: Option<&'static str>,
    }

    fn executable_file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn prefix_bin(prefix: &Path) -> PathBuf {
        if cfg!(target_os = "windows") {
            prefix.to_path_buf()
        } else {
            prefix.join("bin")
        }
    }

    #[async_trait::async_trait]
    impl Host for FakeHost {
        async fn run(
            &self,
            program: &Path,
            args: &[String],
            _envs: &[(String, String)],
            line: &(dyn Fn(String) + Send + Sync),
        ) -> Result<(), String> {
            let name = program.file_stem().unwrap().to_string_lossy().into_owned();
            let command = format!("{name} {}", args.join(" "));
            self.commands.lock().unwrap().push(command.clone());
            if self.fail_on.is_some_and(|needle| command.contains(needle)) {
                line("solve failed".into());
                return Err(format!("{name} {} failed", args[0]));
            }
            match (name.as_str(), args.first().map(String::as_str)) {
                ("pixi", Some("global")) => {
                    let environment = &args[3];
                    let prefix = self.layout_root.join("envs").join(environment);
                    std::fs::create_dir_all(prefix.join("conda-meta")).unwrap();
                    let bin = prefix_bin(&prefix);
                    match environment.as_str() {
                        "uv" => executable_file(&bin.join(executable("uv"))),
                        "python" => executable_file(&bin.join(executable("python"))),
                        "r" => {
                            if cfg!(target_os = "windows") {
                                executable_file(&prefix.join("lib/R/bin/x64").join("Rscript.exe"));
                            } else {
                                executable_file(&bin.join("Rscript"));
                            }
                        }
                        "nodejs" => {
                            executable_file(&bin.join(executable("node")));
                            executable_file(&bin.join(if cfg!(target_os = "windows") {
                                "npm.cmd".to_owned()
                            } else {
                                "npm".to_owned()
                            }));
                        }
                        other => panic!("unexpected environment {other}"),
                    }
                    line("✔ installed".into());
                }
                ("uv", Some("venv")) => {
                    executable_file(
                        &PythonEnv {
                            venv: self.venv.clone(),
                        }
                        .python(),
                    );
                    std::fs::write(self.venv.join("pyvenv.cfg"), "home = fixture\n").unwrap();
                }
                ("uv", Some("pip")) => {}
                other => panic!("unexpected command {other:?}"),
            }
            Ok(())
        }

        async fn download(
            &self,
            url: &str,
            progress: &(dyn Fn(usize, Option<u64>) + Send + Sync),
        ) -> Result<Vec<u8>, String> {
            self.commands
                .lock()
                .unwrap()
                .push(format!("download {url}"));
            progress(self.archive.len(), Some(self.archive.len() as u64));
            Ok(self.archive.clone())
        }
    }

    fn fake_host(app_data: &Path, fail_on: Option<&'static str>) -> (FakeHost, Layout) {
        let layout = Layout::new(app_data);
        let host = FakeHost {
            layout_root: layout.root.clone(),
            venv: layout.venv.clone(),
            archive: tar_gz("pixi", b"fake pixi"),
            commands: Mutex::new(Vec::new()),
            fail_on,
        };
        (host, layout)
    }

    #[tokio::test]
    async fn install_runs_pixi_then_uv_and_returns_the_paths_to_save() {
        let app_data = temp_root();
        let (host, layout) = fake_host(&app_data, None);
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let steps = Arc::new(Mutex::new(Vec::<String>::new()));
        let report = {
            let (lines, steps) = (lines.clone(), steps.clone());
            move |progress: Progress| match progress {
                Progress::Step { label, .. } => steps.lock().unwrap().push(label),
                Progress::Line(line) => lines.lock().unwrap().push(line),
            }
        };
        let requirements = app_data.join("requirements-kernel.txt");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::write(&requirements, "pandas\n").unwrap();
        // Pretend pixi is already present so no checksum is involved here.
        executable_file(&layout.pixi());
        std::fs::write(layout.root.join("bin/.pixi-version"), PIXI_VERSION).unwrap();
        let network = NetworkSettings::default();
        let tools = normalize(&[Tool::Python, Tool::R, Tool::Node]);
        let paths = Installer {
            layout: &layout,
            network: &network,
            host: &host,
            pixi: pixi_asset("macos", "aarch64"),
            requirements: Some(requirements.clone()),
            report: &report,
        }
        .run(&tools)
        .await
        .unwrap();

        let commands = host.commands.lock().unwrap().clone();
        assert_eq!(
            commands,
            [
                "pixi global install --environment uv uv".to_owned(),
                "pixi global install --environment python python=3.12".to_owned(),
                "pixi global install --environment r r-base r-jsonlite".to_owned(),
                "pixi global install --environment nodejs nodejs=24".to_owned(),
                format!(
                    "uv venv --seed --python {} {}",
                    prefix_bin(&layout.env(Tool::Python))
                        .join(executable("python"))
                        .display(),
                    layout.venv.display()
                ),
                format!(
                    "uv pip install --python {} -r {}",
                    layout.venv_python().display(),
                    requirements.display()
                ),
            ]
        );
        assert_eq!(steps.lock().unwrap().len(), plan(&tools).len());
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|line| line == "✔ installed"));
        assert_eq!(
            paths.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "node_executable",
                "npm_executable",
                "python_executable",
                "rscript_executable",
                "uv_executable"
            ]
        );
        assert_eq!(
            paths["python_executable"],
            layout.venv_python().to_string_lossy().as_ref()
        );
        assert!(Path::new(&paths["rscript_executable"]).starts_with(layout.env(Tool::R)));
        // pixi must never fall back to the user's own global manifest.
        assert_eq!(
            std::fs::read_to_string(layout.root.join("manifests/pixi-global.toml")).unwrap(),
            "version = 1\n"
        );
        assert_eq!(
            layout.installed(),
            [Tool::Uv, Tool::Python, Tool::R, Tool::Node]
        );
        let dirs = layout.path_dirs();
        assert_eq!(dirs[0], layout.venv_python().parent().unwrap());
        assert_eq!(dirs[1], layout.root.join("bin"));
        // Only the managed PATH directories are searched: the Node prefix is
        // one, uv's own prefix is not (pixi exposes uv into `bin`).
        assert_eq!(
            layout.find("node"),
            Some(prefix_bin(&layout.env(Tool::Node)).join(executable("node")))
        );
        assert_eq!(layout.find("uv"), None);

        // A second run keeps the environment and its packages.
        host.commands.lock().unwrap().clear();
        Installer {
            layout: &layout,
            network: &network,
            host: &host,
            pixi: pixi_asset("macos", "aarch64"),
            requirements: None,
            report: &report,
        }
        .run(&[Tool::Uv, Tool::Python])
        .await
        .unwrap();
        assert!(host
            .commands
            .lock()
            .unwrap()
            .iter()
            .all(|command| !command.starts_with("uv venv")));
        std::fs::remove_dir_all(app_data).unwrap();
    }

    #[tokio::test]
    async fn pixi_is_downloaded_verified_and_unpacked_into_the_toolchain() {
        use sha2::Digest;
        let app_data = temp_root();
        let (host, layout) = fake_host(&app_data, None);
        let network = NetworkSettings::default();
        let report = |_: Progress| {};
        let run = |pixi: Option<&'static PixiAsset>| {
            let (layout, network, host, report) = (&layout, &network, &host, &report);
            async move {
                Installer {
                    layout,
                    network,
                    host,
                    pixi,
                    requirements: None,
                    report,
                }
                .run(&[Tool::R])
                .await
            }
        };

        // The real archive's checksum does not match the fixture: refused
        // before anything lands in the toolchain.
        let error = run(pixi_asset("macos", "aarch64")).await.unwrap_err();
        assert!(error.contains("verification"), "{error}");
        assert!(!layout.pixi().exists());
        assert!(host.commands.lock().unwrap()[0]
            .starts_with("download https://github.com/prefix-dev/pixi/releases/download/v"));

        // With a matching checksum the binary is unpacked, marked with its
        // version, and the install continues to pixi.
        let fixture: &'static PixiAsset = Box::leak(Box::new(PixiAsset {
            os: "macos",
            arch: "aarch64",
            triple: "aarch64-apple-darwin",
            archive: Archive::TarGz,
            sha256: Box::leak(hex::encode(sha2::Sha256::digest(&host.archive)).into_boxed_str()),
        }));
        host.commands.lock().unwrap().clear();
        run(Some(fixture)).await.unwrap();
        assert_eq!(std::fs::read(layout.pixi()).unwrap(), b"fake pixi");
        assert_eq!(
            std::fs::read_to_string(layout.root.join("bin/.pixi-version")).unwrap(),
            PIXI_VERSION
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(layout.pixi())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        let commands = host.commands.lock().unwrap().clone();
        assert_eq!(commands.len(), 2, "{commands:?}");
        assert!(commands[1].starts_with("pixi global install --environment r"));

        // Once present, pixi is not downloaded again.
        host.commands.lock().unwrap().clear();
        run(Some(fixture)).await.unwrap();
        assert!(host
            .commands
            .lock()
            .unwrap()
            .iter()
            .all(|command| !command.starts_with("download")));

        // A host without a release stops with a manual-setup hint.
        std::fs::remove_file(layout.pixi()).unwrap();
        host.commands.lock().unwrap().clear();
        let error = run(None).await.unwrap_err();
        assert!(error.contains("Edit paths"), "{error}");
        assert!(host.commands.lock().unwrap().is_empty());
        std::fs::remove_dir_all(app_data).unwrap();
    }

    #[tokio::test]
    async fn a_failed_step_stops_the_install_without_paths() {
        let app_data = temp_root();
        let (host, layout) = fake_host(&app_data, Some("r-base"));
        executable_file(&layout.pixi());
        std::fs::write(layout.root.join("bin/.pixi-version"), PIXI_VERSION).unwrap();
        let network = NetworkSettings::default();
        let lines = Mutex::new(Vec::new());
        let report = |progress: Progress| {
            if let Progress::Line(line) = progress {
                lines.lock().unwrap().push(line);
            }
        };
        let error = Installer {
            layout: &layout,
            network: &network,
            host: &host,
            pixi: pixi_asset("macos", "aarch64"),
            requirements: None,
            report: &report,
        }
        .run(&normalize(&[Tool::R, Tool::Node]))
        .await
        .unwrap_err();
        assert!(error.contains("failed"), "{error}");
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|line| line == "solve failed"));
        // Node comes after R and never started.
        assert_eq!(host.commands.lock().unwrap().len(), 1);
        assert!(layout.installed().is_empty());
        std::fs::remove_dir_all(app_data).unwrap();
    }

    #[test]
    fn job_log_keeps_only_recent_lines() {
        let mut status = ToolchainStatus::default();
        for index in 0..(LOG_LINES + 5) {
            push_line(&mut status, index.to_string());
        }
        assert_eq!(status.log.len(), LOG_LINES);
        assert_eq!(status.log[0], "5");
    }
}
