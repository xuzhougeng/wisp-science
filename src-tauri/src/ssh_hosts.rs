//! SSH host registry, validated connection snapshots, and tauri commands.
//!
//! Passwords are never stored in SQLite: they live in the OS keyring under
//! `ssh_password:{alias}` and are injected into OpenSSH via SSH_ASKPASS for
//! non-interactive managed tools (probe/run/runtime/files).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;
use wisp_store::secrets::Secret;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthMethod {
    #[default]
    Key,
    Password,
}

impl SshAuthMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Password => "password",
        }
    }

    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim).unwrap_or_default() {
            "password" => Self::Password,
            _ => Self::Key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SshHost {
    pub alias: String,
    /// Real address (IP or domain) for manually created hosts. When absent,
    /// the alias is the connection target and ~/.ssh/config resolves it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// `key` (default) or `password`. Persisted in settings JSON only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    /// Computed on read from the keyring; never part of the persisted JSON.
    #[serde(default, skip_serializing)]
    pub has_password: bool,
    /// Write-only: accepted from the UI to set/update the keyring secret.
    /// Never returned by list APIs.
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConnection {
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default)]
    pub auth_method: SshAuthMethod,
}

fn password_secret_name(alias: &str) -> String {
    format!("ssh_password:{alias}")
}

fn password_get(alias: &str) -> Option<String> {
    Secret::get(&password_secret_name(alias))
        .ok()
        .filter(|value| !value.is_empty())
}

fn password_set(alias: &str, password: &str) -> Result<(), String> {
    Secret::set(&password_secret_name(alias), password).map_err(|e| e.to_string())
}

fn password_delete(alias: &str) -> Result<(), String> {
    match Secret::delete(&password_secret_name(alias)) {
        Ok(()) => Ok(()),
        // Missing is fine when clearing.
        Err(_) => Ok(()),
    }
}

fn decorate_host(mut host: SshHost) -> SshHost {
    host.has_password = password_get(&host.alias).is_some();
    host.password = None;
    host
}

impl SshConnection {
    pub fn from_execution_context(context: &wisp_store::ExecutionContext) -> Result<Self, String> {
        if context.kind != wisp_store::ExecutionContextKind::Ssh {
            return Err(format!("Execution context is not SSH: {}", context.id));
        }
        let id_alias = context
            .id
            .strip_prefix("ssh:")
            .ok_or_else(|| format!("Invalid SSH execution context id: {}", context.id))?;
        let config: serde_json::Value = serde_json::from_str(&context.config_json)
            .map_err(|e| format!("Invalid SSH context config: {e}"))?;
        if !config.is_object() {
            return Err("SSH context config must be a JSON object".into());
        }
        let alias =
            optional_config_string(&config, "alias")?.unwrap_or_else(|| id_alias.to_string());
        let connection = Self {
            alias,
            host_name: optional_config_string(&config, "host_name")?,
            user: optional_config_string(&config, "user")?,
            port: optional_config_port(&config)?,
            identity_file: optional_config_string(&config, "identity_file")?,
            auth_method: SshAuthMethod::parse(
                optional_config_string(&config, "auth_method")?.as_deref(),
            ),
        };
        connection.validate()?;
        Ok(connection)
    }

    fn from_host(host: &SshHost) -> Result<Self, String> {
        let connection = Self {
            alias: host.alias.clone(),
            host_name: host
                .host_name
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            user: host.user.clone(),
            port: host.port,
            identity_file: host.identity_file.clone(),
            auth_method: SshAuthMethod::parse(host.auth_method.as_deref()),
        };
        connection.validate()?;
        Ok(connection)
    }

    pub fn target(&self) -> Result<String, String> {
        self.validate()?;
        let host = self.host_name.as_deref().unwrap_or(&self.alias);
        Ok(match &self.user {
            Some(user) => format!("{user}@{host}"),
            None => host.to_string(),
        })
    }

    pub fn uses_password(&self) -> bool {
        self.auth_method == SshAuthMethod::Password
    }

    pub fn ssh_args(&self) -> Result<Vec<String>, String> {
        let mut args = if self.uses_password() {
            password_option_args()
        } else {
            common_option_args()
        };
        args.push("-T".into());
        if let Some(port) = self.port {
            args.extend(["-p".into(), port.to_string()]);
        }
        if !self.uses_password() {
            push_batch_identity_args(&mut args, self.identity_file.as_deref());
        }
        args.push(self.target()?);
        Ok(args)
    }

    /// Arguments for a user-driven interactive terminal. Unlike probes and
    /// Runs, this deliberately leaves BatchMode disabled so OpenSSH can show
    /// host-key, password, and keyboard-interactive prompts in the PTY.
    /// When password auth is configured, askpass env still supplies the stored
    /// password so the user is not forced to retype it.
    pub fn interactive_ssh_args(&self) -> Result<Vec<String>, String> {
        self.validate()?;
        let mut args = vec!["-tt".into()];
        if self.uses_password() {
            args.extend([
                "-o".into(),
                "PreferredAuthentications=password,keyboard-interactive".into(),
                "-o".into(),
                "PubkeyAuthentication=no".into(),
                "-o".into(),
                "NumberOfPasswordPrompts=1".into(),
            ]);
        }
        if let Some(port) = self.port {
            args.extend(["-p".into(), port.to_string()]);
        }
        if !self.uses_password() {
            push_interactive_identity_args(&mut args, self.identity_file.as_deref());
        }
        args.push(self.target()?);
        Ok(args)
    }

    pub fn scp_option_args(&self) -> Result<Vec<String>, String> {
        self.validate()?;
        let mut args = if self.uses_password() {
            password_option_args()
        } else {
            common_option_args()
        };
        if let Some(port) = self.port {
            args.extend(["-P".into(), port.to_string()]);
        }
        if !self.uses_password() {
            push_batch_identity_args(&mut args, self.identity_file.as_deref());
        }
        Ok(args)
    }

    /// Fail before spawning when a configured identity file is missing, or
    /// when password auth is selected but no password is stored.
    pub fn assert_ready_to_connect(&self) -> Result<(), String> {
        self.validate()?;
        match self.auth_method {
            SshAuthMethod::Password => {
                if password_get(&self.alias).is_none() {
                    return Err(format!(
                        "SSH password is not set for `{}`. Open host settings, choose password \
                         authentication, and save a password (stored in the OS keyring). \
                         Do not put passwords in shell commands.",
                        self.alias
                    ));
                }
            }
            SshAuthMethod::Key => {
                if let Some(identity_file) = &self.identity_file {
                    ensure_identity_file_accessible(identity_file)?;
                }
            }
        }
        Ok(())
    }

