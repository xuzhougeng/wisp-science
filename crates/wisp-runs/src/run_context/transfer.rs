use super::remote::{checked_output, scp_local_path, ssh_script_command};
use super::{
    run_with_lifecycle_lease, tail, transfer_progress, ActiveRun, RunCommand, RunManager,
    SubmitRunRequest, SubmitRunResponse, ACTIVE_LEASE_SECS, REMOTE_RPC_TIMEOUT,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wisp_llm::ToolSchema;
use wisp_tools::{Approval, Tool, ToolEnv, ToolResult};

const TRUST_EDGES_SETTING: &str = "ssh_trust_edges_v1";
const PUBLIC_KEY_MARKER: &str = "__WISP_PUBLIC_KEY__:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTrustEdge {
    source_context_id: String,
    destination_context_id: String,
    destination_target: String,
    destination_port: Option<u16>,
    key_path: Option<String>,
    managed: bool,
    verified_at: i64,
}

#[derive(Debug, Deserialize)]
struct ConfigureTrustRequest {
    source_context_id: String,
    destination_context_id: String,
    #[serde(default = "default_install_action")]
    action: String,
}

fn default_install_action() -> String {
    "install".into()
}

#[derive(Debug, Deserialize)]
struct TransferRequest {
    source_context_id: String,
    source_path: String,
    destination_context_id: String,
    destination_path: Option<String>,
    #[serde(default = "default_auto")]
    route: String,
    #[serde(default = "default_auto")]
    transport: String,
    /// Continue an interrupted local↔SSH transfer instead of refusing the
    /// partially written destination. Requires transport=rsync.
    #[serde(default)]
    resume: bool,
    timeout_secs: Option<u64>,
}

fn default_auto() -> String {
    "auto".into()
}

/// Transport actually used for local↔SSH transfers. `auto` stays on scp;
/// rsync is explicit because it needs the binary on both sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TransferTransport {
    Scp,
    Rsync,
}

fn local_transport_choice(transport: &str) -> TransferTransport {
    if transport == "rsync" {
        TransferTransport::Rsync
    } else {
        TransferTransport::Scp
    }
}

fn transport_label(transport: TransferTransport) -> &'static str {
    match transport {
        TransferTransport::Scp => "scp",
        TransferTransport::Rsync => "rsync",
    }
}

/// Persisted so a transfer Run can be reclaimed after an app restart instead
/// of being marked `lost`. The scp/rsync process itself is local to Wisp, so
/// restart always retries (or fails cleanly) — it never reattaches a remote PID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransferHandle {
    LocalUpload {
        source_path: String,
        destination_context_id: String,
        destination_path: String,
        transport: String,
        resume: bool,
    },
    LocalDownload {
        source_context_id: String,
        source_path: String,
        destination_path: String,
        transport: String,
    },
    Relay {
        source_context_id: String,
        source_path: String,
        destination_context_id: String,
        destination_path: String,
    },
    Harvest {
        parent_run_id: String,
    },
}

impl TransferHandle {
    fn display_path(&self) -> &str {
        match self {
            Self::LocalUpload {
                destination_path, ..
            }
            | Self::Relay {
                destination_path, ..
            } => destination_path,
            Self::LocalDownload {
                destination_path, ..
            } => destination_path,
            Self::Harvest { parent_run_id } => parent_run_id,
        }
    }
}

pub async fn persist_transfer_handle(
    store: &wisp_store::Store,
    owner_id: &str,
    run_id: &str,
    handle: &TransferHandle,
) -> Result<(), String> {
    let json = serde_json::to_string(handle).map_err(|error| error.to_string())?;
    let _ = store
        .set_run_remote_handle_owned(run_id, owner_id, &json, handle.display_path())
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn ledger_upload_attempt(
    store: &wisp_store::Store,
    run_id: &str,
    destination_alias: &str,
    destination_path: &str,
    size_bytes: Option<i64>,
) -> Result<(), String> {
    let Ok(Some(run)) = store.get_run(run_id).await else {
        return Ok(());
    };
    let mut entry = wisp_store::RemoteStagingEntry::new(
        run.project_id,
        format!("ssh:{destination_alias}"),
        Some(run_id.to_string()),
        destination_path,
        "transfer",
    );
    entry.size_bytes = size_bytes;
    store
        .ensure_remote_staging(&entry)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub struct ConfigureSshTrustTool {
    store: wisp_store::Store,
    manager: RunManager,
    frame_id: Option<String>,
}

impl ConfigureSshTrustTool {
    pub fn new(store: wisp_store::Store, manager: RunManager, frame_id: Option<String>) -> Self {
        Self {
            store,
            manager,
            frame_id,
        }
    }
}

#[async_trait::async_trait]
impl Tool for ConfigureSshTrustTool {
    fn name(&self) -> &str {
        "configure_ssh_trust"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            "With explicit user approval, establish or verify passwordless SSH from one selected SSH context to another. `install` creates a dedicated key on the source, carries only its public key through Wisp, installs it idempotently on the destination, and verifies the directed edge. `verify` records trust the user configured themselves without copying a key.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "source_context_id": { "type": "string", "description": "Selected source SSH context id" },
                    "destination_context_id": { "type": "string", "description": "Selected destination SSH context id" },
                    "action": { "type": "string", "enum": ["install", "verify"], "default": "install" }
                },
                "required": ["source_context_id", "destination_context_id"]
            }),
        )
    }

    fn minimum_approval(&self) -> Approval {
        Approval::Ask
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        let source = args
            .get("source_context_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let destination = args
            .get("destination_context_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let action = args
            .get("action")
            .and_then(|value| value.as_str())
            .unwrap_or("install");
        format!("{action} {source} → {destination}")
    }

    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let request: ConfigureTrustRequest = match serde_json::from_value(args.clone()) {
            Ok(request) => request,
            Err(error) => {
                return ToolResult::fail(format!("configure_ssh_trust args error: {error}"))
            }
        };
        let result = configure_trust(
            &self.store,
            self.manager.runner.as_ref(),
            self.frame_id.as_deref(),
            &request,
        )
        .await;
        match result {
            Ok(edge) => ToolResult::ok(
                serde_json::to_string(&serde_json::json!({
                    "source_context_id": edge.source_context_id,
                    "destination_context_id": edge.destination_context_id,
                    "managed": edge.managed,
                    "key_path": edge.key_path,
                    "verified": true
                }))
                .unwrap_or_default(),
            ),
            Err(error) => ToolResult::fail(error),
        }
    }
}

pub struct TransferBetweenContextsTool {
    store: wisp_store::Store,
    manager: RunManager,
    project_id: String,
    frame_id: Option<String>,
}

impl TransferBetweenContextsTool {
    pub fn new(
        store: wisp_store::Store,
        manager: RunManager,
        project_id: String,
        frame_id: Option<String>,
    ) -> Self {
        Self {
            store,
            manager,
            project_id,
            frame_id,
        }
    }
}

