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
struct SshTrustEdge {
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
    destination_path: String,
    #[serde(default = "default_auto")]
    route: String,
    #[serde(default = "default_auto")]
    transport: String,
    timeout_secs: Option<u64>,
}

fn default_auto() -> String {
    "auto".into()
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
            "Transfer one exact file or directory between two selected SSH contexts as a persisted Run. `auto` uses a verified direct edge when available (rsync with scp fallback), otherwise it relays through a private local temporary directory using each context's configured credentials. Never use shell ssh/scp/rsync for this.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "source_context_id": { "type": "string" },
                    "source_path": { "type": "string", "description": "Exact absolute or ~/ source path; globs are rejected" },
                    "destination_context_id": { "type": "string" },
                    "destination_path": { "type": "string", "description": "Exact absolute or ~/ destination path; globs are rejected" },
                    "route": { "type": "string", "enum": ["auto", "direct", "relay"], "default": "auto" },
                    "transport": { "type": "string", "enum": ["auto", "rsync", "scp"], "default": "auto" },
                    "timeout_secs": { "type": "integer", "description": "Wall timeout, 1 second to 7 days" }
                },
                "required": ["source_context_id", "source_path", "destination_context_id", "destination_path"]
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
            .unwrap_or_default();
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

async fn load_trust_edges(store: &wisp_store::Store) -> Vec<SshTrustEdge> {
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

async fn submit_transfer(
    store: &wisp_store::Store,
    manager: &RunManager,
    project_id: &str,
    frame_id: Option<&str>,
    project_root: &Path,
    request: TransferRequest,
) -> Result<serde_json::Value, String> {
    if request.source_context_id == request.destination_context_id {
        return Err("Source and destination SSH contexts must be different".into());
    }
    validate_remote_path("source_path", &request.source_path)?;
    validate_remote_path("destination_path", &request.destination_path)?;
    if !matches!(request.route.as_str(), "auto" | "direct" | "relay") {
        return Err("route must be 'auto', 'direct', or 'relay'".into());
    }
    if !matches!(request.transport.as_str(), "auto" | "rsync" | "scp") {
        return Err("transport must be 'auto', 'rsync', or 'scp'".into());
    }
    let source = selected_ssh_context(store, frame_id, &request.source_context_id).await?;
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
    let timeout_secs = request
        .timeout_secs
        .unwrap_or(4 * 60 * 60)
        .clamp(1, 7 * 24 * 60 * 60);

    let response = if route == "direct" {
        let edge = edge.expect("direct route requires edge");
        let command = direct_transfer_script(
            &request.source_path,
            &request.destination_path,
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
                &request.destination_path,
                Duration::from_secs(timeout_secs),
            )
            .await?
    };
    Ok(serde_json::json!({
        "run_id": response.run_id,
        "status": response.status,
        "route": route,
        "transport": if route == "relay" { "scp" } else { request.transport.as_str() },
        "next_action": "Call monitor_run exactly once to wait for completion."
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
        let mut download_args = source.scp_option_args()?;
        download_args.push("-r".into());
        download_args.push(format!("{}:{source_path}", source.target()?));
        download_args.push(scp_local_path(&relay_dir.0));
        let download = checked_output(
            "Relay download",
            run_with_lifecycle_lease(
                store,
                run_id,
                owner_id,
                runner.as_ref(),
                RunCommand {
                    context_id: format!("ssh:{}", source.alias),
                    program: "scp".into(),
                    args: download_args,
                    script: "relay download".into(),
                    cwd: Some(relay_dir.0.clone()),
                    stdin: None,
                    envs: crate::ssh_hosts::auth_envs_for_connection(&source)?,
                },
                timeout,
            )
            .await,
        )?;
        let local_item = single_relay_item(&relay_dir.0)?;
        let (total_bytes, files_total) = relay_item_stats(&local_item)?;
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
            if command.script == "relay download" {
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
                destination_path: "/results/".into(),
                route: "auto".into(),
                transport: "auto".into(),
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
}