    /// Build env vars that force OpenSSH to read a one-shot password via ASKPASS.
    /// Caller must run `cleanup_password_auth_env` after the process exits.
    pub fn password_auth_env(&self) -> Result<Vec<(String, String)>, String> {
        if !self.uses_password() {
            return Ok(Vec::new());
        }
        let password = password_get(&self.alias).ok_or_else(|| {
            format!(
                "SSH password is not set for `{}`. Save it in host settings first.",
                self.alias
            )
        })?;
        build_password_askpass_env(&password)
    }

    fn validate(&self) -> Result<(), String> {
        validate_connection_name("SSH alias", &self.alias)?;
        if let Some(host_name) = &self.host_name {
            validate_connection_name("SSH host address", host_name)?;
        }
        if let Some(user) = &self.user {
            validate_connection_name("SSH user", user)?;
        }
        if self.port == Some(0) {
            return Err("SSH port must be greater than zero".into());
        }
        if let Some(identity_file) = &self.identity_file {
            if identity_file.is_empty() {
                return Err("SSH identity file must not be empty".into());
            }
            if identity_file.chars().any(char::is_control) {
                return Err("SSH identity file must not contain control characters".into());
            }
        }
        if self.auth_method == SshAuthMethod::Password && self.user.is_none() {
            // user can come from ~/.ssh/config for the alias; warn only in UI.
        }
        Ok(())
    }
}

const PASSFILE_ENV: &str = "WISP_SSH_PASSFILE";
const ASKPASS_ENV_MARKER: &str = "WISP_SSH_ASKPASS_SCRIPT";

fn password_option_args() -> Vec<String> {
    vec![
        // Not BatchMode: password auth requires prompts, supplied by ASKPASS.
        "-o".into(),
        "BatchMode=no".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        "PreferredAuthentications=password,keyboard-interactive".into(),
        "-o".into(),
        "PubkeyAuthentication=no".into(),
        "-o".into(),
        "NumberOfPasswordPrompts=1".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
    ]
}

