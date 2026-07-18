//! SSH host registry, validated connection snapshots, and tauri commands.

use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SshHost {
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConnection {
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
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
            user: optional_config_string(&config, "user")?,
            port: optional_config_port(&config)?,
            identity_file: optional_config_string(&config, "identity_file")?,
        };
        connection.validate()?;
        Ok(connection)
    }

    fn from_host(host: &SshHost) -> Result<Self, String> {
        let connection = Self {
            alias: host.alias.clone(),
            user: host.user.clone(),
            port: host.port,
            identity_file: host.identity_file.clone(),
        };
        connection.validate()?;
        Ok(connection)
    }

    pub fn target(&self) -> Result<String, String> {
        self.validate()?;
        Ok(match &self.user {
            Some(user) => format!("{user}@{}", self.alias),
            None => self.alias.clone(),
        })
    }

    pub fn ssh_args(&self) -> Result<Vec<String>, String> {
        let mut args = common_option_args();
        args.push("-T".into());
        if let Some(port) = self.port {
            args.extend(["-p".into(), port.to_string()]);
        }
        push_batch_identity_args(&mut args, self.identity_file.as_deref());
        args.push(self.target()?);
        Ok(args)
    }

    /// Arguments for a user-driven interactive terminal. Unlike probes and
    /// Runs, this deliberately leaves BatchMode disabled so OpenSSH can show
    /// host-key, password, and keyboard-interactive prompts in the PTY.
    pub fn interactive_ssh_args(&self) -> Result<Vec<String>, String> {
        self.validate()?;
        let mut args = vec!["-tt".into()];
        if let Some(port) = self.port {
            args.extend(["-p".into(), port.to_string()]);
        }
        push_interactive_identity_args(&mut args, self.identity_file.as_deref());
        args.push(self.target()?);
        Ok(args)
    }

    pub fn scp_option_args(&self) -> Result<Vec<String>, String> {
        self.validate()?;
        let mut args = common_option_args();
        if let Some(port) = self.port {
            args.extend(["-P".into(), port.to_string()]);
        }
        push_batch_identity_args(&mut args, self.identity_file.as_deref());
        Ok(args)
    }

    /// Fail before spawning when a configured identity file is missing.
    /// Call this at connection entry points (not during pure arg construction).
    pub fn assert_ready_to_connect(&self) -> Result<(), String> {
        self.validate()?;
        if let Some(identity_file) = &self.identity_file {
            ensure_identity_file_accessible(identity_file)?;
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        validate_connection_name("SSH alias", &self.alias)?;
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
        Ok(())
    }
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
                 Check the SSH server (network, firewall/IP unlock, key path), then run Probe \
                 on this environment. Free-form shell `ssh` is disabled; agent access uses only \
                 the configured host settings (alias/user/port/identity).",
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
The user has these SSH hosts available. Use the shell tool only for quick, \
read-only probes. Submit real work and all long-running commands with \
`run_in_context` using the `ssh:<alias>` context. Do not use shell `sleep`, \
`ssh ... ps`, `nohup`, background `&`, or polling loops to monitor work. After \
submission, observe or cancel it through the Runs control plane. Remote paths \
live on the host, not on this machine.\n\n",
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
compute when they fit the task. Use the shell tool only for \
quick, read-only probes. Submit real work and all long-running commands with \
`run_in_context` using the context id. Do not use shell `sleep`, `ssh ... ps`, \
`nohup`, background `&`, or polling loops to monitor work. After submission, \
observe or cancel it through the Runs control plane. Remote paths are not local \
paths. For persistent interactive analysis, call `python` or `r` with the \
matching `context_id`; omitting it selects `local`. Interpreter paths come from \
the execution context's saved settings or probe result, not shell environment \
changes. Use `set_runtime_interpreter` when the user provides a different \
Python or R executable.\n\
**SSH connectivity policy:** remote work is only allowed after a successful Probe \
on the registered environment. Always use `run_in_context`, `python`, or `r` with \
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
    if let Some(user) = host.user.as_deref().filter(|s| !s.trim().is_empty()) {
        cfg.insert("user".into(), serde_json::Value::String(user.trim().into()));
    }
    if let Some(port) = host.port {
        cfg.insert("port".into(), serde_json::Value::from(port));
    }
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
    if let Some(notes) = host.notes.as_deref().filter(|s| !s.trim().is_empty()) {
        cfg.insert(
            "notes".into(),
            serde_json::Value::String(notes.trim().into()),
        );
    }
    serde_json::to_string(&serde_json::Value::Object(cfg)).map_err(|e| e.to_string())
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
    Ok(load(&state.store).await)
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

#[tauri::command]
pub async fn add_ssh_host(
    state: State<'_, crate::AppState>,
    host: SshHost,
) -> Result<Vec<SshHost>, String> {
    SshConnection::from_host(&host)?;
    let hosts = upsert_host(load(&state.store).await, host);
    save_and_sync_contexts(&state.store, &hosts).await?;
    Ok(hosts)
}

#[tauri::command]
pub async fn remove_ssh_host(
    state: State<'_, crate::AppState>,
    alias: String,
) -> Result<Vec<SshHost>, String> {
    let hosts = remove_host(load(&state.store).await, &alias);
    save(&state.store, &hosts).await?;
    remove_context_for_alias(&state.store, &alias).await?;
    Ok(hosts)
}

#[tauri::command]
pub async fn list_ssh_config_aliases() -> Result<Vec<String>, String> {
    let text = dirs::home_dir()
        .map(|h| h.join(".ssh").join("config"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    Ok(parse_ssh_config_aliases(&text))
}

/// Merge every connectable `Host` alias from ~/.ssh/config into the registry,
/// keeping hosts the user already configured (notes etc.) untouched (#56/#67).
pub fn merge_config_aliases(mut hosts: Vec<SshHost>, aliases: Vec<String>) -> Vec<SshHost> {
    for alias in aliases {
        if !hosts.iter().any(|h| h.alias == alias) {
            hosts.push(SshHost {
                alias,
                user: None,
                port: None,
                identity_file: None,
                notes: None,
            });
        }
    }
    hosts
}

#[tauri::command]
pub async fn import_ssh_config_hosts(
    state: State<'_, crate::AppState>,
) -> Result<Vec<SshHost>, String> {
    let aliases = list_ssh_config_aliases().await?;
    let hosts = merge_config_aliases(load(&state.store).await, aliases);
    save_and_sync_contexts(&state.store, &hosts).await?;
    Ok(hosts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(alias: &str, notes: Option<&str>) -> SshHost {
        SshHost {
            alias: alias.into(),
            user: None,
            port: None,
            identity_file: None,
            notes: notes.map(Into::into),
        }
    }

    #[test]
    fn missing_identity_file_fails_before_connect() {
        let connection = SshConnection {
            alias: "gpu".into(),
            user: None,
            port: None,
            identity_file: Some("/definitely/missing/wisp-test-key".into()),
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
                user: Some("alice".into()),
                port: Some(2222),
                identity_file: None,
                notes: Some("slurm; sbatch".into()),
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
    fn ssh_connection_rejects_unsafe_names_and_identity_paths() {
        for alias in ["", "-proxy", "gpu box", "gpu/box", "güp"] {
            let connection = SshConnection {
                alias: alias.into(),
                user: None,
                port: None,
                identity_file: None,
            };
            assert!(connection.ssh_args().is_err(), "accepted alias {alias:?}");
        }
        for user in ["", "-root", "user@host", "用户"] {
            let connection = SshConnection {
                alias: "gpu-box".into(),
                user: Some(user.into()),
                port: None,
                identity_file: None,
            };
            assert!(connection.target().is_err(), "accepted user {user:?}");
        }
        let connection = SshConnection {
            alias: "gpu-box".into(),
            user: None,
            port: None,
            identity_file: Some("\n/tmp/key".into()),
        };
        assert!(connection.scp_option_args().is_err());
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