#[async_trait::async_trait]
impl Tool for TransferBetweenContextsTool {
    fn name(&self) -> &str {
        "transfer_between_contexts"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            "Transfer one exact file or directory between `local` and a selected SSH context, or between two selected SSH contexts, as a persisted Run. SSH-to-SSH `auto` uses a verified direct edge when available (rsync with scp fallback), otherwise it relays through a private local temporary directory. Local transfers default to scp and never overwrite an existing destination; pick transport=rsync for large files so an interrupted transfer can be retried with resume=true instead of starting over. Never use shell ssh/scp/rsync for this.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "source_context_id": { "type": "string", "description": "Selected SSH context id, or `local` for an upload" },
                    "source_path": { "type": "string", "description": "SSH: exact absolute or ~/ path. Local: exact absolute file/directory path. Globs are rejected." },
                    "destination_context_id": { "type": "string", "description": "Selected SSH context id, or `local` for a download" },
                    "destination_path": { "type": "string", "description": "SSH: exact absolute or ~/ path; omit to place the file under this project's configured remote data root for the destination server. Local: exact new absolute file/directory path; do not guess it—ask the user when unspecified. Globs are rejected." },
                    "route": { "type": "string", "enum": ["auto", "direct", "relay"], "default": "auto", "description": "direct/relay apply to SSH-to-SSH; transfers involving local accept auto or relay" },
                    "transport": { "type": "string", "enum": ["auto", "rsync", "scp"], "default": "auto", "description": "Local↔SSH: auto/scp use scp; rsync requires rsync on both sides and supports resumable transfers" },
                    "resume": { "type": "boolean", "default": false, "description": "Local↔SSH with transport=rsync only: continue an interrupted transfer, reusing partial data instead of refusing the existing destination" },
                    "timeout_secs": { "type": "integer", "description": "Wall timeout, 1 second to 7 days" }
                },
                "required": ["source_context_id", "source_path", "destination_context_id"]
            }),
        )
    }

    fn minimum_approval(&self) -> Approval {
        Approval::Ask
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        let source = args
            .get("source_context_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let source_path = args
            .get("source_path")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let destination = args
            .get("destination_context_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let destination_path = args
            .get("destination_path")
            .and_then(|value| value.as_str())
            .unwrap_or("<remote data root>");
        format!("{source}:{source_path} → {destination}:{destination_path}")
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let request: TransferRequest = match serde_json::from_value(args.clone()) {
            Ok(request) => request,
            Err(error) => {
                return ToolResult::fail(format!("transfer_between_contexts args error: {error}"))
            }
        };
        match submit_transfer(
            &self.store,
            &self.manager,
            &self.project_id,
            self.frame_id.as_deref(),
            env.project_root(),
            request,
        )
        .await
        {
            Ok(value) => ToolResult::ok(value.to_string()),
            Err(error) => ToolResult::fail(error),
        }
    }
}

async fn selected_ssh_context(
    store: &wisp_store::Store,
    frame_id: Option<&str>,
    context_id: &str,
) -> Result<wisp_store::ExecutionContext, String> {
    let context = store
        .get_execution_context(context_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
    if context.kind != wisp_store::ExecutionContextKind::Ssh {
        return Err(format!("Execution context is not SSH: {context_id}"));
    }
    let frame_id = frame_id.ok_or_else(|| {
        "Server-to-server operations require an active conversation with both SSH contexts selected"
            .to_string()
    })?;
    if !store
        .session_execution_context_enabled(frame_id, context_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(format!(
            "Execution context {context_id} is not selected for this session"
        ));
    }
    crate::ssh_hosts::require_managed_ssh_ready(&context)?;
    Ok(context)
}

async fn configure_trust(
    store: &wisp_store::Store,
    runner: &dyn super::RunCommandRunner,
    frame_id: Option<&str>,
    request: &ConfigureTrustRequest,
) -> Result<SshTrustEdge, String> {
    if request.source_context_id == request.destination_context_id {
        return Err("Source and destination SSH contexts must be different".into());
    }
    if !matches!(request.action.as_str(), "install" | "verify") {
        return Err("action must be 'install' or 'verify'".into());
    }
    let source = selected_ssh_context(store, frame_id, &request.source_context_id).await?;
    let destination =
        selected_ssh_context(store, frame_id, &request.destination_context_id).await?;
    let source_connection = crate::ssh_hosts::SshConnection::from_execution_context(&source)?;
    let destination_connection =
        crate::ssh_hosts::SshConnection::from_execution_context(&destination)?;
    let target = destination_connection.target()?;
    let marker = format!(
        "wisp:{}:{}",
        source_connection.alias, destination_connection.alias
    );
    let key_path = (request.action == "install")
        .then(|| format!(".ssh/wisp-{}-ed25519", destination_connection.alias));

    if let Some(key_path) = key_path.as_deref() {
        let output = checked_output(
            "Generate source transfer key",
            runner
                .run(
                    ssh_script_command(
                        &source_connection,
                        "generate source transfer key",
                        generate_key_payload(key_path, &marker),
                    )?,
                    REMOTE_RPC_TIMEOUT,
                )
                .await,
        )?;
        let public_key = parse_public_key(&output.stdout, &marker)?;
        checked_output(
            "Install destination transfer key",
            runner
                .run(
                    ssh_script_command(
                        &destination_connection,
                        "install destination transfer key",
                        install_public_key_payload(&public_key, &marker),
                    )?,
                    REMOTE_RPC_TIMEOUT,
                )
                .await,
        )?;
    }

    let verify = checked_output(
        "Verify server-to-server SSH trust",
        runner
            .run(
                ssh_script_command(
                    &source_connection,
                    "verify server-to-server SSH trust",
                    verify_trust_payload(
                        &target,
                        destination_connection.port,
                        key_path.as_deref(),
                        request.action == "install",
                    ),
                )?,
                REMOTE_RPC_TIMEOUT,
            )
            .await,
    )
    .map_err(|error| {
        if request.action == "install" {
            format!(
                "{error}. The dedicated public key was installed on the destination, but A→B \
                 verification failed; check that the destination address is reachable from the \
                 source and that public-key authentication is enabled."
            )
        } else {
            error
        }
    })?;
    if !verify.stdout.contains("__WISP_TRUST_VERIFIED__") {
        let detail = verify
            .stderr
            .lines()
            .find_map(|line| line.strip_prefix("__WISP_TRUST_FAILED__:"))
            .unwrap_or("source could not authenticate to the destination");
        return Err(format!(
            "Server-to-server SSH verification failed: {detail}"
        ));
    }

    let edge = SshTrustEdge {
        source_context_id: source.id,
        destination_context_id: destination.id,
        destination_target: target,
        destination_port: destination_connection.port,
        key_path,
        managed: request.action == "install",
        verified_at: chrono::Utc::now().timestamp(),
    };
    save_trust_edge(store, edge.clone()).await?;
    Ok(edge)
}

fn generate_key_payload(key_path: &str, marker: &str) -> String {
    format!(
        r#"set -eu
umask 077
mkdir -p "$HOME/.ssh"
chmod 700 "$HOME/.ssh"
key="$HOME/{key_path}"
if [ ! -f "$key" ]; then
  command -v ssh-keygen >/dev/null 2>&1 || {{ echo 'ssh-keygen is not installed on the source' >&2; exit 69; }}
  rm -f "$key.pub"
  ssh-keygen -q -t ed25519 -N '' -C '{marker}' -f "$key"
fi
if [ ! -f "$key.pub" ]; then
  ssh-keygen -y -f "$key" > "$key.pub"
fi
set -- $(cat "$key.pub")
[ "$#" -ge 2 ] || {{ echo 'generated public key is malformed' >&2; exit 65; }}
printf '{PUBLIC_KEY_MARKER}%s %s\n' "$1" "$2"
"#
    )
}

fn parse_public_key(stdout: &str, marker: &str) -> Result<String, String> {
    let value = stdout
        .lines()
        .find_map(|line| line.strip_prefix(PUBLIC_KEY_MARKER))
        .ok_or_else(|| "Source did not return its generated public key".to_string())?;
    let mut fields = value.split_whitespace();
    let kind = fields.next().unwrap_or_default();
    let encoded = fields.next().unwrap_or_default();
    if kind != "ssh-ed25519"
        || encoded.len() < 32
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+/=".contains(&byte))
    {
        return Err("Source returned an invalid Ed25519 public key".into());
    }
    Ok(format!("{kind} {encoded} {marker}"))
}

fn install_public_key_payload(public_key: &str, marker: &str) -> String {
    let public_key = shell_single_quote(public_key);
    let marker = shell_single_quote(&format!(" {marker}"));
    format!(
        r#"set -eu
umask 077
mkdir -p "$HOME/.ssh"
chmod 700 "$HOME/.ssh"
auth="$HOME/.ssh/authorized_keys"
touch "$auth"
chmod 600 "$auth"
tmp="$auth.wisp.$$"
grep -Fv -- {marker} "$auth" > "$tmp" || true
printf '%s\n' {public_key} >> "$tmp"
chmod 600 "$tmp"
mv "$tmp" "$auth"
printf '__WISP_TRUST_INSTALLED__\n'
"#
    )
}

fn verify_trust_payload(
    target: &str,
    port: Option<u16>,
    key_path: Option<&str>,
    accept_new_host_key: bool,
) -> String {
    let mut options = vec![
        "-T".to_string(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        format!(
            "StrictHostKeyChecking={}",
            if accept_new_host_key {
                "accept-new"
            } else {
                "yes"
            }
        ),
    ];
    if let Some(key_path) = key_path {
        options.extend([
            "-o".into(),
            "IdentitiesOnly=yes".into(),
            "-i".into(),
            format!("$HOME/{key_path}"),
        ]);
    }
    if let Some(port) = port {
        options.extend(["-p".into(), port.to_string()]);
    }
    let args = options
        .iter()
        .map(|value| {
            if value.starts_with("$HOME/") {
                format!("\"{value}\"")
            } else {
                shell_single_quote(value)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "set -eu\ncommand -v ssh >/dev/null 2>&1 || {{ echo 'ssh is not installed on the source' >&2; exit 69; }}\nset +e\nssh {args} {} true\nrc=$?\nset -e\nif [ \"$rc\" = 0 ]; then printf '__WISP_TRUST_VERIFIED__\\n'; else printf '__WISP_TRUST_FAILED__:ssh exit %s\\n' \"$rc\" >&2; fi\n",
        shell_single_quote(target)
    )
}

pub async fn load_trust_edges(store: &wisp_store::Store) -> Vec<SshTrustEdge> {
    store
        .get_setting(TRUST_EDGES_SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

async fn save_trust_edge(store: &wisp_store::Store, edge: SshTrustEdge) -> Result<(), String> {
    let mut edges = load_trust_edges(store).await;
    edges.retain(|current| {
        current.source_context_id != edge.source_context_id
            || current.destination_context_id != edge.destination_context_id
    });
    edges.push(edge);
    store
        .set_setting(
            TRUST_EDGES_SETTING,
            &serde_json::to_string(&edges).map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| error.to_string())
}

#[derive(Debug, Serialize)]
pub struct RevokeTrustResponse {
    edges: Vec<SshTrustEdge>,
    cleanup_error: Option<String>,
}

pub async fn revoke_trust_edge(
    store: &wisp_store::Store,
    manager: &RunManager,
    source_context_id: &str,
    destination_context_id: &str,
) -> Result<RevokeTrustResponse, String> {
    let mut edges = load_trust_edges(store).await;
    let Some(index) = edges.iter().position(|edge| {
        edge.source_context_id == source_context_id
            && edge.destination_context_id == destination_context_id
    }) else {
        return Ok(RevokeTrustResponse {
            edges,
            cleanup_error: None,
        });
    };
    let edge = edges.remove(index);
    store
        .set_setting(
            TRUST_EDGES_SETTING,
            &serde_json::to_string(&edges).map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
    // The record is what authorizes the app's direct route, so it goes first;
    // removing the installed key material is best effort — an unreachable or
    // already-deleted host must never block revocation. Failures are reported
    // back to the user, not treated as fatal.
    let cleanup_error = if edge.managed {
        remove_managed_key(store, manager.runner.as_ref(), &edge)
            .await
            .err()
    } else {
        None
    };
    Ok(RevokeTrustResponse {
        edges,
        cleanup_error,
    })
}

fn context_alias(context_id: &str) -> &str {
    context_id.strip_prefix("ssh:").unwrap_or(context_id)
}

async fn ssh_connection_for(
    store: &wisp_store::Store,
    context_id: &str,
) -> Result<crate::ssh_hosts::SshConnection, String> {
    let context = store
        .get_execution_context(context_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("execution context no longer exists: {context_id}"))?;
    crate::ssh_hosts::SshConnection::from_execution_context(&context)
}

async fn best_effort_ssh(
    runner: &dyn super::RunCommandRunner,
    connection: &crate::ssh_hosts::SshConnection,
    label: &str,
    payload: String,
    errors: &mut Vec<String>,
) {
    let result = match ssh_script_command(connection, label, payload) {
        Ok(command) => {
            checked_output(label, runner.run(command, REMOTE_RPC_TIMEOUT).await).map(|_| ())
        }
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        errors.push(error);
    }
}

async fn remove_managed_key(
    store: &wisp_store::Store,
    runner: &dyn super::RunCommandRunner,
    edge: &SshTrustEdge,
) -> Result<(), String> {
    let mut errors = Vec::new();
    let source = ssh_connection_for(store, &edge.source_context_id).await;
    let source_alias = source
        .as_ref()
        .map(|connection| connection.alias.clone())
        .unwrap_or_else(|_| context_alias(&edge.source_context_id).to_string());
    match ssh_connection_for(store, &edge.destination_context_id).await {
        Ok(connection) => {
            let marker = format!("wisp:{source_alias}:{}", connection.alias);
            best_effort_ssh(
                runner,
                &connection,
                "remove destination transfer key",
                remove_public_key_payload(&marker),
                &mut errors,
            )
            .await;
        }
        Err(error) => errors.push(format!("destination: {error}")),
    }
    if let Some(key_path) = edge.key_path.as_deref() {
        match &source {
            Ok(connection) => {
                best_effort_ssh(
                    runner,
                    connection,
                    "remove source transfer key",
                    remove_key_file_payload(key_path),
                    &mut errors,
                )
                .await;
            }
            Err(error) => errors.push(format!("source: {error}")),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Reverse of `install_public_key_payload`: drop the authorized_keys line
/// carrying our marker, leaving every other key untouched.
fn remove_public_key_payload(marker: &str) -> String {
    let marker = shell_single_quote(&format!(" {marker}"));
    format!(
        r#"set -eu
auth="$HOME/.ssh/authorized_keys"
if [ -f "$auth" ]; then
  tmp="$auth.wisp.$$"
  grep -Fv -- {marker} "$auth" > "$tmp" || true
  chmod 600 "$tmp"
  mv "$tmp" "$auth"
fi
"#
    )
}

fn remove_key_file_payload(key_path: &str) -> String {
    format!("set -eu\nrm -f \"$HOME/{key_path}\" \"$HOME/{key_path}.pub\"\n")
}

fn validate_remote_path(label: &str, path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains(['\0', '\n', '\r'])
        || path.contains(['*', '?', '[', ']', '{', '}'])
    {
        return Err(format!(
            "{label} must be one exact path without control characters or globs"
        ));
    }
    if !(path.starts_with('/') || path.starts_with("~/")) {
        return Err(format!("{label} must be absolute or start with ~/"));
    }
    if matches!(path.trim_end_matches('/'), "" | "~") {
        return Err(format!("{label} may not be the filesystem or home root"));
    }
    Ok(())
}

fn validate_local_destination(path: &str) -> Result<PathBuf, String> {
    if path.is_empty()
        || path.contains(['\0', '\n', '\r'])
        || path.contains(['*', '?', '[', ']', '{', '}'])
    {
        return Err(
            "destination_path must be one exact local path without control characters or globs"
                .into(),
        );
    }
    let destination = PathBuf::from(path);
    if !destination.is_absolute() || destination.file_name().is_none() {
        return Err("local destination_path must be an absolute non-root path".into());
    }
    if destination.exists() {
        return Err(format!(
            "local destination_path already exists: {}",
            destination.display()
        ));
    }
    Ok(destination)
}

fn validate_local_source(path: &str) -> Result<PathBuf, String> {
    if path.is_empty()
        || path.contains(['\0', '\n', '\r'])
        || path.contains(['*', '?', '[', ']', '{', '}'])
    {
        return Err(
            "source_path must be one exact local path without control characters or globs".into(),
        );
    }
    let source = PathBuf::from(path);
    if !source.is_absolute() || source.file_name().is_none() {
        return Err("local source_path must be an absolute non-root path".into());
    }
    let metadata = std::fs::symlink_metadata(&source)
        .map_err(|error| format!("local source_path is unavailable: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("local source_path may not be a symbolic link".into());
    }
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("local source_path must be a regular file or directory".into());
    }
    Ok(source)
}

fn remote_item_name(path: &str) -> Result<&str, String> {
    let name = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty() && !matches!(*name, "." | ".."))
        .ok_or_else(|| "source_path must name one file or directory".to_string())?;
    Ok(name)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadToContextItem {
    pub source_path: String,
    pub destination_path: String,
    pub run_id: String,
    pub status: String,
}

/// Place one local item into the remote directory the Files panel is showing.
pub fn join_remote_upload_destination(dir: &str, item_name: &str) -> Result<String, String> {
    if dir.is_empty()
        || dir.contains(['\0', '\n', '\r'])
        || dir.contains(['*', '?', '[', ']', '{', '}'])
    {
        return Err(
            "destination directory must be one exact path without control characters or globs"
                .into(),
        );
    }
    if !(dir.starts_with('/') || dir == "~" || dir.starts_with("~/")) {
        return Err("destination directory must be absolute or start with ~/".into());
    }
    if item_name.is_empty()
        || item_name.contains(['/', '\\', '\0', '\n', '\r'])
        || matches!(item_name, "." | "..")
    {
        return Err("upload item name is invalid".into());
    }
    let dest = match dir.trim_end_matches('/') {
        "" | "/" => format!("/{item_name}"),
        "~" => format!("~/{item_name}"),
        trimmed => format!("{trimmed}/{item_name}"),
    };
    validate_remote_path("destination_path", &dest)?;
    Ok(dest)
}

/// UI-initiated local → SSH uploads. Unlike the agent tool, this does not
/// require the destination to be attached to the current session — Files can
/// browse any registered, probed host.
pub async fn submit_local_uploads_to_context(
    store: &wisp_store::Store,
    manager: &RunManager,
    project_id: &str,
    frame_id: Option<&str>,
    context_id: &str,
    destination_dir: &str,
    source_paths: &[String],
) -> Result<Vec<UploadToContextItem>, String> {
    if source_paths.is_empty() {
        return Err("upload_to_context requires at least one local path".into());
    }
    if context_id == "local" {
        return Err("upload_to_context requires an SSH context".into());
    }
    let context = store
        .get_execution_context(context_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
    if context.kind != wisp_store::ExecutionContextKind::Ssh {
        return Err(format!("Execution context is not SSH: {context_id}"));
    }
    crate::ssh_hosts::require_managed_ssh_ready(&context)?;

    let mut prepared = Vec::with_capacity(source_paths.len());
    for path in source_paths {
        let source = validate_local_source(path)?;
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "local source_path has no portable item name".to_string())?;
        let destination_path = join_remote_upload_destination(destination_dir, name)?;
        prepared.push((source, destination_path));
    }

    let timeout = Duration::from_secs(4 * 60 * 60);
    let mut items = Vec::with_capacity(prepared.len());
    for (source, destination_path) in prepared {
        let response = manager
            .submit_local_upload_to_ssh(
                store.clone(),
                project_id,
                frame_id,
                &source,
                &context,
                &destination_path,
                TransferTransport::Scp,
                false,
                timeout,
            )
            .await?;
        items.push(UploadToContextItem {
            source_path: source.to_string_lossy().into_owned(),
            destination_path,
            run_id: response.run_id,
            status: response.status.as_str().to_string(),
        });
    }
    Ok(items)
}

/// When the caller omits destination_path for an SSH destination, uploads land
/// under this project's configured remote data root for that server.
async fn default_remote_destination(
    store: &wisp_store::Store,
    project_id: &str,
    context_id: &str,
    source_path: &str,
) -> Result<String, String> {
    let (prefs, _) = crate::storage_prefs::effective_prefs(store, project_id, context_id).await?;
    let root =
        if prefs.remote_data_root.starts_with('/') || prefs.remote_data_root.starts_with("~/") {
            prefs.remote_data_root
        } else {
            format!("~/{}", prefs.remote_data_root)
        };
    let normalized = source_path.replace('\\', "/");
    let name = normalized
        .rsplit('/')
        .find(|part| !part.is_empty())
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| "cannot derive a destination file name from source_path".to_string())?;
    Ok(format!("{root}/{name}"))
}

async fn submit_transfer(
    store: &wisp_store::Store,
    manager: &RunManager,
    project_id: &str,
    frame_id: Option<&str>,
    project_root: &Path,
    request: TransferRequest,
) -> Result<serde_json::Value, String> {
    if !matches!(request.route.as_str(), "auto" | "direct" | "relay") {
        return Err("route must be 'auto', 'direct', or 'relay'".into());
    }
    if !matches!(request.transport.as_str(), "auto" | "rsync" | "scp") {
        return Err("transport must be 'auto', 'rsync', or 'scp'".into());
    }
    let local_route =
        request.source_context_id == "local" || request.destination_context_id == "local";
    if request.resume && !(local_route && request.transport == "rsync") {
        return Err(
            "resume only applies to local↔SSH transfers with transport=rsync; retry the \
             interrupted transfer with transport=rsync, resume=true"
                .into(),
        );
    }
    let timeout_secs = request
        .timeout_secs
        .unwrap_or(4 * 60 * 60)
        .clamp(1, 7 * 24 * 60 * 60);
    let destination_path = match request.destination_path.clone() {
        Some(path) => path,
        None if request.destination_context_id == "local" => {
            return Err(
                "destination_path is required for downloads to local; ask the user where the \
                 file should go"
                    .into(),
            )
        }
        None => {
            default_remote_destination(
                store,
                project_id,
                &request.destination_context_id,
                &request.source_path,
            )
            .await?
        }
    };

    if request.source_context_id == "local" {
        if request.destination_context_id == "local" {
            return Err("Source and destination contexts cannot both be local".into());
        }
        if request.route == "direct" {
            return Err("Local-to-SSH transfers use route=auto or route=relay".into());
        }
        let source_path = validate_local_source(&request.source_path)?;
        validate_remote_path("destination_path", &destination_path)?;
        let destination =
            selected_ssh_context(store, frame_id, &request.destination_context_id).await?;
        let transport = local_transport_choice(&request.transport);
        let response = manager
            .submit_local_upload_to_ssh(
                store.clone(),
                project_id,
                frame_id,
                &source_path,
                &destination,
                &destination_path,
                transport,
                request.resume,
                Duration::from_secs(timeout_secs),
            )
            .await?;
        return Ok(serde_json::json!({
            "run_id": response.run_id,
            "status": response.status,
            "route": "local",
            "transport": transport_label(transport),
            "source_path": source_path,
            "destination_path": destination_path,
            "next_action": "Call monitor_run to wait for completion; if wait_interrupted, respond then call it again with the same run_id. Do not resubmit."
        }));
    }

    validate_remote_path("source_path", &request.source_path)?;
    super::remote_files::refuse_if_context_path_discarded(
        store,
        &request.source_context_id,
        &request.source_path,
    )
    .await?;
    let source = selected_ssh_context(store, frame_id, &request.source_context_id).await?;

    if request.destination_context_id == "local" {
        if request.route == "direct" {
            return Err("SSH-to-local transfers use route=auto or route=relay".into());
        }
        let destination = validate_local_destination(&destination_path)?;
        let transport = local_transport_choice(&request.transport);
        let response = manager
            .submit_ssh_download_to_local(
                store.clone(),
                project_id,
                frame_id,
                &source,
                &request.source_path,
                &destination,
                transport,
                Duration::from_secs(timeout_secs),
            )
            .await?;
        return Ok(serde_json::json!({
            "run_id": response.run_id,
            "status": response.status,
            "route": "local",
            "transport": transport_label(transport),
            "destination_path": destination,
            "next_action": "Call monitor_run to wait for completion; if wait_interrupted, respond then call it again with the same run_id. Do not resubmit."
        }));
    }

    if request.source_context_id == request.destination_context_id {
        return Err("Source and destination SSH contexts must be different".into());
    }
    validate_remote_path("destination_path", &destination_path)?;
    let destination =
        selected_ssh_context(store, frame_id, &request.destination_context_id).await?;
    let destination_connection =
        crate::ssh_hosts::SshConnection::from_execution_context(&destination)?;
    let current_target = destination_connection.target()?;
    let mut edge = load_trust_edges(store).await.into_iter().find(|edge| {
        edge.source_context_id == source.id
            && edge.destination_context_id == destination.id
            && edge.destination_target == current_target
            && edge.destination_port == destination_connection.port
    });
    if request.route != "relay" {
        if let Some(candidate) = edge.as_ref() {
            let source_connection =
                crate::ssh_hosts::SshConnection::from_execution_context(&source)?;
            let output = checked_output(
                "Check server-to-server SSH trust",
                manager
                    .runner
                    .run(
                        ssh_script_command(
                            &source_connection,
                            "check server-to-server SSH trust",
                            verify_trust_payload(
                                &candidate.destination_target,
                                candidate.destination_port,
                                candidate.key_path.as_deref(),
                                false,
                            ),
                        )?,
                        REMOTE_RPC_TIMEOUT,
                    )
                    .await,
            )?;
            if !output.stdout.contains("__WISP_TRUST_VERIFIED__") {
                edge = None;
            }
        }
    }
    let route = match request.route.as_str() {
        "direct" if edge.is_none() => {
            return Err(format!(
                "No verified direct SSH edge exists from {} to {}. Call configure_ssh_trust \
                 with action=install, verify user-managed trust, or choose route=relay.",
                source.id, destination.id
            ))
        }
        "direct" => "direct",
        "relay" => "relay",
        _ if edge.is_some() => "direct",
        _ => "relay",
    };
    let response = if route == "direct" {
        let edge = edge.expect("direct route requires edge");
        let command = direct_transfer_script(
            &request.source_path,
            &destination_path,
            &edge,
            &request.transport,
        )?;
        manager
            .submit(
                store.clone(),
                project_id.into(),
                frame_id.map(Into::into),
                SubmitRunRequest {
                    context_id: source.id.clone(),
                    command,
                    title: Some(format!("Transfer {} → {}", source.label, destination.label)),
                    timeout_secs: Some(timeout_secs),
                    input_paths: None,
                    output_specs: None,
                },
                Some(project_root.to_path_buf()),
            )
            .await?
    } else {
        if request.transport == "rsync" {
            return Err("The relay route uses scp; choose transport=auto or transport=scp".into());
        }
        manager
            .submit_ssh_relay(
                store.clone(),
                project_id,
                frame_id,
                &source,
                &request.source_path,
                &destination,
                &destination_path,
                Duration::from_secs(timeout_secs),
            )
            .await?
    };
    Ok(serde_json::json!({
        "run_id": response.run_id,
        "status": response.status,
        "route": route,
        "transport": if route == "relay" { "scp" } else { request.transport.as_str() },
        "next_action": "Call monitor_run to wait for completion; if wait_interrupted, respond then call it again with the same run_id. Do not resubmit."
    }))
}

fn direct_transfer_script(
    source_path: &str,
    destination_path: &str,
    edge: &SshTrustEdge,
    transport: &str,
) -> Result<String, String> {
    let source_assignment = remote_path_assignment("src", source_path);
    let destination_assignment = format!("dst={}", shell_single_quote(destination_path));
    let key_setup = edge.key_path.as_deref().map_or_else(
        || "key=''\n".to_string(),
        |path| format!("key=\"$HOME/{}\"\n[ -f \"$key\" ] || {{ echo 'managed transfer key is missing on the source' >&2; exit 66; }}\n", path),
    );
    let identity = edge
        .key_path
        .is_some()
        .then_some("ssh_options+=( -o IdentitiesOnly=yes -i \"$key\" )\nscp_options+=( -o IdentitiesOnly=yes -i \"$key\" )\n")
        .unwrap_or_default();
    let port = edge.destination_port.map_or_else(String::new, |port| {
        format!("ssh_options+=( -p '{port}' )\nscp_options+=( -P '{port}' )\n")
    });
    let selection = match transport {
        "auto" => {
            r#"if command -v rsync >/dev/null 2>&1 && "${ssh_options[@]}" "$target" 'command -v rsync >/dev/null 2>&1'; then
  selected=rsync
else
  selected=scp
fi"#
        }
        "rsync" => {
            r#"command -v rsync >/dev/null 2>&1 || { echo 'rsync is not installed on the source' >&2; exit 69; }
"${ssh_options[@]}" "$target" 'command -v rsync >/dev/null 2>&1' || { echo 'rsync is not installed on the destination' >&2; exit 69; }
selected=rsync"#
        }
        "scp" => "selected=scp",
        _ => return Err("Unsupported transfer transport".into()),
    };
    Ok(format!(
        r#"set -euo pipefail
{source_assignment}
{destination_assignment}
[ -e "$src" ] || {{ echo 'source path does not exist' >&2; exit 66; }}
{key_setup}target={target}
ssh_options=(ssh -T -o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=yes)
scp_options=(-o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=yes)
{identity}{port}if [[ "$dst" = "~/"* ]]; then
  remote_home=$("${{ssh_options[@]}}" "$target" 'printf %s "$HOME"')
  dst="$remote_home/${{dst:2}}"
fi
{selection}
if [ "$selected" = rsync ]; then
  rsh='ssh -T -o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=yes'
  if [ -n "$key" ]; then printf -v quoted_key '%q' "$key"; rsh="$rsh -o IdentitiesOnly=yes -i $quoted_key"; fi
  {rsh_port}
  printf '__WISP_TRANSFER_TRANSPORT__:rsync\n'
  rsync -a -s --partial -e "$rsh" "$src" "$target:$dst"
else
  command -v scp >/dev/null 2>&1 || {{ echo 'scp is not installed on the source' >&2; exit 69; }}
  if [ -d "$src" ]; then scp_options+=( -r ); fi
  printf '__WISP_TRANSFER_TRANSPORT__:scp\n'
  scp "${{scp_options[@]}}" "$src" "$target:$dst"
fi
"#,
        target = shell_single_quote(&edge.destination_target),
        rsh_port = edge
            .destination_port
            .map(|port| format!("rsh=\"$rsh -p {port}\""))
            .unwrap_or_default(),
    ))
}

impl RunManager {
    /// Resume a transfer after process restart, or fail it cleanly. Never
    /// mark a transfer `lost` — there is no remote supervisor to reattach.
    pub(super) async fn reclaim_transfer(
        &self,
        store: wisp_store::Store,
        run: &wisp_store::RunRecord,
    ) -> Result<(), String> {
        if self.active.lock().await.contains_key(&run.id) {
            return Ok(());
        }
        let handle = run
            .remote_handle_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<TransferHandle>(json).ok());
        let Some(handle) = handle else {
            let _ = store
                .record_run_poll_owned(
                    &run.id,
                    &self.owner_id,
                    None,
                    None,
                    Some("transfer has no recoverable handle; retry the transfer"),
                )
                .await;
            let _ = store
                .finish_active_run_owned(
                    &run.id,
                    &self.owner_id,
                    wisp_store::RunStatus::Failed,
                    Some(-1),
                )
                .await;
            return Ok(());
        };
        let timeout = Duration::from_secs(run.timeout_secs.unwrap_or(4 * 60 * 60) as u64);
        let started = Instant::now();
        match handle {
            TransferHandle::LocalUpload {
                source_path,
                destination_context_id,
                destination_path,
                transport,
                ..
            } => {
                let Some(destination) = store
                    .get_execution_context(&destination_context_id)
                    .await
                    .map_err(|e| e.to_string())?
                else {
                    return self
                        .fail_transfer(
                            &store,
                            &run.id,
                            &format!("destination context {destination_context_id} is gone"),
                        )
                        .await;
                };
                let connection =
                    crate::ssh_hosts::SshConnection::from_execution_context(&destination)?;
                let runner = self.runner.clone();
                let owner_id = self.owner_id.clone();
                let run_id = run.id.clone();
                let active = self.active.clone();
                let cleanup_id = run_id.clone();
                let task_store = store;
                let task = tokio::spawn(async move {
                    let result = local_upload_lifecycle(
                        &task_store,
                        &owner_id,
                        &run_id,
                        runner,
                        PathBuf::from(source_path),
                        connection,
                        destination_path,
                        local_transport_choice(&transport),
                        true,
                        timeout,
                        started,
                    )
                    .await;
                    if let Err(error) = result {
                        tracing::warn!(run_id, "reclaimed upload failed: {error}");
                    }
                });
                self.active.lock().await.insert(
                    cleanup_id.clone(),
                    ActiveRun {
                        abort: task.abort_handle(),
                    },
                );
                tokio::spawn(async move {
                    let _ = task.await;
                    active.lock().await.remove(&cleanup_id);
                });
            }
            TransferHandle::LocalDownload {
                source_context_id,
                source_path,
                destination_path,
                transport,
            } => {
                let Some(source) = store
                    .get_execution_context(&source_context_id)
                    .await
                    .map_err(|e| e.to_string())?
                else {
                    return self
                        .fail_transfer(
                            &store,
                            &run.id,
                            &format!("source context {source_context_id} is gone"),
                        )
                        .await;
                };
                let dest = PathBuf::from(&destination_path);
                let parent = dest.parent().filter(|p| !p.as_os_str().is_empty());
                let Some(parent) = parent else {
                    return self
                        .fail_transfer(&store, &run.id, "download destination has no parent")
                        .await;
                };
                let staging_dir = match RelayTempDir::new_in(parent, &run.id) {
                    Ok(dir) => dir,
                    Err(error) => return self.fail_transfer(&store, &run.id, &error).await,
                };
                let connection = crate::ssh_hosts::SshConnection::from_execution_context(&source)?;
                let runner = self.runner.clone();
                let owner_id = self.owner_id.clone();
                let run_id = run.id.clone();
                let active = self.active.clone();
                let cleanup_id = run_id.clone();
                let task_store = store;
                let task = tokio::spawn(async move {
                    let result = local_download_lifecycle(
                        &task_store,
                        &owner_id,
                        &run_id,
                        runner,
                        staging_dir,
                        connection,
                        source_path,
                        dest,
                        local_transport_choice(&transport),
                        timeout,
                        started,
                    )
                    .await;
                    if let Err(error) = result {
                        tracing::warn!(run_id, "reclaimed download failed: {error}");
                    }
                });
                self.active.lock().await.insert(
                    cleanup_id.clone(),
                    ActiveRun {
                        abort: task.abort_handle(),
                    },
                );
                tokio::spawn(async move {
                    let _ = task.await;
                    active.lock().await.remove(&cleanup_id);
                });
            }
            TransferHandle::Relay {
                source_context_id,
                source_path,
                destination_context_id,
                destination_path,
            } => {
                let source = store
                    .get_execution_context(&source_context_id)
                    .await
                    .map_err(|e| e.to_string())?;
                let destination = store
                    .get_execution_context(&destination_context_id)
                    .await
                    .map_err(|e| e.to_string())?;
                let (Some(source), Some(destination)) = (source, destination) else {
                    return self
                        .fail_transfer(&store, &run.id, "relay context is gone")
                        .await;
                };
                let source_conn = crate::ssh_hosts::SshConnection::from_execution_context(&source)?;
                let dest_conn =
                    crate::ssh_hosts::SshConnection::from_execution_context(&destination)?;
                let relay_dir = match RelayTempDir::new(&run.id) {
                    Ok(dir) => dir,
                    Err(error) => return self.fail_transfer(&store, &run.id, &error).await,
                };
                let runner = self.runner.clone();
                let owner_id = self.owner_id.clone();
                let run_id = run.id.clone();
                let active = self.active.clone();
                let cleanup_id = run_id.clone();
                let task_store = store;
                let task = tokio::spawn(async move {
                    let result = relay_lifecycle(
                        &task_store,
                        &owner_id,
                        &run_id,
                        runner,
                        relay_dir,
                        source_conn,
                        source_path,
                        dest_conn,
                        destination_path,
                        timeout,
                        started,
                    )
                    .await;
                    if let Err(error) = result {
                        tracing::warn!(run_id, "reclaimed relay failed: {error}");
                    }
                });
                self.active.lock().await.insert(
                    cleanup_id.clone(),
                    ActiveRun {
                        abort: task.abort_handle(),
                    },
                );
                tokio::spawn(async move {
                    let _ = task.await;
                    active.lock().await.remove(&cleanup_id);
                });
            }
            TransferHandle::Harvest { parent_run_id } => {
                self.fail_transfer(
                    &store,
                    &run.id,
                    &format!("harvest transfer interrupted; retry harvest_run on {parent_run_id}"),
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn fail_transfer(
        &self,
        store: &wisp_store::Store,
        run_id: &str,
        error: &str,
    ) -> Result<(), String> {
        let _ = store
            .record_run_poll_owned(run_id, &self.owner_id, None, None, Some(error))
            .await;
        let _ = store
            .finish_active_run_owned(
                run_id,
                &self.owner_id,
                wisp_store::RunStatus::Failed,
                Some(-1),
            )
            .await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn submit_local_upload_to_ssh(
        &self,
        store: wisp_store::Store,
        project_id: &str,
        frame_id: Option<&str>,
        source_path: &Path,
        destination: &wisp_store::ExecutionContext,
        destination_path: &str,
        transport: TransferTransport,
        resume: bool,
        timeout: Duration,
    ) -> Result<SubmitRunResponse, String> {
        let destination_connection =
            crate::ssh_hosts::SshConnection::from_execution_context(destination)?;
        let item_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "local source_path has no portable item name".to_string())?
            .to_string();
        let run_id = uuid::Uuid::new_v4().to_string();
        let started = Instant::now();
        let mut run = wisp_store::RunRecord::new(
            &run_id,
            project_id,
            &destination.id,
            format!("Upload {item_name} to {}", destination.label),
            "file_transfer",
        );
        run.frame_id = frame_id.map(Into::into);
        run.command = Some(format!(
            "upload local:{} -> {}:{}",
            source_path.display(),
            destination.id,
            destination_path
        ));
        run.timeout_secs = Some(timeout.as_secs() as i64);
        run.progress_json = serde_json::to_string(&transfer_progress(
            "upload",
            "uploading",
            0,
            0,
            0,
            0,
            Some(item_name),
            started,
        ))
        .map_err(|error| error.to_string())?;
        run.env_snapshot_json = serde_json::json!({
            "route": "local",
            "transport": transport_label(transport),
            "source_context_id": "local",
            "destination_context_id": destination.id,
            "source_path": source_path,
            "destination_path": destination_path
        })
        .to_string();
        store
            .create_run(&run)
            .await
            .map_err(|error| error.to_string())?;
        if !store
            .activate_run_lifecycle(
                &run_id,
                wisp_store::RunStatus::Submitted,
                &self.owner_id,
                ACTIVE_LEASE_SECS,
            )
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("Upload Run changed state before it could start".into());
        }
        persist_transfer_handle(
            &store,
            &self.owner_id,
            &run_id,
            &TransferHandle::LocalUpload {
                source_path: source_path.display().to_string(),
                destination_context_id: destination.id.clone(),
                destination_path: destination_path.to_string(),
                transport: transport_label(transport).into(),
                resume,
            },
        )
        .await?;

        let runner = self.runner.clone();
        let owner_id = self.owner_id.clone();
        let active = self.active.clone();
        let cleanup_id = run_id.clone();
        let task_run_id = run_id.clone();
        let source_path = source_path.to_path_buf();
        let destination_path = destination_path.to_string();
        let task_store = store.clone();
        let task = tokio::spawn(async move {
            let result = local_upload_lifecycle(
                &task_store,
                &owner_id,
                &task_run_id,
                runner,
                source_path,
                destination_connection,
                destination_path,
                transport,
                resume,
                timeout,
                started,
            )
            .await;
            if let Err(error) = result {
                tracing::warn!(run_id = %task_run_id, "local upload lifecycle failed: {error}");
            }
        });
        let abort = task.abort_handle();
        self.active
            .lock()
            .await
            .insert(run_id.clone(), ActiveRun { abort });
        tokio::spawn(async move {
            let _ = task.await;
            active.lock().await.remove(&cleanup_id);
        });
        Ok(SubmitRunResponse {
            run_id,
            status: wisp_store::RunStatus::Submitted,
            exit_code: None,
            stdout_tail: None,
            stderr_tail: None,
            remote_workdir: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn submit_ssh_download_to_local(
        &self,
        store: wisp_store::Store,
        project_id: &str,
        frame_id: Option<&str>,
        source: &wisp_store::ExecutionContext,
        source_path: &str,
        destination_path: &Path,
        transport: TransferTransport,
        timeout: Duration,
    ) -> Result<SubmitRunResponse, String> {
        if destination_path.exists() {
            return Err(format!(
                "local destination_path already exists: {}",
                destination_path.display()
            ));
        }
        let destination_parent = destination_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| "local destination_path must have a parent directory".to_string())?;
        std::fs::create_dir_all(destination_parent)
            .map_err(|error| format!("create local destination directory: {error}"))?;

        let source_connection = crate::ssh_hosts::SshConnection::from_execution_context(source)?;
        let item_name = remote_item_name(source_path)?.to_string();
        let run_id = uuid::Uuid::new_v4().to_string();
        let staging_dir = RelayTempDir::new_in(destination_parent, &run_id)?;
        let started = Instant::now();
        let mut run = wisp_store::RunRecord::new(
            &run_id,
            project_id,
            &source.id,
            format!("Download {item_name} from {}", source.label),
            "file_transfer",
        );
        run.frame_id = frame_id.map(Into::into);
        run.command = Some(format!(
            "download {}:{} -> local:{}",
            source.id,
            source_path,
            destination_path.display()
        ));
        run.timeout_secs = Some(timeout.as_secs() as i64);
        run.progress_json = serde_json::to_string(&transfer_progress(
            "download",
            "downloading",
            0,
            0,
            0,
            0,
            Some(item_name),
            started,
        ))
        .map_err(|error| error.to_string())?;
        run.env_snapshot_json = serde_json::json!({
            "route": "local",
            "transport": transport_label(transport),
            "source_context_id": source.id,
            "destination_context_id": "local",
            "destination_path": destination_path
        })
        .to_string();
        store
            .create_run(&run)
            .await
            .map_err(|error| error.to_string())?;
        if !store
            .activate_run_lifecycle(
                &run_id,
                wisp_store::RunStatus::Submitted,
                &self.owner_id,
                ACTIVE_LEASE_SECS,
            )
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("Download Run changed state before it could start".into());
        }
        persist_transfer_handle(
            &store,
            &self.owner_id,
            &run_id,
            &TransferHandle::LocalDownload {
                source_context_id: source.id.clone(),
                source_path: source_path.to_string(),
                destination_path: destination_path.display().to_string(),
                transport: transport_label(transport).into(),
            },
        )
        .await?;

        let runner = self.runner.clone();
        let owner_id = self.owner_id.clone();
        let active = self.active.clone();
        let cleanup_id = run_id.clone();
        let task_run_id = run_id.clone();
        let source_path = source_path.trim_end_matches('/').to_string();
        let destination_path = destination_path.to_path_buf();
        let task_store = store.clone();
        let task = tokio::spawn(async move {
            let result = local_download_lifecycle(
                &task_store,
                &owner_id,
                &task_run_id,
                runner,
                staging_dir,
                source_connection,
                source_path,
                destination_path,
                transport,
                timeout,
                started,
            )
            .await;
            if let Err(error) = result {
                tracing::warn!(run_id = %task_run_id, "local download lifecycle failed: {error}");
            }
        });
        let abort = task.abort_handle();
        self.active
            .lock()
            .await
            .insert(run_id.clone(), ActiveRun { abort });
        tokio::spawn(async move {
            let _ = task.await;
            active.lock().await.remove(&cleanup_id);
        });
        Ok(SubmitRunResponse {
            run_id,
            status: wisp_store::RunStatus::Submitted,
            exit_code: None,
            stdout_tail: None,
            stderr_tail: None,
            remote_workdir: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn submit_ssh_relay(
        &self,
        store: wisp_store::Store,
        project_id: &str,
        frame_id: Option<&str>,
        source: &wisp_store::ExecutionContext,
        source_path: &str,
        destination: &wisp_store::ExecutionContext,
        destination_path: &str,
        timeout: Duration,
    ) -> Result<SubmitRunResponse, String> {
        let source_connection = crate::ssh_hosts::SshConnection::from_execution_context(source)?;
        let destination_connection =
            crate::ssh_hosts::SshConnection::from_execution_context(destination)?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let started = Instant::now();
        let mut run = wisp_store::RunRecord::new(
            &run_id,
            project_id,
            &source.id,
            format!("Relay {} → {}", source.label, destination.label),
            "file_transfer",
        );
        run.frame_id = frame_id.map(Into::into);
        run.command = Some(format!(
            "relay {}:{} -> {}:{}",
            source.id, source_path, destination.id, destination_path
        ));
        run.timeout_secs = Some(timeout.as_secs() as i64);
        run.progress_json = serde_json::to_string(&transfer_progress(
            "relay",
            "downloading",
            0,
            0,
            0,
            0,
            None,
            started,
        ))
        .map_err(|error| error.to_string())?;
        run.env_snapshot_json = serde_json::json!({
            "route": "relay",
            "transport": "scp",
            "source_context_id": source.id,
            "destination_context_id": destination.id
        })
        .to_string();
        let relay_dir = RelayTempDir::new(&run_id)?;
        store
            .create_run(&run)
            .await
            .map_err(|error| error.to_string())?;
        if !store
            .activate_run_lifecycle(
                &run_id,
                wisp_store::RunStatus::Submitted,
                &self.owner_id,
                ACTIVE_LEASE_SECS,
            )
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("Relay Run changed state before it could start".into());
        }
        persist_transfer_handle(
            &store,
            &self.owner_id,
            &run_id,
            &TransferHandle::Relay {
                source_context_id: source.id.clone(),
                source_path: source_path.to_string(),
                destination_context_id: destination.id.clone(),
                destination_path: destination_path.to_string(),
            },
        )
        .await?;

        let runner = self.runner.clone();
        let owner_id = self.owner_id.clone();
        let active = self.active.clone();
        let cleanup_id = run_id.clone();
        let task_run_id = run_id.clone();
        let source_path = source_path.trim_end_matches('/').to_string();
        let destination_path = destination_path.to_string();
        let task_store = store.clone();
        let task = tokio::spawn(async move {
            let result = relay_lifecycle(
                &task_store,
                &owner_id,
                &task_run_id,
                runner,
                relay_dir,
                source_connection,
                source_path,
                destination_connection,
                destination_path,
                timeout,
                started,
            )
            .await;
            if let Err(error) = result {
                tracing::warn!(run_id = %task_run_id, "relay transfer lifecycle failed: {error}");
            }
        });
        let abort = task.abort_handle();
        self.active
            .lock()
            .await
            .insert(run_id.clone(), ActiveRun { abort });
        tokio::spawn(async move {
            let _ = task.await;
            active.lock().await.remove(&cleanup_id);
        });
        Ok(SubmitRunResponse {
            run_id,
            status: wisp_store::RunStatus::Submitted,
            exit_code: None,
            stdout_tail: None,
            stderr_tail: None,
            remote_workdir: None,
        })
    }
}

struct RelayTempDir(PathBuf);

impl RelayTempDir {
    fn new(run_id: &str) -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("wisp-relay-{run_id}"));
        Self::create(path)
    }

    fn new_in(parent: &Path, run_id: &str) -> Result<Self, String> {
        Self::create(parent.join(format!(".wisp-transfer-{run_id}")))
    }

    fn create(path: PathBuf) -> Result<Self, String> {
        std::fs::create_dir(&path).map_err(|error| format!("create relay directory: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("secure relay directory: {error}"))?;
        }
        Ok(Self(path))
    }
}

impl Drop for RelayTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[allow(clippy::too_many_arguments)]
async fn download_remote_item(
    store: &wisp_store::Store,
    owner_id: &str,
    run_id: &str,
    runner: &dyn super::RunCommandRunner,
    staging_dir: &Path,
    source: &crate::ssh_hosts::SshConnection,
    source_path: &str,
    timeout: Duration,
    script: &str,
) -> Result<(super::RunCommandOutput, PathBuf, u64, u64), String> {
    let mut download_args = source.scp_option_args()?;
    download_args.push("-r".into());
    download_args.push(format!("{}:{source_path}", source.target()?));
    download_args.push(scp_local_path(staging_dir));
    let download = checked_output(
        "SSH download",
        run_with_lifecycle_lease(
            store,
            run_id,
            owner_id,
            runner,
            RunCommand {
                context_id: format!("ssh:{}", source.alias),
                program: "scp".into(),
                args: download_args,
                script: script.into(),
                cwd: Some(staging_dir.to_path_buf()),
                stdin: None,
                envs: crate::ssh_hosts::auth_envs_for_connection(source)?,
            },
            timeout,
        )
        .await,
    )?;
    let local_item = single_relay_item(staging_dir)?;
    let (total_bytes, files_total) = relay_item_stats(&local_item)?;
    Ok((download, local_item, total_bytes, files_total))
}

const RSYNC_PROBE_MARKER: &str = "__WISP_RSYNC__";

fn rsync_probe_line(transport: TransferTransport) -> &'static str {
    match transport {
        TransferTransport::Scp => "",
        TransferTransport::Rsync => {
            "command -v rsync >/dev/null 2>&1 && printf '__WISP_RSYNC__:yes\\n' || printf '__WISP_RSYNC__:no\\n'\n"
        }
    }
}

/// rsync's --protect-args disables remote tilde expansion, so `~/` prefixes
/// become plain home-relative paths.
fn rsync_remote_path(path: &str) -> String {
    match path {
        "~" => ".".into(),
        _ => path.strip_prefix("~/").unwrap_or(path).to_string(),
    }
}

fn rsync_base_args(connection: &crate::ssh_hosts::SshConnection) -> Result<Vec<String>, String> {
    Ok(vec![
        "-a".into(),
        "-s".into(),
        "--partial".into(),
        "-e".into(),
        connection.rsync_rsh()?,
    ])
}

/// The remote side answered the probe in a pre-transfer RPC; the local side
/// is probed by running `rsync --version`.
async fn require_rsync_available(
    runner: &dyn super::RunCommandRunner,
    connection: &crate::ssh_hosts::SshConnection,
    probe_stdout: &str,
) -> Result<(), String> {
    if !probe_stdout.contains(&format!("{RSYNC_PROBE_MARKER}:yes")) {
        return Err(format!(
            "rsync is not installed on {}; use transport=scp",
            connection.alias
        ));
    }
    let probe = runner
        .run(
            RunCommand {
                context_id: "local".into(),
                program: "rsync".into(),
                args: vec!["--version".into()],
                script: "probe local rsync".into(),
                cwd: None,
                stdin: None,
                envs: Vec::new(),
            },
            REMOTE_RPC_TIMEOUT,
        )
        .await;
    if !probe.map(|output| output.exit_code == 0).unwrap_or(false) {
        return Err("rsync is not installed on this machine; use transport=scp".into());
    }
    Ok(())
}

/// Download through a deterministic hidden partial directory next to the
/// destination: an interrupted transfer leaves it behind, and the retried
/// rsync continues from the partial data instead of starting over.
#[allow(clippy::too_many_arguments)]
async fn rsync_download_into_partial(
    store: &wisp_store::Store,
    owner_id: &str,
    run_id: &str,
    runner: &dyn super::RunCommandRunner,
    source: &crate::ssh_hosts::SshConnection,
    source_path: &str,
    destination_path: &Path,
    timeout: Duration,
) -> Result<(super::RunCommandOutput, PathBuf, u64, u64), String> {
    let probe = checked_output(
        "Check download source",
        run_with_lifecycle_lease(
            store,
            run_id,
            owner_id,
            runner,
            ssh_script_command(
                source,
                "check rsync download source",
                format!("set -eu\n{}", rsync_probe_line(TransferTransport::Rsync)),
            )?,
            timeout,
        )
        .await,
    )?;
    require_rsync_available(runner, source, &probe.stdout).await?;
    let file_name = destination_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "local destination_path has no portable item name".to_string())?;
    let partial_dir = destination_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "local destination_path must have a parent directory".to_string())?
        .join(format!(".wisp-partial-{file_name}"));
    std::fs::create_dir_all(&partial_dir)
        .map_err(|error| format!("create partial download directory: {error}"))?;
    let mut args = rsync_base_args(source)?;
    args.push(format!(
        "{}:{}",
        source.target()?,
        rsync_remote_path(source_path)
    ));
    args.push(scp_local_path(&partial_dir));
    let download = checked_output(
        "SSH download",
        run_with_lifecycle_lease(
            store,
            run_id,
            owner_id,
            runner,
            RunCommand {
                context_id: format!("ssh:{}", source.alias),
                program: "rsync".into(),
                args,
                script: "local download".into(),
                cwd: None,
                stdin: None,
                envs: crate::ssh_hosts::auth_envs_for_connection(source)?,
            },
            timeout,
        )
        .await,
    )?;
    let local_item = single_relay_item(&partial_dir)?;
    let (total_bytes, files_total) = relay_item_stats(&local_item)?;
    Ok((download, local_item, total_bytes, files_total))
}

#[allow(clippy::too_many_arguments)]
async fn local_upload_lifecycle(
    store: &wisp_store::Store,
    owner_id: &str,
    run_id: &str,
    runner: Arc<dyn super::RunCommandRunner>,
    source_path: PathBuf,
    destination: crate::ssh_hosts::SshConnection,
    destination_path: String,
    transport: TransferTransport,
    resume: bool,
    timeout: Duration,
    started: Instant,
) -> Result<(), String> {
    if !store
        .transition_run_to_running_owned(run_id, owner_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(());
    }
    let result = async {
        let (total_bytes, files_total) = relay_item_stats(&source_path)?;
        // With resume the partially written destination is expected; rsync
        // continues from it instead of refusing it.
        let exists_guard = if resume {
            // A crashed attempt may have left a partial file. Remove it
            // before scp; rsync --partial continues from whatever is there.
            if transport == TransferTransport::Scp {
                "rm -rf \"$dst\"\n"
            } else {
                ""
            }
        } else {
            "[ ! -e \"$dst\" ] || { echo 'remote destination_path already exists' >&2; exit 73; }\n"
        };
        let destination_check = format!(
            "set -eu\n{}\n{}{exists_guard}",
            remote_path_assignment("dst", &destination_path),
            rsync_probe_line(transport),
        );
        let check = checked_output(
            "Check upload destination",
            run_with_lifecycle_lease(
                store,
                run_id,
                owner_id,
                runner.as_ref(),
                ssh_script_command(
                    &destination,
                    "check local upload destination",
                    destination_check,
                )?,
                timeout,
            )
            .await,
        )?;
        if transport == TransferTransport::Rsync {
            require_rsync_available(runner.as_ref(), &destination, &check.stdout).await?;
        }
        // Ledger the destination *before* bytes move so a crash or cancel
        // leaves an orphan the user can see and delete — not a silent partial.
        if let Err(error) = ledger_upload_attempt(
            store,
            run_id,
            &destination.alias,
            &destination_path,
            i64::try_from(total_bytes).ok(),
        )
        .await
        {
            tracing::warn!(run_id, "remote staging ledger write failed: {error}");
        }
        let remaining = timeout
            .checked_sub(started.elapsed())
            .ok_or_else(|| format!("run_in_context timed out after {}s", timeout.as_secs()))?;
        let upload_command = match transport {
            TransferTransport::Scp => {
                let mut upload_args = destination.scp_option_args()?;
                if source_path.is_dir() {
                    upload_args.push("-r".into());
                }
                upload_args.push(scp_local_path(&source_path));
                upload_args.push(format!("{}:{destination_path}", destination.target()?));
                RunCommand {
                    context_id: format!("ssh:{}", destination.alias),
                    program: "scp".into(),
                    args: upload_args,
                    script: "local upload".into(),
                    cwd: source_path.parent().map(Path::to_path_buf),
                    stdin: None,
                    envs: crate::ssh_hosts::auth_envs_for_connection(&destination)?,
                }
            }
            TransferTransport::Rsync => {
                let mut args = rsync_base_args(&destination)?;
                let remote = rsync_remote_path(&destination_path);
                if source_path.is_dir() {
                    // Trailing slashes copy contents into `dst` itself, so
                    // rsync creates the same layout scp -r would.
                    args.push(format!(
                        "{}/",
                        scp_local_path(&source_path).trim_end_matches('/')
                    ));
                    args.push(format!("{}:{remote}/", destination.target()?));
                } else {
                    args.push(scp_local_path(&source_path));
                    args.push(format!("{}:{remote}", destination.target()?));
                }
                RunCommand {
                    context_id: format!("ssh:{}", destination.alias),
                    program: "rsync".into(),
                    args,
                    script: "local upload".into(),
                    cwd: source_path.parent().map(Path::to_path_buf),
                    stdin: None,
                    envs: crate::ssh_hosts::auth_envs_for_connection(&destination)?,
                }
            }
        };
        let upload = checked_output(
            "SSH upload",
            run_with_lifecycle_lease(
                store,
                run_id,
                owner_id,
                runner.as_ref(),
                upload_command,
                remaining,
            )
            .await,
        )?;
        Ok::<_, String>((upload, total_bytes, files_total))
    }
    .await;

    let (status, exit_code, stdout, stderr, progress) = match result {
        Ok((upload, total_bytes, files_total)) => (
            wisp_store::RunStatus::Succeeded,
            Some(0),
            upload.stdout,
            upload.stderr,
            transfer_progress(
                "upload",
                "uploaded",
                total_bytes,
                total_bytes,
                files_total,
                files_total,
                source_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(Into::into),
                started,
            ),
        ),
        Err(error) if error == "run_in_context cancelled" => (
            wisp_store::RunStatus::Cancelled,
            None,
            String::new(),
            error,
            transfer_progress("upload", "cancelled", 0, 0, 0, 0, None, started),
        ),
        Err(error) if error.starts_with("run_in_context timed out after ") => (
            wisp_store::RunStatus::TimedOut,
            Some(124),
            String::new(),
            error,
            transfer_progress("upload", "failed", 0, 0, 0, 0, None, started),
        ),
        Err(error) => (
            wisp_store::RunStatus::Failed,
            Some(-1),
            String::new(),
            error,
            transfer_progress("upload", "failed", 0, 0, 0, 0, None, started),
        ),
    };
    let _ = store
        .update_run_progress_owned(run_id, owner_id, &progress)
        .await;
    let _ = store
        .update_run_output_owned(run_id, owner_id, Some(&tail(&stdout)), Some(&tail(&stderr)))
        .await;
    let _ = store
        .finish_active_run_owned(run_id, owner_id, status, exit_code)
        .await
        .map_err(|error| error.to_string())?;
    // Success keeps the attempt row (it is already the current file). Failure
    // leaves it as an orphan so partials can be cleaned. A crash before this
    // point still has the attempt row from `ledger_upload_attempt`.
    if status == wisp_store::RunStatus::Succeeded {
        if let Err(error) = ledger_upload_attempt(
            store,
            run_id,
            &destination.alias,
            &destination_path,
            i64::try_from(progress.total_bytes).ok(),
        )
        .await
        {
            tracing::warn!(run_id, "remote staging ledger write failed: {error}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn local_download_lifecycle(
    store: &wisp_store::Store,
    owner_id: &str,
    run_id: &str,
    runner: Arc<dyn super::RunCommandRunner>,
    staging_dir: RelayTempDir,
    source: crate::ssh_hosts::SshConnection,
    source_path: String,
    destination_path: PathBuf,
    transport: TransferTransport,
    timeout: Duration,
    started: Instant,
) -> Result<(), String> {
    if !store
        .transition_run_to_running_owned(run_id, owner_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(());
    }
    let result = async {
        let (download, local_item, total_bytes, files_total) = match transport {
            TransferTransport::Scp => {
                download_remote_item(
                    store,
                    owner_id,
                    run_id,
                    runner.as_ref(),
                    &staging_dir.0,
                    &source,
                    &source_path,
                    timeout,
                    "local download",
                )
                .await?
            }
            TransferTransport::Rsync => {
                rsync_download_into_partial(
                    store,
                    owner_id,
                    run_id,
                    runner.as_ref(),
                    &source,
                    &source_path,
                    &destination_path,
                    timeout,
                )
                .await?
            }
        };
        if destination_path.exists() {
            return Err(format!(
                "local destination_path appeared during transfer and was not overwritten: {}",
                destination_path.display()
            ));
        }
        std::fs::rename(&local_item, &destination_path)
            .map_err(|error| format!("finalize local download: {error}"))?;
        if transport == TransferTransport::Rsync {
            if let Some(partial_dir) = local_item.parent() {
                let _ = std::fs::remove_dir_all(partial_dir);
            }
        }
        Ok::<_, String>((download, total_bytes, files_total))
    }
    .await;

    let (status, exit_code, stdout, stderr, progress) = match result {
        Ok((download, total_bytes, files_total)) => (
            wisp_store::RunStatus::Succeeded,
            Some(0),
            download.stdout,
            download.stderr,
            transfer_progress(
                "download",
                "downloaded",
                total_bytes,
                total_bytes,
                files_total,
                files_total,
                destination_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(Into::into),
                started,
            ),
        ),
        Err(error) if error == "run_in_context cancelled" => (
            wisp_store::RunStatus::Cancelled,
            None,
            String::new(),
            error,
            transfer_progress("download", "cancelled", 0, 0, 0, 0, None, started),
        ),
        Err(error) if error.starts_with("run_in_context timed out after ") => (
            wisp_store::RunStatus::TimedOut,
            Some(124),
            String::new(),
            error,
            transfer_progress("download", "failed", 0, 0, 0, 0, None, started),
        ),
        Err(error) => (
            wisp_store::RunStatus::Failed,
            Some(-1),
            String::new(),
            error,
            transfer_progress("download", "failed", 0, 0, 0, 0, None, started),
        ),
    };
    let _ = store
        .update_run_progress_owned(run_id, owner_id, &progress)
        .await;
    let _ = store
        .update_run_output_owned(run_id, owner_id, Some(&tail(&stdout)), Some(&tail(&stderr)))
        .await;
    let _ = store
        .finish_active_run_owned(run_id, owner_id, status, exit_code)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn relay_lifecycle(
    store: &wisp_store::Store,
    owner_id: &str,
    run_id: &str,
    runner: Arc<dyn super::RunCommandRunner>,
    relay_dir: RelayTempDir,
    source: crate::ssh_hosts::SshConnection,
    source_path: String,
    destination: crate::ssh_hosts::SshConnection,
    destination_path: String,
    timeout: Duration,
    started: Instant,
) -> Result<(), String> {
    if !store
        .transition_run_to_running_owned(run_id, owner_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(());
    }
    let result = async {
        let (download, local_item, total_bytes, files_total) = download_remote_item(
            store,
            owner_id,
            run_id,
            runner.as_ref(),
            &relay_dir.0,
            &source,
            &source_path,
            timeout,
            "relay download",
        )
        .await?;
        let uploading = transfer_progress(
            "relay",
            "uploading",
            0,
            total_bytes,
            0,
            files_total,
            local_item
                .file_name()
                .and_then(|name| name.to_str())
                .map(Into::into),
            started,
        );
        if !store
            .update_run_progress_owned(run_id, owner_id, &uploading)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("Relay lifecycle lease expired before upload".into());
        }
        if let Err(error) = ledger_upload_attempt(
            store,
            run_id,
            &destination.alias,
            &destination_path,
            i64::try_from(total_bytes).ok(),
        )
        .await
        {
            tracing::warn!(run_id, "remote staging ledger write failed: {error}");
        }
        let remaining = timeout
            .checked_sub(started.elapsed())
            .ok_or_else(|| format!("run_in_context timed out after {}s", timeout.as_secs()))?;
        let mut upload_args = destination.scp_option_args()?;
        if local_item.is_dir() {
            upload_args.push("-r".into());
        }
        upload_args.push(scp_local_path(&local_item));
        upload_args.push(format!("{}:{destination_path}", destination.target()?));
        let upload = checked_output(
            "Relay upload",
            run_with_lifecycle_lease(
                store,
                run_id,
                owner_id,
                runner.as_ref(),
                RunCommand {
                    context_id: format!("ssh:{}", destination.alias),
                    program: "scp".into(),
                    args: upload_args,
                    script: "relay upload".into(),
                    cwd: Some(relay_dir.0.clone()),
                    stdin: None,
                    envs: crate::ssh_hosts::auth_envs_for_connection(&destination)?,
                },
                remaining,
            )
            .await,
        )?;
        Ok::<_, String>((download, upload, total_bytes, files_total))
    }
    .await;

    let (status, exit_code, stdout, stderr, progress) = match result {
        Ok((download, upload, total_bytes, files_total)) => (
            wisp_store::RunStatus::Succeeded,
            Some(0),
            format!("{}\n{}", download.stdout, upload.stdout),
            format!("{}\n{}", download.stderr, upload.stderr),
            transfer_progress(
                "relay",
                "uploaded",
                total_bytes,
                total_bytes,
                files_total,
                files_total,
                None,
                started,
            ),
        ),
        Err(error) if error == "run_in_context cancelled" => (
            wisp_store::RunStatus::Cancelled,
            None,
            String::new(),
            error,
            transfer_progress("relay", "cancelled", 0, 0, 0, 0, None, started),
        ),
        Err(error) if error.starts_with("run_in_context timed out after ") => (
            wisp_store::RunStatus::TimedOut,
            Some(124),
            String::new(),
            error,
            transfer_progress("relay", "failed", 0, 0, 0, 0, None, started),
        ),
        Err(error) => (
            wisp_store::RunStatus::Failed,
            Some(-1),
            String::new(),
            error,
            transfer_progress("relay", "failed", 0, 0, 0, 0, None, started),
        ),
    };
    let _ = store
        .update_run_progress_owned(run_id, owner_id, &progress)
        .await;
    let _ = store
        .update_run_output_owned(run_id, owner_id, Some(&tail(&stdout)), Some(&tail(&stderr)))
        .await;
    let _ = store
        .finish_active_run_owned(run_id, owner_id, status, exit_code)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn single_relay_item(directory: &Path) -> Result<PathBuf, String> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| format!("read relay directory: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read relay item: {error}"))?;
    if entries.len() != 1 {
        return Err(format!(
            "Relay download produced {} top-level items; one exact source path was expected",
            entries.len()
        ));
    }
    Ok(entries.swap_remove(0).path())
}

fn relay_item_stats(path: &Path) -> Result<(u64, u64), String> {
    if path.is_file() {
        return std::fs::metadata(path)
            .map(|metadata| (metadata.len(), 1))
            .map_err(|error| format!("read relay file metadata: {error}"));
    }
    let mut bytes = 0_u64;
    let mut files = 0_u64;
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let entry = entry.map_err(|error| format!("walk relay directory: {error}"))?;
        if entry.file_type().is_file() {
            bytes = bytes.saturating_add(
                entry
                    .metadata()
                    .map_err(|error| format!("read relay file metadata: {error}"))?
                    .len(),
            );
            files += 1;
        }
    }
    Ok((bytes, files))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn remote_path_assignment(variable: &str, path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        format!("{variable}=\"$HOME\"/{}", shell_single_quote(rest))
    } else {
        format!("{variable}={}", shell_single_quote(path))
    }
}

#[cfg(test)]
mod tests {
    use super::super::RunCommandOutput;
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn public_key_parser_accepts_only_the_generated_ed25519_shape() {
        let encoded = "A".repeat(48);
        let key = parse_public_key(
            &format!("noise\n{PUBLIC_KEY_MARKER}ssh-ed25519 {encoded}\n"),
            "wisp:a:b",
        )
        .unwrap();
        assert_eq!(key, format!("ssh-ed25519 {encoded} wisp:a:b"));
        assert!(parse_public_key(
            &format!("{PUBLIC_KEY_MARKER}ssh-rsa {encoded}\n"),
            "wisp:a:b"
        )
        .is_err());
    }

    #[test]
    fn direct_transfer_auto_contains_rsync_and_scp_fallback() {
        let edge = SshTrustEdge {
            source_context_id: "ssh:a".into(),
            destination_context_id: "ssh:b".into(),
            destination_target: "bob@b.example".into(),
            destination_port: Some(2222),
            key_path: Some(".ssh/wisp-b-ed25519".into()),
            managed: true,
            verified_at: 1,
        };
        let script =
            direct_transfer_script("/data/source", "/data/destination", &edge, "auto").unwrap();
        assert!(script.contains("command -v rsync"));
        assert!(script.contains("selected=scp"));
        assert!(script.contains("rsync -a -s --partial"));
        assert!(script.contains("scp \"${scp_options[@]}\""));
        assert!(!script.contains("--delete"));
        let home_script =
            direct_transfer_script("/data/source", "~/destination", &edge, "scp").unwrap();
        assert!(home_script.contains("dst='~/destination'"));
        assert!(home_script.contains(r#"dst="$remote_home/${dst:2}""#));
    }

    #[test]
    fn transfer_paths_are_exact_and_not_roots_or_globs() {
        assert!(validate_remote_path("source", "/data/run-1").is_ok());
        assert!(validate_remote_path("source", "~/results/run 1").is_ok());
        for path in ["", "/", "~", "relative", "/data/*.csv", "/tmp/a\nb"] {
            assert!(validate_remote_path("source", path).is_err(), "{path:?}");
        }

        let root = std::env::temp_dir().join(format!("wisp_local_path_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(validate_local_destination(&root.join("new.bam").to_string_lossy()).is_ok());
        assert!(validate_local_destination("relative.bam").is_err());
        assert!(validate_local_destination(&root.to_string_lossy()).is_err());
        let existing = root.join("existing.bam");
        std::fs::write(&existing, b"keep").unwrap();
        assert!(validate_local_destination(&existing.to_string_lossy()).is_err());
        assert_eq!(
            validate_local_source(&existing.to_string_lossy()).unwrap(),
            existing
        );
        assert!(validate_local_source("relative.bam").is_err());
        assert!(validate_local_source(&root.join("missing.bam").to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    struct RecordingRunner {
        outputs: StdMutex<VecDeque<Result<RunCommandOutput, String>>>,
        commands: StdMutex<Vec<RunCommand>>,
    }

    #[async_trait::async_trait]
    impl super::super::RunCommandRunner for RecordingRunner {
        async fn run(
            &self,
            command: RunCommand,
            _timeout: Duration,
        ) -> Result<RunCommandOutput, String> {
            self.commands.lock().unwrap().push(command);
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err("unexpected command".into()))
        }
    }

    struct RelayRunner {
        commands: StdMutex<Vec<RunCommand>>,
    }

    #[async_trait::async_trait]
    impl super::super::RunCommandRunner for RelayRunner {
        async fn run(
            &self,
            command: RunCommand,
            _timeout: Duration,
        ) -> Result<RunCommandOutput, String> {
            if command.script.ends_with("download") {
                let directory = PathBuf::from(command.args.last().unwrap());
                std::fs::write(directory.join("result.txt"), b"relay bytes").unwrap();
            }
            self.commands.lock().unwrap().push(command);
            Ok(RunCommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    async fn test_store() -> (PathBuf, wisp_store::Store) {
        let root =
            std::env::temp_dir().join(format!("wisp_context_transfer_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = wisp_store::Store::open(&root.join("wisp.sqlite"))
            .await
            .unwrap();
        store
            .create_project("p", "project", &root.to_string_lossy())
            .await
            .unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        for (id, alias, host, user) in [
            ("ssh:a", "a", "a.example", "alice"),
            ("ssh:b", "b", "b.example", "bob"),
        ] {
            let mut context = wisp_store::ExecutionContext::new(id, alias).unwrap();
            context.config_json = serde_json::json!({
                "alias": alias,
                "host_name": host,
                "user": user
            })
            .to_string();
            context.last_probe_status = Some("ok".into());
            store.upsert_execution_context(&context).await.unwrap();
            store
                .set_session_execution_context_enabled("f", id, true)
                .await
                .unwrap();
        }
        (root, store)
    }

    #[tokio::test]
    async fn managed_trust_carries_only_the_public_key_between_contexts() {
        let (root, store) = test_store().await;
        let encoded = "A".repeat(48);
        let runner = RecordingRunner {
            outputs: StdMutex::new(
                vec![
                    Ok(RunCommandOutput {
                        exit_code: 0,
                        stdout: format!("{PUBLIC_KEY_MARKER}ssh-ed25519 {encoded}\n"),
                        stderr: String::new(),
                    }),
                    Ok(RunCommandOutput {
                        exit_code: 0,
                        stdout: "__WISP_TRUST_INSTALLED__\n".into(),
                        stderr: String::new(),
                    }),
                    Ok(RunCommandOutput {
                        exit_code: 0,
                        stdout: "__WISP_TRUST_VERIFIED__\n".into(),
                        stderr: String::new(),
                    }),
                ]
                .into(),
            ),
            commands: StdMutex::new(Vec::new()),
        };
        let edge = configure_trust(
            &store,
            &runner,
            Some("f"),
            &ConfigureTrustRequest {
                source_context_id: "ssh:a".into(),
                destination_context_id: "ssh:b".into(),
                action: "install".into(),
            },
        )
        .await
        .unwrap();

        assert!(edge.managed);
        assert_eq!(edge.key_path.as_deref(), Some(".ssh/wisp-b-ed25519"));
        let commands = runner.commands.lock().unwrap();
        assert_eq!(
            commands
                .iter()
                .map(|command| command.script.as_str())
                .collect::<Vec<_>>(),
            [
                "generate source transfer key",
                "install destination transfer key",
                "verify server-to-server SSH trust"
            ]
        );
        let install = commands[1].stdin.as_deref().unwrap();
        assert!(install.contains(&format!("ssh-ed25519 {encoded} wisp:a:b")));
        assert!(install.contains("authorized_keys"));
        assert!(!install.contains("PRIVATE KEY"));
        let verify = commands[2].stdin.as_deref().unwrap();
        assert!(verify.contains("$HOME/.ssh/wisp-b-ed25519"));
        drop(commands);
        assert_eq!(load_trust_edges(&store).await, vec![edge]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn revoke_removes_the_record_and_cleans_managed_keys_best_effort() {
        let (root, store) = test_store().await;
        let managed = SshTrustEdge {
            source_context_id: "ssh:a".into(),
            destination_context_id: "ssh:b".into(),
            destination_target: "bob@b.example".into(),
            destination_port: None,
            key_path: Some(".ssh/wisp-b-ed25519".into()),
            managed: true,
            verified_at: 1,
        };
        let verified = SshTrustEdge {
            source_context_id: "ssh:b".into(),
            destination_context_id: "ssh:a".into(),
            destination_target: "alice@a.example".into(),
            destination_port: None,
            key_path: None,
            managed: false,
            verified_at: 2,
        };
        save_trust_edge(&store, managed.clone()).await.unwrap();
        save_trust_edge(&store, verified.clone()).await.unwrap();

        let ok = || {
            Ok(RunCommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        };
        let runner = Arc::new(RecordingRunner {
            outputs: StdMutex::new(vec![ok(), ok()].into()),
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());

        let response = revoke_trust_edge(&store, &manager, "ssh:a", "ssh:b")
            .await
            .unwrap();
        assert_eq!(response.edges, vec![verified.clone()]);
        assert_eq!(response.cleanup_error, None);
        assert_eq!(load_trust_edges(&store).await, vec![verified.clone()]);
        {
            let commands = runner.commands.lock().unwrap();
            assert_eq!(
                commands
                    .iter()
                    .map(|command| command.script.as_str())
                    .collect::<Vec<_>>(),
                [
                    "remove destination transfer key",
                    "remove source transfer key"
                ]
            );
            let destination = commands[0].stdin.as_deref().unwrap();
            assert!(destination.contains("grep -Fv -- ' wisp:a:b'"));
            assert!(destination.contains("authorized_keys"));
            let source = commands[1].stdin.as_deref().unwrap();
            assert!(source.contains("rm -f \"$HOME/.ssh/wisp-b-ed25519\""));
        }

        // Unmanaged edge: record-only removal, no SSH round-trips.
        let response = revoke_trust_edge(&store, &manager, "ssh:b", "ssh:a")
            .await
            .unwrap();
        assert!(response.edges.is_empty());
        assert_eq!(response.cleanup_error, None);
        assert!(load_trust_edges(&store).await.is_empty());
        assert_eq!(runner.commands.lock().unwrap().len(), 2);

        // Unknown pair: idempotent no-op.
        let response = revoke_trust_edge(&store, &manager, "ssh:a", "ssh:b")
            .await
            .unwrap();
        assert!(response.edges.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn revoke_still_removes_the_record_when_cleanup_fails() {
        let (root, store) = test_store().await;
        let edge = SshTrustEdge {
            source_context_id: "ssh:a".into(),
            destination_context_id: "ssh:gone".into(),
            destination_target: "bob@gone.example".into(),
            destination_port: None,
            key_path: Some(".ssh/wisp-gone-ed25519".into()),
            managed: true,
            verified_at: 1,
        };
        save_trust_edge(&store, edge).await.unwrap();
        let runner = Arc::new(RecordingRunner {
            outputs: StdMutex::new(
                vec![Ok(RunCommandOutput {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                })]
                .into(),
            ),
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let response = revoke_trust_edge(&store, &manager, "ssh:a", "ssh:gone")
            .await
            .unwrap();
        assert!(response.edges.is_empty());
        assert!(load_trust_edges(&store).await.is_empty());
        // Destination context is gone → reported, while the source key file
        // cleanup still ran.
        let error = response.cleanup_error.unwrap();
        assert!(error.contains("destination"), "{error}");
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].script, "remove source transfer key");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn auto_route_relays_with_each_contexts_own_scp_connection() {
        let (root, store) = test_store().await;
        let runner = Arc::new(RelayRunner {
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let response = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "ssh:a".into(),
                source_path: "/data/result.txt".into(),
                destination_context_id: "ssh:b".into(),
                destination_path: Some("/results/".into()),
                route: "auto".into(),
                transport: "auto".into(),
                resume: false,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();
        assert_eq!(response["route"], "relay");
        let run_id = response["run_id"].as_str().unwrap();
        let run = loop {
            let run = store.get_run(run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
        assert_eq!(run.kind, "file_transfer");
        let progress: wisp_store::RunProgress = serde_json::from_str(&run.progress_json).unwrap();
        assert_eq!(progress.phase, "uploaded");
        assert_eq!(progress.completed_bytes, 11);
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].context_id, "ssh:a");
        assert_eq!(commands[0].script, "relay download");
        assert_eq!(commands[1].context_id, "ssh:b");
        assert_eq!(commands[1].script, "relay upload");
        assert!(commands[0]
            .args
            .iter()
            .any(|arg| arg == "alice@a.example:/data/result.txt"));
        assert!(commands[1]
            .args
            .iter()
            .any(|arg| arg == "bob@b.example:/results/"));
        drop(commands);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn relay_transfer_fails_when_the_scp_step_exits_nonzero() {
        let (root, store) = test_store().await;
        let runner = Arc::new(RecordingRunner {
            outputs: StdMutex::new(
                vec![Ok(RunCommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: "scp: /data/result.txt: No such file or directory".into(),
                })]
                .into(),
            ),
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let response = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "ssh:a".into(),
                source_path: "/data/result.txt".into(),
                destination_context_id: "ssh:b".into(),
                destination_path: Some("/results/".into()),
                route: "auto".into(),
                transport: "auto".into(),
                resume: false,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();
        assert_eq!(response["route"], "relay");
        let run_id = response["run_id"].as_str().unwrap();
        let run = loop {
            let run = store.get_run(run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(run.status, wisp_store::RunStatus::Failed);
        assert_eq!(run.kind, "file_transfer");
        assert_eq!(run.exit_code, Some(-1));
        let progress: wisp_store::RunProgress = serde_json::from_str(&run.progress_json).unwrap();
        assert_eq!(progress.phase, "failed");
        let stderr = run.stderr_tail.as_deref().unwrap();
        assert!(
            stderr.contains("SSH download failed with exit 1"),
            "{stderr}"
        );
        assert!(stderr.contains("No such file or directory"), "{stderr}");
        let commands = runner.commands.lock().unwrap();
        assert_eq!(
            commands.len(),
            1,
            "a failed download must stop before the upload step"
        );
        assert_eq!(commands[0].script, "relay download");
        drop(commands);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn local_source_uploads_to_selected_ssh_without_shell_transfer() {
        let (root, store) = test_store().await;
        let source = root.join("sample data.bam");
        std::fs::write(&source, b"bam bytes").unwrap();
        let runner = Arc::new(RelayRunner {
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let response = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "local".into(),
                source_path: source.to_string_lossy().into_owned(),
                destination_context_id: "ssh:a".into(),
                destination_path: Some("/results/sample data.bam".into()),
                route: "auto".into(),
                transport: "auto".into(),
                resume: false,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();
        assert_eq!(response["route"], "local");
        let run_id = response["run_id"].as_str().unwrap();
        let run = loop {
            let run = store.get_run(run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
        assert_eq!(run.kind, "file_transfer");
        let progress: wisp_store::RunProgress = serde_json::from_str(&run.progress_json).unwrap();
        assert_eq!(progress.phase, "uploaded");
        assert_eq!(progress.completed_bytes, 9);
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].script, "check local upload destination");
        assert_eq!(commands[1].program, "scp");
        assert_eq!(commands[1].script, "local upload");
        assert!(commands[1]
            .args
            .iter()
            .any(|arg| arg == "alice@a.example:/results/sample data.bam"));
        assert!(commands[1]
            .args
            .iter()
            .any(|arg| arg.contains("sample data.bam")));
        drop(commands);
        let handle: TransferHandle =
            serde_json::from_str(run.remote_handle_json.as_deref().unwrap()).unwrap();
        assert!(matches!(handle, TransferHandle::LocalUpload { .. }));
        let staged = store
            .list_remote_staging("p", "ssh:a", false)
            .await
            .unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].source, "transfer");
        assert_eq!(staged[0].remote_path, "/results/sample data.bam");
        let files = crate::run_context::remote_files::list_remote_files(&store, "p", "ssh:a")
            .await
            .unwrap();
        assert_eq!(
            files[0].state,
            crate::run_context::remote_files::RemoteFileState::Active
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn failed_upload_is_ledgered_as_orphan() {
        let (root, store) = test_store().await;
        let source = root.join("sample.bam");
        std::fs::write(&source, b"bam bytes").unwrap();
        let runner = Arc::new(RecordingRunner {
            outputs: StdMutex::new(
                vec![
                    Ok(RunCommandOutput {
                        exit_code: 0,
                        stdout: String::new(),
                        stderr: String::new(),
                    }),
                    Ok(RunCommandOutput {
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: "scp: failed".into(),
                    }),
                ]
                .into(),
            ),
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner);
        let response = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "local".into(),
                source_path: source.to_string_lossy().into_owned(),
                destination_context_id: "ssh:a".into(),
                destination_path: Some("/results/sample.bam".into()),
                route: "auto".into(),
                transport: "auto".into(),
                resume: false,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();
        let run_id = response["run_id"].as_str().unwrap();
        let run = loop {
            let run = store.get_run(run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(run.status, wisp_store::RunStatus::Failed);
        let files = crate::run_context::remote_files::list_remote_files(&store, "p", "ssh:a")
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].state,
            crate::run_context::remote_files::RemoteFileState::Orphan
        );
        assert_eq!(files[0].remote_path, "/results/sample.bam");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn upload_without_destination_defaults_to_the_remote_data_root() {
        let (root, store) = test_store().await;
        let source = root.join("sample data.bam");
        std::fs::write(&source, b"bam bytes").unwrap();
        let runner = Arc::new(RelayRunner {
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let response = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "local".into(),
                source_path: source.to_string_lossy().into_owned(),
                destination_context_id: "ssh:a".into(),
                destination_path: None,
                route: "auto".into(),
                transport: "auto".into(),
                resume: false,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();
        // Project "project" → default remote data root ~/wisp/project/data.
        assert_eq!(
            response["destination_path"].as_str().unwrap(),
            "~/wisp/project/data/sample data.bam"
        );
        let run_id = response["run_id"].as_str().unwrap().to_string();
        loop {
            let run = store.get_run(&run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let commands = runner.commands.lock().unwrap();
        assert!(commands.iter().any(|command| command
            .args
            .iter()
            .any(|arg| arg == "alice@a.example:~/wisp/project/data/sample data.bam")));
        drop(commands);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn upload_default_destination_honors_stored_prefs() {
        let (root, store) = test_store().await;
        store
            .upsert_context_storage_prefs(&wisp_store::ContextStoragePrefs {
                project_id: "p".into(),
                context_id: "ssh:a".into(),
                remote_data_root: "/scratch/proj".into(),
                remote_workdir_root: ".wisp-science/runs".into(),
                local_results_dir: "remote/a".into(),
                created_at: 0,
                updated_at: 0,
            })
            .await
            .unwrap();
        let source = root.join("input.fasta");
        std::fs::write(&source, b">seq\nACGT\n").unwrap();
        let runner = Arc::new(RelayRunner {
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let response = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "local".into(),
                source_path: source.to_string_lossy().into_owned(),
                destination_context_id: "ssh:a".into(),
                destination_path: None,
                route: "auto".into(),
                transport: "auto".into(),
                resume: false,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            response["destination_path"].as_str().unwrap(),
            "/scratch/proj/input.fasta"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn download_without_destination_still_requires_an_explicit_path() {
        let (root, store) = test_store().await;
        let runner = Arc::new(RelayRunner {
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let error = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "ssh:a".into(),
                source_path: "/data/result.txt".into(),
                destination_context_id: "local".into(),
                destination_path: None,
                route: "auto".into(),
                transport: "auto".into(),
                resume: false,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("destination_path is required"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn ssh_source_downloads_to_exact_new_local_path() {
        let (root, store) = test_store().await;
        let runner = Arc::new(RelayRunner {
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let destination = root.join("igv").join("sample.bam");
        let response = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "ssh:a".into(),
                source_path: "/data/result.txt".into(),
                destination_context_id: "local".into(),
                destination_path: Some(destination.to_string_lossy().into_owned()),
                route: "auto".into(),
                transport: "auto".into(),
                resume: false,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();

        assert_eq!(response["route"], "local");
        assert_eq!(response["transport"], "scp");
        let run_id = response["run_id"].as_str().unwrap();
        let run = loop {
            let run = store.get_run(run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
        assert_eq!(run.kind, "file_transfer");
        assert_eq!(std::fs::read(&destination).unwrap(), b"relay bytes");
        let progress: wisp_store::RunProgress = serde_json::from_str(&run.progress_json).unwrap();
        assert_eq!(progress.phase, "downloaded");
        assert_eq!(progress.completed_bytes, 11);
        assert_eq!(progress.files_completed, 1);

        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].context_id, "ssh:a");
        assert_eq!(commands[0].script, "local download");
        assert!(commands[0]
            .args
            .iter()
            .any(|arg| arg == "alice@a.example:/data/result.txt"));
        drop(commands);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn failed_local_download_leaves_no_destination_or_staging_item() {
        let (root, store) = test_store().await;
        let runner = Arc::new(RecordingRunner {
            outputs: StdMutex::new(
                vec![Ok(RunCommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: "scp: /missing: No such file or directory".into(),
                })]
                .into(),
            ),
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let destination = root.join("failed").join("sample.bam");
        let response = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "ssh:a".into(),
                source_path: "/missing".into(),
                destination_context_id: "local".into(),
                destination_path: Some(destination.to_string_lossy().into_owned()),
                route: "auto".into(),
                transport: "scp".into(),
                resume: false,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();

        let run_id = response["run_id"].as_str().unwrap();
        let run = loop {
            let run = store.get_run(run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(run.status, wisp_store::RunStatus::Failed);
        assert!(!destination.exists());
        for _ in 0..20 {
            let staging_exists = std::fs::read_dir(destination.parent().unwrap())
                .unwrap()
                .flatten()
                .any(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".wisp-transfer-")
                });
            if !staging_exists {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            std::fs::read_dir(destination.parent().unwrap())
                .unwrap()
                .flatten()
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".wisp-transfer-")),
            "failed transfer staging directory was not cleaned"
        );
        assert_eq!(runner.commands.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn join_remote_upload_destination_covers_home_and_absolute_dirs() {
        assert_eq!(
            join_remote_upload_destination("~", "counts.csv").unwrap(),
            "~/counts.csv"
        );
        assert_eq!(
            join_remote_upload_destination("/home/research/", "counts.csv").unwrap(),
            "/home/research/counts.csv"
        );
        assert_eq!(
            join_remote_upload_destination("/", "counts.csv").unwrap(),
            "/counts.csv"
        );
        assert!(join_remote_upload_destination("relative", "a.csv").is_err());
        assert!(join_remote_upload_destination("~", "../escape").is_err());
        assert!(join_remote_upload_destination("~", "a/b.csv").is_err());
    }

    #[tokio::test]
    async fn ui_upload_submits_one_transfer_per_local_path() {
        let (root, store) = test_store().await;
        let source = root.join("counts.csv");
        std::fs::write(&source, b"a,b\n").unwrap();
        let runner = Arc::new(RelayRunner {
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let items = submit_local_uploads_to_context(
            &store,
            &manager,
            "p",
            Some("f"),
            "ssh:a",
            "/home/research",
            &[source.to_string_lossy().into_owned()],
        )
        .await
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].destination_path, "/home/research/counts.csv");
        assert!(!items[0].run_id.is_empty());
        let run_id = items[0].run_id.clone();
        loop {
            let run = store.get_run(&run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
                assert_eq!(run.kind, "file_transfer");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let commands = runner.commands.lock().unwrap();
        assert!(commands.iter().any(|command| command
            .args
            .iter()
            .any(|arg| arg == "alice@a.example:/home/research/counts.csv")));
        drop(commands);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn ui_upload_rejects_local_context_and_empty_paths() {
        let (root, store) = test_store().await;
        let manager = RunManager::with_runner(Arc::new(RelayRunner {
            commands: StdMutex::new(Vec::new()),
        }));
        let empty = submit_local_uploads_to_context(
            &store,
            &manager,
            "p",
            None,
            "ssh:a",
            "/home/research",
            &[],
        )
        .await
        .unwrap_err();
        assert!(empty.contains("at least one"), "{empty}");
        let local = submit_local_uploads_to_context(
            &store,
            &manager,
            "p",
            None,
            "local",
            "/home/research",
            &[root.join("missing.csv").to_string_lossy().into_owned()],
        )
        .await
        .unwrap_err();
        assert!(local.contains("SSH context"), "{local}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn resume_requires_a_local_route_with_rsync_transport() {
        let (root, store) = test_store().await;
        let manager = RunManager::with_runner(Arc::new(RelayRunner {
            commands: StdMutex::new(Vec::new()),
        }));
        let source = root.join("data.bin");
        std::fs::write(&source, b"payload").unwrap();
        let error = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "local".into(),
                source_path: source.to_string_lossy().into_owned(),
                destination_context_id: "ssh:a".into(),
                destination_path: Some("/results/data.bin".into()),
                route: "auto".into(),
                transport: "scp".into(),
                resume: true,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("transport=rsync"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn rsync_resume_upload_probes_both_ends_and_skips_the_exists_guard() {
        let (root, store) = test_store().await;
        let source = root.join("data.bin");
        std::fs::write(&source, b"payload").unwrap();
        let runner = Arc::new(RecordingRunner {
            outputs: StdMutex::new(
                vec![
                    Ok(RunCommandOutput {
                        exit_code: 0,
                        stdout: "__WISP_RSYNC__:yes\n".into(),
                        stderr: String::new(),
                    }),
                    Ok(RunCommandOutput {
                        exit_code: 0,
                        stdout: "rsync  version 3.2.7".into(),
                        stderr: String::new(),
                    }),
                    Ok(RunCommandOutput {
                        exit_code: 0,
                        stdout: String::new(),
                        stderr: String::new(),
                    }),
                ]
                .into(),
            ),
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let response = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "local".into(),
                source_path: source.to_string_lossy().into_owned(),
                destination_context_id: "ssh:a".into(),
                destination_path: Some("~/results/data.bin".into()),
                route: "auto".into(),
                transport: "rsync".into(),
                resume: true,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();
        assert_eq!(response["transport"], "rsync");
        let run_id = response["run_id"].as_str().unwrap();
        let run = loop {
            let run = store.get_run(run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0].script, "check local upload destination");
        let check_payload = commands[0].stdin.as_deref().unwrap();
        assert!(check_payload.contains("__WISP_RSYNC__"), "{check_payload}");
        // Resume keeps a partially uploaded destination instead of refusing it.
        assert!(!check_payload.contains("already exists"), "{check_payload}");
        assert_eq!(commands[1].program, "rsync");
        assert_eq!(commands[1].args, vec!["--version".to_string()]);
        assert_eq!(commands[2].program, "rsync");
        assert!(commands[2].args.contains(&"--partial".to_string()));
        // --protect-args disables remote tilde expansion, so ~/ is stripped.
        assert!(commands[2]
            .args
            .iter()
            .any(|arg| arg == "alice@a.example:results/data.bin"));
        drop(commands);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn rsync_upload_fails_fast_when_the_remote_lacks_rsync() {
        let (root, store) = test_store().await;
        let source = root.join("data.bin");
        std::fs::write(&source, b"payload").unwrap();
        let runner = Arc::new(RecordingRunner {
            outputs: StdMutex::new(
                vec![Ok(RunCommandOutput {
                    exit_code: 0,
                    stdout: "__WISP_RSYNC__:no\n".into(),
                    stderr: String::new(),
                })]
                .into(),
            ),
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let response = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "local".into(),
                source_path: source.to_string_lossy().into_owned(),
                destination_context_id: "ssh:a".into(),
                destination_path: Some("/results/data.bin".into()),
                route: "auto".into(),
                transport: "rsync".into(),
                resume: false,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();
        let run_id = response["run_id"].as_str().unwrap();
        let run = loop {
            let run = store.get_run(run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(run.status, wisp_store::RunStatus::Failed);
        let stderr = run.stderr_tail.unwrap_or_default();
        assert!(stderr.contains("rsync is not installed on a"), "{stderr}");
        assert_eq!(runner.commands.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Answers the remote rsync probe, then materializes the downloaded file
    /// inside the partial directory rsync would have written into.
    struct RsyncDownloadRunner {
        commands: StdMutex<Vec<RunCommand>>,
    }

    #[async_trait::async_trait]
    impl super::super::RunCommandRunner for RsyncDownloadRunner {
        async fn run(
            &self,
            command: RunCommand,
            _timeout: Duration,
        ) -> Result<RunCommandOutput, String> {
            let stdout = if command.script == "check rsync download source" {
                "__WISP_RSYNC__:yes\n".to_string()
            } else {
                if command.program == "rsync" && command.script == "local download" {
                    let directory = PathBuf::from(command.args.last().unwrap());
                    std::fs::write(directory.join("result.txt"), b"rsync bytes").unwrap();
                }
                String::new()
            };
            self.commands.lock().unwrap().push(command);
            Ok(RunCommandOutput {
                exit_code: 0,
                stdout,
                stderr: String::new(),
            })
        }
    }

    #[tokio::test]
    async fn rsync_download_lands_at_the_destination_and_clears_the_partial_dir() {
        let (root, store) = test_store().await;
        let runner = Arc::new(RsyncDownloadRunner {
            commands: StdMutex::new(Vec::new()),
        });
        let manager = RunManager::with_runner(runner.clone());
        let destination = root.join("igv").join("result.txt");
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        let response = submit_transfer(
            &store,
            &manager,
            "p",
            Some("f"),
            &root,
            TransferRequest {
                source_context_id: "ssh:a".into(),
                source_path: "/data/result.txt".into(),
                destination_context_id: "local".into(),
                destination_path: Some(destination.to_string_lossy().into_owned()),
                route: "auto".into(),
                transport: "rsync".into(),
                resume: false,
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();
        assert_eq!(response["transport"], "rsync");
        let run_id = response["run_id"].as_str().unwrap();
        let run = loop {
            let run = store.get_run(run_id).await.unwrap().unwrap();
            if run.status.is_terminal() {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(run.status, wisp_store::RunStatus::Succeeded);
        assert_eq!(std::fs::read(&destination).unwrap(), b"rsync bytes");
        assert!(!destination
            .parent()
            .unwrap()
            .join(".wisp-partial-result.txt")
            .exists());
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0].script, "check rsync download source");
        assert_eq!(commands[1].program, "rsync");
        assert_eq!(commands[1].args, vec!["--version".to_string()]);
        assert_eq!(commands[2].program, "rsync");
        assert!(commands[2].args.contains(&"--partial".to_string()));
        assert!(commands[2]
            .args
            .iter()
            .any(|arg| arg == "alice@a.example:/data/result.txt"));
        drop(commands);
        let _ = std::fs::remove_dir_all(root);
    }
}