fn build_password_askpass_env(password: &str) -> Result<Vec<(String, String)>, String> {
    let dir = std::env::temp_dir().join(format!("wisp-ssh-askpass-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create askpass dir: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let pass_path = dir.join("pass");
    let askpass_path = dir.join(if cfg!(windows) {
        "askpass.cmd"
    } else {
        "askpass.sh"
    });
    std::fs::write(&pass_path, password.as_bytes()).map_err(|e| format!("write passfile: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&pass_path, std::fs::Permissions::from_mode(0o600));
    }
    let script = if cfg!(windows) {
        format!(
            "@echo off\r\ntype \"{}\"\r\n",
            pass_path.to_string_lossy().replace('"', "")
        )
    } else {
        format!(
            "#!/bin/sh\nexec cat {}\n",
            shell_single_quote_path(&pass_path)
        )
    };
    std::fs::write(&askpass_path, script).map_err(|e| format!("write askpass: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&askpass_path, std::fs::Permissions::from_mode(0o700));
    }
    Ok(vec![
        (
            "SSH_ASKPASS".into(),
            askpass_path.to_string_lossy().into_owned(),
        ),
        ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
        // Older OpenSSH only enables ASKPASS when DISPLAY is set.
        ("DISPLAY".into(), "wisp-ssh-askpass".into()),
        (
            PASSFILE_ENV.into(),
            pass_path.to_string_lossy().into_owned(),
        ),
        (
            ASKPASS_ENV_MARKER.into(),
            askpass_path.to_string_lossy().into_owned(),
        ),
    ])
}

fn shell_single_quote_path(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

/// Remove one-shot passfile/askpass created for a managed SSH process.
pub fn cleanup_password_auth_env(envs: &[(String, String)]) {
    let mut paths = Vec::new();
    for (key, value) in envs {
        if key == PASSFILE_ENV || key == ASKPASS_ENV_MARKER || key == "SSH_ASKPASS" {
            paths.push(PathBuf::from(value));
        }
    }
    for path in &paths {
        let _ = std::fs::remove_file(path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

/// Attach password ASKPASS env for a connection when needed.
pub fn auth_envs_for_connection(
    connection: &SshConnection,
) -> Result<Vec<(String, String)>, String> {
    connection.password_auth_env()
}

fn expand_user_path(path: &str) -> std::path::PathBuf {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if let Some(rest) = path.strip_prefix("~\\") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    std::path::PathBuf::from(path)
}

fn ensure_identity_file_accessible(identity_file: &str) -> Result<(), String> {
    ensure_identity_path_accessible(identity_file)
}

/// Public so runners can re-check `-i` paths without rebuilding the connection.
pub fn ensure_identity_path_accessible(identity_file: &str) -> Result<(), String> {
    let path = expand_user_path(identity_file);
    if path.is_file() {
        return Ok(());
    }
    Err(format!(
        "SSH identity file is not accessible: {identity_file} (resolved {}). \
         Fix the IdentityFile path in the SSH host settings before connecting. \
         Do not retry with shell `ssh` or alternate `-i` keys.",
        path.display()
    ))
}

pub const SSH_NOT_CONFIRMED_MARKER: &str = "SSH connectivity is not confirmed";

/// Gate every managed SSH use: known-good probe, open circuit breaker, and a
/// resolvable identity file. Agent tools must call this before spawning SSH so
/// they only use the configured `SshConnection` when the host is known reachable.
pub fn require_managed_ssh_ready(ctx: &wisp_store::ExecutionContext) -> Result<(), String> {
    if ctx.kind != wisp_store::ExecutionContextKind::Ssh {
        return Ok(());
    }
    crate::ssh_guard::assert_allowed(&ctx.id)?;
    let connection = SshConnection::from_execution_context(ctx)?;
    if let Err(error) = connection.assert_ready_to_connect() {
        crate::ssh_guard::record_failure(&ctx.id, &error);
        return Err(error);
    }
    match ctx.last_probe_status.as_deref() {
        Some("ok") => Ok(()),
        Some("error") => {
            let detail = ctx
                .last_probe_error
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("probe failed");
            Err(format!(
                "{SSH_NOT_CONFIRMED_MARKER} for `{}`: last probe failed ({detail}). \
                 Check the SSH server (network, firewall/IP unlock, key or password settings), then run Probe \
                 on this environment. Free-form shell `ssh` is disabled; agent access uses only \
                 the configured host settings.",
                ctx.id
            ))
        }
        _ => Err(format!(
            "{SSH_NOT_CONFIRMED_MARKER} for `{}`: no successful probe yet. \
             Probe this environment first so Wisp knows the server is reachable with the \
             configured settings. Free-form shell `ssh` is disabled.",
            ctx.id
        )),
    }
}

fn common_option_args() -> Vec<String> {
    vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
    ]
}

fn push_batch_identity_args(args: &mut Vec<String>, identity_file: Option<&str>) {
    args.extend(["-o".into(), "IdentitiesOnly=yes".into()]);
    if let Some(identity_file) = identity_file {
        args.extend(["-i".into(), identity_file.into()]);
    }
}

fn push_interactive_identity_args(args: &mut Vec<String>, identity_file: Option<&str>) {
    if let Some(identity_file) = identity_file {
        args.extend([
            "-o".into(),
            "IdentitiesOnly=yes".into(),
            "-i".into(),
            identity_file.into(),
        ]);
    }
}

fn validate_connection_name(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.starts_with('-') {
        return Err(format!("{label} must not start with '-'"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(format!(
            "{label} may contain only ASCII letters, digits, '.', '_' and '-'"
        ));
    }
    Ok(())
}

fn optional_config_string(config: &serde_json::Value, key: &str) -> Result<Option<String>, String> {
    match config.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("SSH context field '{key}' must be a string")),
    }
}

fn optional_config_port(config: &serde_json::Value) -> Result<Option<u16>, String> {
    match config.get("port") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .map(Some)
            .ok_or_else(|| "SSH context field 'port' must be an integer from 1 to 65535".into()),
    }
}

pub fn upsert_host(mut hosts: Vec<SshHost>, host: SshHost) -> Vec<SshHost> {
    if let Some(existing) = hosts.iter_mut().find(|h| h.alias == host.alias) {
        *existing = host;
    } else {
        hosts.push(host);
    }
    hosts
}

pub fn remove_host(mut hosts: Vec<SshHost>, alias: &str) -> Vec<SshHost> {
    hosts.retain(|h| h.alias != alias);
    hosts
}

/// Parse `Host` aliases from an ~/.ssh/config body. Skips wildcard patterns
/// (`*`, `?` — those are match rules, not connectable hosts) and dedupes,
/// preserving first-seen order.
pub fn parse_ssh_config_aliases(config: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let Some(kw) = parts.next() else { continue };
        if !kw.eq_ignore_ascii_case("host") {
            continue;
        }
        for alias in parts {
            if alias.contains('*')
                || alias.contains('?')
                || validate_connection_name("SSH alias", alias).is_err()
            {
                continue;
            }
            if !out.iter().any(|a| a == alias) {
                out.push(alias.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
pub fn render_hosts_section(hosts: &[SshHost]) -> Option<String> {
    if hosts.is_empty() {
        return None;
    }
    let mut s = String::from(
        "## Compute hosts\n\n\
The user has these SSH hosts available. Submit remote discovery, real work, and all long-running commands with \
`run_in_context` using the `ssh:<alias>` context. Do not use shell `sleep`, \
`ssh ... ps`, `nohup`, background `&`, or polling loops to monitor work. After \
submission, observe or cancel it through the Runs control plane. Remote paths \
live on the host, not on this machine. To watch a submitted Run or wait for its \
result, call `monitor_run` exactly once with its Run id. Wisp shows a live card, \
suspends the tool, and resumes after completion without repeated `get_run` calls.\n\n",
    );
    for h in hosts {
        let mut conn = String::new();
        if let Some(u) = &h.user {
            conn.push_str(u);
            conn.push('@');
        }
        conn.push_str(&h.alias);
        if let Some(p) = h.port {
            conn.push_str(&format!(":{p}"));
        }
        s.push_str(&format!("- {} — {}", h.alias, conn));
        if let Some(n) = h.notes.as_deref().filter(|n| !n.trim().is_empty()) {
            s.push_str(&format!(" — {n}"));
        }
        s.push('\n');
    }
    Some(s)
}

pub fn render_contexts_section(contexts: &[wisp_store::ExecutionContext]) -> Option<String> {
    let contexts: Vec<_> = contexts
        .iter()
        .filter(|c| {
            matches!(
                c.kind,
                wisp_store::ExecutionContextKind::Ssh | wisp_store::ExecutionContextKind::Wsl
            )
        })
        .collect();
    if contexts.is_empty() {
        return None;
    }
    let mut s = String::from(
        "## Compute contexts\n\n\
The user selected these execution contexts for the current conversation. Prefer them over local \
compute when they fit the task. Submit remote discovery, real work, and all long-running commands with \
`run_in_context` using the context id. Do not use shell `sleep`, `ssh ... ps`, \
`nohup`, background `&`, or polling loops to monitor work. After submission, \
observe or cancel it through the Runs control plane. Remote paths are not local \
paths. To watch a submitted Run or wait for its result, call `monitor_run` \
exactly once instead of repeatedly calling `get_run`. For persistent interactive analysis, call `python` or `r` with the \
matching `context_id`; omitting it selects `local`. Interpreter paths come from \
the execution context's saved settings or probe result, not shell environment \
changes. Use `set_runtime_interpreter` when the user provides a different \
Python or R executable.\n\
**SSH connectivity policy:** remote work is only allowed after a successful Probe \
on the registered environment. Always use `run_in_context`, `python`, `r`, \
`configure_ssh_trust`, or `transfer_between_contexts` with \
the matching `context_id` so Wisp uses the configured alias/user/port/identity \
exactly. Free-form shell `ssh`/`scp` is disabled. If connectivity is unknown or \
failed: STOP, tell the user to check the SSH server and Probe again — do not invent \
`ssh -i`, ports, or `StrictHostKeyChecking` options. After one failure, do not \
retry; repeated attempts look like SSH brute force and can ban the user's IP.\n\n",
    );
    for ctx in contexts {
        let cfg: serde_json::Value = serde_json::from_str(&ctx.config_json).unwrap_or_default();
        match ctx.kind {
            wisp_store::ExecutionContextKind::Ssh => {
                let (conn, port) = match SshConnection::from_execution_context(ctx) {
                    Ok(connection) => (
                        connection
                            .target()
                            .unwrap_or_else(|error| format!("invalid SSH configuration: {error}")),
                        connection
                            .port
                            .map(|port| format!(":{port}"))
                            .unwrap_or_default(),
                    ),
                    Err(error) => (format!("invalid SSH configuration: {error}"), String::new()),
                };
                s.push_str(&format!("- {} — {conn}{port}", ctx.id));
                if let Some(notes) = cfg
                    .get("notes")
                    .and_then(|v| v.as_str())
                    .filter(|n| !n.trim().is_empty())
                {
                    s.push_str(&format!(" — {notes}"));
                }
                if ctx.last_probe_status.as_deref() == Some("error") {
                    let detail = ctx
                        .last_probe_error
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("probe failed");
                    s.push_str(&format!(
                        " — CONNECTIVITY: last probe failed ({detail}). Do not attempt SSH to this host until the user re-probes successfully"
                    ));
                }
            }
            wisp_store::ExecutionContextKind::Wsl => {
                let distro = cfg
                    .get("distro")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| ctx.id.strip_prefix("wsl:").unwrap_or(&ctx.id));
                s.push_str(&format!(
                    "- {} — WSL distro `{distro}` — Linux execution context; paths may differ from Windows paths",
                    ctx.id
                ));
            }
            wisp_store::ExecutionContextKind::Local => {}
        }
        if let Some(summary) = capability_summary(&ctx.capabilities_json) {
            s.push_str(&format!(" — capabilities: {summary}"));
        }
        let capabilities: serde_json::Value =
            serde_json::from_str(&ctx.capabilities_json).unwrap_or_default();
        if capabilities
            .get("gpu_summary")
            .is_none_or(serde_json::Value::is_null)
        {
            s.push_str(" — GPU: none; do not plan GPU/CUDA work");
        }
        if capabilities
            .get("privilege")
            .and_then(|value| value.as_str())
            == Some("unprivileged")
        {
            s.push_str(" — privilege: unprivileged; do not use sudo or system package managers");
        }
        if let Some((_, reason)) = crate::ssh_guard::blocked_contexts()
            .into_iter()
            .find(|(id, _)| id == &ctx.id)
        {
            s.push_str(&format!(
                " — CONNECTIVITY GATE OPEN: blocked after prior failure ({reason}). Do not attempt any SSH to this host"
            ));
        }
        s.push('\n');
    }
    Some(s)
}

/// Remove the global compute section written into system prompts by versions
/// that treated resource selection as an ExecutionContext setting. The current
/// session-specific section is injected at turn time instead.
pub(crate) fn strip_legacy_compute_section(prompt: &mut String) {
    let Some(section_start) = prompt.find("## Compute contexts\n") else {
        return;
    };
    let Some(relative_end) = prompt[section_start..].find("\n\n## User Rules\n") else {
        return;
    };
    let start = section_start.saturating_sub(
        prompt[..section_start]
            .ends_with("\n\n")
            .then_some(2)
            .unwrap_or(0),
    );
    prompt.replace_range(start..section_start + relative_end, "");
}

fn capability_summary(capabilities_json: &str) -> Option<String> {
    let caps: serde_json::Value = serde_json::from_str(capabilities_json).ok()?;
    let mut parts = Vec::new();
    match (
        caps.get("os").and_then(|v| v.as_str()),
        caps.get("arch").and_then(|v| v.as_str()),
    ) {
        (Some(os), Some(arch)) => parts.push(format!("{os}/{arch}")),
        (Some(os), None) => parts.push(os.into()),
        _ => {}
    }
    if let Some(gpu) = caps
        .get("gpu_summary")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        parts.push(gpu.into());
    }
    if let Some(scheduler) = caps
        .get("scheduler")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        parts.push(format!("scheduler: {scheduler}"));
    }
    if let Some(python) = caps
        .get("python")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        parts.push(python.into());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

const KEY: &str = "ssh_hosts";

async fn load(store: &wisp_store::Store) -> Vec<SshHost> {
    store
        .get_setting(KEY)
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

async fn save(store: &wisp_store::Store, hosts: &[SshHost]) -> Result<(), String> {
    let json = serde_json::to_string(hosts).map_err(|e| e.to_string())?;
    store
        .set_setting(KEY, &json)
        .await
        .map_err(|e| e.to_string())
}

fn ssh_context_id(alias: &str) -> Result<String, String> {
    let alias = alias.trim();
    validate_connection_name("SSH alias", alias)?;
    let id = format!("ssh:{alias}");
    wisp_store::ExecutionContextKind::from_id(&id).map_err(|e| e.to_string())?;
    Ok(id)
}

fn ssh_context_config_json(host: &SshHost) -> Result<String, String> {
    let mut cfg = serde_json::Map::new();
    cfg.insert(
        "alias".into(),
        serde_json::Value::String(host.alias.trim().into()),
    );
    if let Some(host_name) = host.host_name.as_deref().filter(|s| !s.trim().is_empty()) {
        cfg.insert(
            "host_name".into(),
            serde_json::Value::String(host_name.trim().into()),
        );
    }
    if let Some(user) = host.user.as_deref().filter(|s| !s.trim().is_empty()) {
        cfg.insert("user".into(), serde_json::Value::String(user.trim().into()));
    }
    if let Some(port) = host.port {
        cfg.insert("port".into(), serde_json::Value::from(port));
    }
    let auth = SshAuthMethod::parse(host.auth_method.as_deref());
    cfg.insert(
        "auth_method".into(),
        serde_json::Value::String(auth.as_str().into()),
    );
    if auth == SshAuthMethod::Key {
        if let Some(identity_file) = host
            .identity_file
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            cfg.insert(
                "identity_file".into(),
                serde_json::Value::String(identity_file.trim().into()),
            );
        }
    }
    if let Some(notes) = host.notes.as_deref().filter(|s| !s.trim().is_empty()) {
        cfg.insert(
            "notes".into(),
            serde_json::Value::String(notes.trim().into()),
        );
    }
    serde_json::to_string(&serde_json::Value::Object(cfg)).map_err(|e| e.to_string())
}

fn persistable_host(host: &SshHost) -> SshHost {
    SshHost {
        alias: host.alias.trim().into(),
        host_name: host
            .host_name
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        user: host
            .user
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        port: host.port,
        identity_file: if SshAuthMethod::parse(host.auth_method.as_deref()) == SshAuthMethod::Key {
            host.identity_file
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        } else {
            None
        },
        notes: host
            .notes
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        auth_method: Some(
            SshAuthMethod::parse(host.auth_method.as_deref())
                .as_str()
                .into(),
        ),
        has_password: false,
        password: None,
    }
}

fn apply_host_password(host: &SshHost) -> Result<(), String> {
    let alias = host.alias.trim();
    let auth = SshAuthMethod::parse(host.auth_method.as_deref());
    match auth {
        SshAuthMethod::Password => {
            if let Some(password) = host.password.as_deref() {
                let password = password.trim();
                if password.is_empty() {
                    // Empty means "leave existing password unchanged" when already set.
                    if password_get(alias).is_none() {
                        return Err("Password authentication requires a non-empty password".into());
                    }
                } else {
                    password_set(alias, password)?;
                }
            } else if password_get(alias).is_none() {
                return Err("Password authentication requires a password".into());
            }
        }
        SshAuthMethod::Key => {
            // Switching back to key auth clears any stored password.
            password_delete(alias)?;
        }
    }
    Ok(())
}

async fn upsert_context_for_host(store: &wisp_store::Store, host: &SshHost) -> Result<(), String> {
    SshConnection::from_host(host)?;
    let id = ssh_context_id(&host.alias)?;
    let now = chrono::Utc::now().timestamp();
    let mut ctx = match store
        .get_execution_context(&id)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(ctx) => ctx,
        None => {
            wisp_store::ExecutionContext::new(&id, host.alias.trim()).map_err(|e| e.to_string())?
        }
    };
    ctx.kind = wisp_store::ExecutionContextKind::Ssh;
    ctx.label = host.alias.trim().into();
    ctx.config_json = crate::runtime_launcher::preserve_interpreter_config(
        &ctx.config_json,
        &ssh_context_config_json(host)?,
    )
    .map_err(|error| error.to_string())?;
    ctx.updated_at = now;
    store
        .upsert_execution_context(&ctx)
        .await
        .map_err(|e| e.to_string())
}

async fn save_and_sync_contexts(
    store: &wisp_store::Store,
    hosts: &[SshHost],
) -> Result<(), String> {
    for host in hosts {
        SshConnection::from_host(host)?;
    }
    save(store, hosts).await?;
    for host in hosts {
        upsert_context_for_host(store, host).await?;
    }
    Ok(())
}

async fn remove_context_for_alias(store: &wisp_store::Store, alias: &str) -> Result<(), String> {
    let id = ssh_context_id(alias)?;
    store
        .delete_execution_context(&id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn stored_compute_section(store: &wisp_store::Store, frame_id: &str) -> Option<String> {
    let hosts = load(store).await;
    for host in &hosts {
        if let Err(e) = upsert_context_for_host(store, host).await {
            tracing::warn!("sync SSH host to execution context failed: {e}");
        }
    }
    let selected = match store.list_session_execution_context_ids(frame_id).await {
        Ok(ids) => ids.into_iter().collect::<std::collections::HashSet<_>>(),
        Err(e) => {
            tracing::warn!("load session execution contexts failed: {e}");
            return None;
        }
    };
    match store.list_execution_contexts().await {
        Ok(contexts) => render_contexts_section(
            &contexts
                .into_iter()
                .filter(|context| selected.contains(&context.id))
                .collect::<Vec<_>>(),
        ),
        Err(e) => {
            tracing::warn!("load execution contexts failed: {e}");
            None
        }
    }
}

#[tauri::command]
pub async fn list_ssh_hosts(state: State<'_, crate::AppState>) -> Result<Vec<SshHost>, String> {
    Ok(load(&state.store)
        .await
        .into_iter()
        .map(decorate_host)
        .collect())
}

#[tauri::command]
pub async fn list_session_execution_context_ids(
    state: State<'_, crate::AppState>,
    session_id: String,
) -> Result<Vec<String>, String> {
    state
        .store
        .list_session_execution_context_ids(&session_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_session_execution_context_enabled(
    state: State<'_, crate::AppState>,
    session_id: String,
    context_id: String,
    enabled: bool,
) -> Result<Vec<String>, String> {
    state
        .store
        .set_session_execution_context_enabled(&session_id, &context_id, enabled)
        .await
        .map_err(|error| error.to_string())?;
    state
        .store
        .list_session_execution_context_ids(&session_id)
        .await
        .map_err(|error| error.to_string())
}

/// One-shot connectivity check for the host editor, using the form's current
/// (possibly unsaved) values. Deliberately bypasses the ssh_guard gate (this
/// is the user's diagnostic tool) and the master pool (a fresh connection is
/// the point); a success clears any guard block for the alias.
#[tauri::command]
pub async fn test_ssh_connection(host: SshHost) -> Result<(), String> {
    let connection = SshConnection::from_host(&host)?;
    let envs = if connection.uses_password() {
        match host
            .password
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            Some(password) => build_password_askpass_env(password)?,
            None => connection.password_auth_env()?,
        }
    } else {
        connection.assert_ready_to_connect()?;
        Vec::new()
    };
    let mut args = connection.ssh_args()?;
    args.push("echo __WISP_SSH_OK__".into());
    let mut cmd = tokio::process::Command::new("ssh");
    cmd.args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if !envs.is_empty() {
        cmd.envs(envs.iter().cloned());
    }
    wisp_tools::process::hide_console_async(&mut cmd);
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output()).await;
    cleanup_password_auth_env(&envs);
    let output = result
        .map_err(|_| "SSH connection test timed out after 30s".to_string())?
        .map_err(|e| format!("failed to run ssh: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.success() && stdout.contains("__WISP_SSH_OK__") {
        crate::ssh_guard::record_success(&format!("ssh:{}", connection.alias));
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        Err(if detail.is_empty() {
            format!(
                "ssh exited with status {}",
                output.status.code().unwrap_or(-1)
            )
        } else {
            detail
        })
    }
}

#[tauri::command]
pub async fn add_ssh_host(
    state: State<'_, crate::AppState>,
    host: SshHost,
) -> Result<Vec<SshHost>, String> {
    SshConnection::from_host(&host)?;
    apply_host_password(&host)?;
    let host = persistable_host(&host);
    let hosts = upsert_host(load(&state.store).await, host);
    save_and_sync_contexts(&state.store, &hosts).await?;
    Ok(hosts.into_iter().map(decorate_host).collect())
}

#[tauri::command]
pub async fn remove_ssh_host(
    state: State<'_, crate::AppState>,
    alias: String,
) -> Result<Vec<SshHost>, String> {
    let hosts = remove_host(load(&state.store).await, &alias);
    save(&state.store, &hosts).await?;
    remove_context_for_alias(&state.store, &alias).await?;
    let _ = password_delete(&alias);
    Ok(hosts.into_iter().map(decorate_host).collect())
}

/// Merge every connectable `Host` alias from ~/.ssh/config into the registry,
/// keeping hosts the user already configured (notes etc.) untouched (#56/#67).
pub fn merge_config_aliases(mut hosts: Vec<SshHost>, aliases: Vec<String>) -> Vec<SshHost> {
    for alias in aliases {
        if !hosts.iter().any(|h| h.alias == alias) {
            hosts.push(SshHost {
                alias,
                host_name: None,
                user: None,
                port: None,
                identity_file: None,
                notes: None,
                auth_method: None,
                has_password: false,
                password: None,
            });
        }
    }
    hosts
}

fn list_ssh_config_aliases() -> Vec<String> {
    let text = dirs::home_dir()
        .map(|h| h.join(".ssh").join("config"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    parse_ssh_config_aliases(&text)
}

#[tauri::command]
pub async fn import_ssh_config_hosts(
    state: State<'_, crate::AppState>,
) -> Result<Vec<SshHost>, String> {
    let aliases = list_ssh_config_aliases();
    let hosts = merge_config_aliases(load(&state.store).await, aliases);
    save_and_sync_contexts(&state.store, &hosts).await?;
    Ok(hosts.into_iter().map(decorate_host).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(alias: &str, notes: Option<&str>) -> SshHost {
        SshHost {
            alias: alias.into(),
            host_name: None,
            user: None,
            port: None,
            identity_file: None,
            notes: notes.map(Into::into),
            auth_method: None,
            has_password: false,
            password: None,
        }
    }

    #[test]
    fn missing_identity_file_fails_before_connect() {
        let connection = SshConnection {
            alias: "gpu".into(),
            host_name: None,
            user: None,
            port: None,
            identity_file: Some("/definitely/missing/wisp-test-key".into()),
            auth_method: SshAuthMethod::Key,
        };
        let err = connection.assert_ready_to_connect().unwrap_err();
        assert!(err.contains("identity file is not accessible"), "{err}");
        assert!(err.contains("Do not retry"), "{err}");
    }

    #[test]
    fn managed_ssh_requires_successful_probe() {
        let mut ctx = wisp_store::ExecutionContext::new("ssh:lab", "lab").unwrap();
        ctx.config_json = serde_json::json!({ "alias": "lab" }).to_string();
        let unknown = require_managed_ssh_ready(&ctx).unwrap_err();
        assert!(unknown.contains(SSH_NOT_CONFIRMED_MARKER), "{unknown}");
        assert!(unknown.contains("no successful probe"), "{unknown}");

        ctx.last_probe_status = Some("error".into());
        ctx.last_probe_error = Some("Connection timed out".into());
        let failed = require_managed_ssh_ready(&ctx).unwrap_err();
        assert!(failed.contains(SSH_NOT_CONFIRMED_MARKER), "{failed}");
        assert!(failed.contains("Connection timed out"), "{failed}");

        ctx.last_probe_status = Some("ok".into());
        ctx.last_probe_error = None;
        assert!(require_managed_ssh_ready(&ctx).is_ok());
    }

    #[test]
    fn upsert_adds_new_and_replaces_by_alias_in_place() {
        let list = vec![host("a", Some("first")), host("b", None)];
        let added = upsert_host(list, host("c", None));
        assert_eq!(
            added.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );

        let replaced = upsert_host(added, host("a", Some("second")));
        assert_eq!(
            replaced
                .iter()
                .map(|h| h.alias.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(replaced[0].notes.as_deref(), Some("second"));
    }

    #[test]
    fn remove_drops_matching_alias() {
        let list = vec![host("a", None), host("b", None)];
        let out = remove_host(list, "a");
        assert_eq!(
            out.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(),
            ["b"]
        );
    }

    #[test]
    fn parses_host_aliases_skips_wildcards_and_dedupes() {
        let cfg = "\
Host gpu-box lab-gpu
    HostName 10.0.0.5
    User alice

Host *
    ForwardAgent yes

Host biowulf
    HostName biowulf.nih.gov

Host gpu-box
    Port 2222

Host -unsafe bad/name !negated
";
        assert_eq!(
            parse_ssh_config_aliases(cfg),
            vec![
                "gpu-box".to_string(),
                "lab-gpu".to_string(),
                "biowulf".to_string()
            ]
        );
    }

    #[test]
    fn merge_config_aliases_adds_new_keeps_existing() {
        let existing = vec![host("gpu-box", Some("slurm cluster"))];
        let merged = merge_config_aliases(existing, vec!["gpu-box".into(), "biowulf".into()]);
        assert_eq!(
            merged.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(),
            ["gpu-box", "biowulf"]
        );
        // The pre-existing entry (with its notes) must not be overwritten.
        assert_eq!(merged[0].notes.as_deref(), Some("slurm cluster"));
        assert!(merged[1].notes.is_none());
    }

    #[test]
    fn render_empty_is_none() {
        assert!(render_hosts_section(&[]).is_none());
    }

    #[test]
    fn render_lists_conn_and_notes() {
        let hosts = vec![
            SshHost {
                alias: "gpu".into(),
                host_name: None,
                user: Some("alice".into()),
                port: Some(2222),
                identity_file: None,
                notes: Some("slurm; sbatch".into()),
                auth_method: None,
                has_password: false,
                password: None,
            },
            host("plain", None),
        ];
        let s = render_hosts_section(&hosts).unwrap();
        assert!(s.starts_with("## Compute hosts"), "{s}");
        assert!(
            s.contains("`run_in_context`"),
            "must direct real work to the run manager:\n{s}"
        );
        assert!(
            s.contains("Runs control plane"),
            "runs guidance missing:\n{s}"
        );
        assert!(s.contains("`nohup`"), "shell prohibition missing:\n{s}");
        assert!(s.contains("alice@gpu:2222"), "conn missing:\n{s}");
        assert!(s.contains("slurm; sbatch"), "notes missing:\n{s}");
        assert!(s.contains("- plain"), "bare alias missing:\n{s}");
    }

    #[tokio::test]
    async fn imported_aliases_create_ssh_execution_contexts() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_ssh_contexts_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();

        let hosts = merge_config_aliases(Vec::new(), vec!["gpu-box".into(), "biowulf".into()]);
        save_and_sync_contexts(&store, &hosts).await.unwrap();

        let contexts = store.list_execution_contexts().await.unwrap();
        assert_eq!(
            contexts
                .iter()
                .filter(|context| context.kind == wisp_store::ExecutionContextKind::Ssh)
                .map(|context| context.id.as_str())
                .collect::<Vec<_>>(),
            ["ssh:biowulf", "ssh:gpu-box"]
        );
        assert_eq!(
            store
                .get_execution_context("ssh:gpu-box")
                .await
                .unwrap()
                .unwrap()
                .kind,
            wisp_store::ExecutionContextKind::Ssh
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn syncing_hosts_preserves_notes_and_probe_state() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_ssh_contexts_preserve_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();

        let hosts = merge_config_aliases(
            vec![host("gpu-box", Some("slurm cluster"))],
            vec!["gpu-box".into()],
        );
        assert_eq!(hosts[0].notes.as_deref(), Some("slurm cluster"));
        save_and_sync_contexts(&store, &hosts).await.unwrap();

        let mut probed = store
            .get_execution_context("ssh:gpu-box")
            .await
            .unwrap()
            .unwrap();
        let mut config: serde_json::Value = serde_json::from_str(&probed.config_json).unwrap();
        config["python_executable"] = "/opt/python/bin/python".into();
        config["rscript_executable"] = "/opt/R/bin/Rscript".into();
        probed.config_json = config.to_string();
        probed.capabilities_json = r#"{"gpu_summary":"A100"}"#.into();
        probed.last_probe_at = Some(456);
        probed.last_probe_status = Some("ok".into());
        store.upsert_execution_context(&probed).await.unwrap();

        save_and_sync_contexts(&store, &hosts).await.unwrap();
        let got = store
            .get_execution_context("ssh:gpu-box")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.capabilities_json, r#"{"gpu_summary":"A100"}"#);
        assert_eq!(got.last_probe_at, Some(456));
        assert_eq!(got.last_probe_status.as_deref(), Some("ok"));
        let cfg: serde_json::Value = serde_json::from_str(&got.config_json).unwrap();
        assert_eq!(cfg["alias"], "gpu-box");
        assert_eq!(cfg["notes"], "slurm cluster");
        assert_eq!(cfg["python_executable"], "/opt/python/bin/python");
        assert_eq!(cfg["rscript_executable"], "/opt/R/bin/Rscript");

        remove_context_for_alias(&store, "gpu-box").await.unwrap();
        assert!(store
            .get_execution_context("ssh:gpu-box")
            .await
            .unwrap()
            .is_none());

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn render_contexts_lists_context_ids_and_remote_path_warning() {
        let mut ctx = wisp_store::ExecutionContext::new("ssh:gpu-box", "gpu-box").unwrap();
        ctx.config_json = serde_json::json!({
            "alias": "gpu-box",
            "user": "alice",
            "port": 2222,
            "notes": "slurm; sbatch"
        })
        .to_string();

        let s = render_contexts_section(&[ctx]).unwrap();
        assert!(s.starts_with("## Compute contexts"), "{s}");
        assert!(s.contains("ssh:gpu-box"), "context id missing:\n{s}");
        assert!(s.contains("alice@gpu-box:2222"), "ssh target missing:\n{s}");
        assert!(s.contains("`run_in_context`"), "run guidance missing:\n{s}");
        assert!(
            s.contains("matching `context_id`"),
            "runtime guidance missing:\n{s}"
        );
        assert!(
            s.contains("not shell environment changes"),
            "interpreter configuration guidance missing:\n{s}"
        );
        assert!(
            s.contains("`set_runtime_interpreter`"),
            "tool guidance missing:\n{s}"
        );
        assert!(
            s.contains("Remote paths are not local paths"),
            "remote path warning missing:\n{s}"
        );
        assert!(
            s.contains("SSH connectivity policy"),
            "connectivity policy missing:\n{s}"
        );
        assert!(
            s.contains("Free-form shell `ssh`/`scp` is disabled"),
            "shell ssh ban missing:\n{s}"
        );
        assert!(
            s.contains("`configure_ssh_trust`") && s.contains("`transfer_between_contexts`"),
            "structured transfer guidance missing:\n{s}"
        );
        assert!(
            s.contains("successful Probe"),
            "probe-first guidance missing:\n{s}"
        );
        assert!(s.contains("slurm; sbatch"), "notes missing:\n{s}");
    }

    #[test]
    fn render_contexts_flags_failed_probe_connectivity() {
        let mut ctx = wisp_store::ExecutionContext::new("ssh:down", "down").unwrap();
        ctx.config_json = serde_json::json!({ "alias": "down" }).to_string();
        ctx.last_probe_status = Some("error".into());
        ctx.last_probe_error = Some("Connection timed out".into());
        let s = render_contexts_section(&[ctx]).unwrap();
        assert!(s.contains("CONNECTIVITY: last probe failed"), "{s}");
        assert!(s.contains("Connection timed out"), "{s}");
        assert!(s.contains("Do not attempt SSH"), "{s}");
    }

    #[test]
    fn render_contexts_excludes_local_and_includes_selected_capabilities() {
        let local = wisp_store::ExecutionContext::new("local", "Local").unwrap();
        let mut selected = wisp_store::ExecutionContext::new("ssh:on", "on").unwrap();
        selected.config_json = serde_json::json!({ "alias": "on" }).to_string();
        selected.capabilities_json = serde_json::json!({
            "probe_skill": "probe-compute-environment",
            "gpu_summary": null,
            "privilege": "unprivileged"
        })
        .to_string();

        let rendered = render_contexts_section(&[local, selected]).unwrap();
        assert!(rendered.contains("ssh:on"));
        assert!(!rendered.contains("- local"));
        assert!(rendered.contains("current conversation"));
        assert!(rendered.contains("GPU: none; do not plan GPU/CUDA work"));
        assert!(rendered.contains("do not use sudo or system package managers"));
    }

    #[test]
    fn strips_compute_section_from_legacy_system_prompts() {
        let mut prompt =
            "Base\n\n## Compute contexts\n\n- ssh:old\n\n## User Rules\n\nRules".to_string();
        strip_legacy_compute_section(&mut prompt);
        assert_eq!(prompt, "Base\n\n## User Rules\n\nRules");
    }

    #[test]
    fn ssh_connection_builds_ssh_and_scp_arguments() {
        let mut ctx = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        ctx.config_json = serde_json::json!({
            "alias": "gpu-box",
            "user": "alice",
            "port": 2222,
            "identity_file": "/home/alice/.ssh/lab key"
        })
        .to_string();

        let connection = SshConnection::from_execution_context(&ctx).unwrap();
        assert_eq!(connection.target().unwrap(), "alice@gpu-box");
        assert_eq!(
            connection.ssh_args().unwrap(),
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-T",
                "-p",
                "2222",
                "-o",
                "IdentitiesOnly=yes",
                "-i",
                "/home/alice/.ssh/lab key",
                "alice@gpu-box",
            ]
        );
        assert_eq!(
            connection.scp_option_args().unwrap(),
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-P",
                "2222",
                "-o",
                "IdentitiesOnly=yes",
                "-i",
                "/home/alice/.ssh/lab key",
            ]
        );
        assert_eq!(
            connection.interactive_ssh_args().unwrap(),
            [
                "-tt",
                "-p",
                "2222",
                "-o",
                "IdentitiesOnly=yes",
                "-i",
                "/home/alice/.ssh/lab key",
                "alice@gpu-box",
            ]
        );

        let json = serde_json::to_string(&connection).unwrap();
        let restored: SshConnection = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, connection);
    }

    #[test]
    fn ssh_connection_defaults_alias_from_context_id() {
        let ctx = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        let connection = SshConnection::from_execution_context(&ctx).unwrap();
        assert_eq!(connection.target().unwrap(), "gpu-box");
        assert_eq!(connection.ssh_args().unwrap().last().unwrap(), "gpu-box");
        assert!(connection
            .ssh_args()
            .unwrap()
            .windows(2)
            .any(|args| args == ["-o", "IdentitiesOnly=yes"]));
        assert!(connection
            .scp_option_args()
            .unwrap()
            .windows(2)
            .any(|args| args == ["-o", "IdentitiesOnly=yes"]));
        assert!(!connection
            .scp_option_args()
            .unwrap()
            .contains(&"gpu-box".into()));
    }

    #[test]
    fn host_name_overrides_alias_as_connection_target() {
        let connection = SshConnection {
            alias: "gpu-lab".into(),
            host_name: Some("10.0.0.5".into()),
            user: Some("alice".into()),
            port: None,
            identity_file: None,
            auth_method: SshAuthMethod::Key,
        };
        assert_eq!(connection.target().unwrap(), "alice@10.0.0.5");
        assert_eq!(
            connection.ssh_args().unwrap().last().unwrap(),
            "alice@10.0.0.5"
        );
        for host_name in ["", "-bad", "host name", "地址"] {
            let connection = SshConnection {
                alias: "gpu-lab".into(),
                host_name: Some(host_name.into()),
                user: None,
                port: None,
                identity_file: None,
                auth_method: SshAuthMethod::Key,
            };
            assert!(
                connection.target().is_err(),
                "accepted host_name {host_name:?}"
            );
        }
    }

    #[test]
    fn ssh_connection_rejects_unsafe_names_and_identity_paths() {
        for alias in ["", "-proxy", "gpu box", "gpu/box", "güp"] {
            let connection = SshConnection {
                alias: alias.into(),
                host_name: None,
                user: None,
                port: None,
                identity_file: None,
                auth_method: SshAuthMethod::Key,
            };
            assert!(connection.ssh_args().is_err(), "accepted alias {alias:?}");
        }
        for user in ["", "-root", "user@host", "用户"] {
            let connection = SshConnection {
                alias: "gpu-box".into(),
                host_name: None,
                user: Some(user.into()),
                port: None,
                identity_file: None,
                auth_method: SshAuthMethod::Key,
            };
            assert!(connection.target().is_err(), "accepted user {user:?}");
        }
        let connection = SshConnection {
            alias: "gpu-box".into(),
            host_name: None,
            user: None,
            port: None,
            identity_file: Some("\n/tmp/key".into()),
            auth_method: SshAuthMethod::Key,
        };
        assert!(connection.scp_option_args().is_err());
    }

    #[test]
    fn password_mode_args_disable_batch_and_pubkey() {
        let connection = SshConnection {
            alias: "lab".into(),
            host_name: None,
            user: Some("alice".into()),
            port: Some(22),
            identity_file: None,
            auth_method: SshAuthMethod::Password,
        };
        let args = connection.ssh_args().unwrap();
        assert!(args.windows(2).any(|w| w == ["-o", "BatchMode=no"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["-o", "PubkeyAuthentication=no"]));
        assert!(!args.iter().any(|a| a == "-i"));
        assert!(connection.assert_ready_to_connect().is_err());
    }

    #[test]
    fn render_contexts_lists_wsl_contexts_with_path_warning() {
        let mut ctx =
            wisp_store::ExecutionContext::new("wsl:Ubuntu-22.04", "Ubuntu-22.04").unwrap();
        ctx.config_json = serde_json::json!({ "distro": "Ubuntu-22.04" }).to_string();

        let s = render_contexts_section(&[ctx]).unwrap();
        assert!(s.contains("wsl:Ubuntu-22.04"), "context id missing:\n{s}");
        assert!(
            s.contains("Linux execution context"),
            "WSL guidance missing:\n{s}"
        );
        assert!(
            s.contains("paths may differ from Windows paths"),
            "WSL path warning missing:\n{s}"
        );
    }

    #[test]
    fn render_contexts_summarizes_capabilities() {
        let mut ctx = wisp_store::ExecutionContext::new("ssh:gpu-box", "gpu-box").unwrap();
        ctx.config_json = serde_json::json!({ "alias": "gpu-box" }).to_string();
        ctx.capabilities_json = serde_json::json!({
            "os": "Linux",
            "arch": "x86_64",
            "gpu_summary": "GPU 0: NVIDIA A100",
            "scheduler": "slurm",
            "python": "Python 3.11.8"
        })
        .to_string();

        let s = render_contexts_section(&[ctx]).unwrap();
        assert!(s.contains("Linux/x86_64"), "os/arch missing:\n{s}");
        assert!(s.contains("GPU 0: NVIDIA A100"), "gpu missing:\n{s}");
        assert!(s.contains("scheduler: slurm"), "scheduler missing:\n{s}");
        assert!(s.contains("Python 3.11.8"), "python missing:\n{s}");
    }
}
