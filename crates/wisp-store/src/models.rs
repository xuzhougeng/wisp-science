use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInstallation {
    pub plugin_id: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub author: String,
    pub license: String,
    pub source_uri: String,
    pub install_root: String,
    pub archive_sha256: String,
    pub manifest_json: String,
    pub trust_state: String,
    pub installed_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPlugin {
    pub project_id: String,
    pub plugin_id: String,
    pub version: String,
    pub enabled: bool,
    pub grants_json: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ExecLog {
    pub id: String,
    pub frame_id: String,
    pub cell_index: i64,
    pub tool: String,
    pub language: String,
    pub source: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_status: String,
    pub wall_s: Option<f64>,
    pub files_written: Vec<String>,
    pub files_read: Vec<String>,
    pub env_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    pub id: String,
    pub artifact_id: String,
    pub version_number: i64,
    pub content_type: String,
    pub storage_path: String,
    pub size_bytes: Option<i64>,
    pub checksum: Option<String>,
    pub parent_version_id: Option<String>,
    pub producing_run_id: Option<String>,
    pub env_snapshot_hash: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageResourceLink {
    pub id: String,
    pub frame_id: String,
    pub message_seq: i64,
    pub ordinal: i64,
    pub original_reference: String,
    pub artifact_id: Option<String>,
    pub artifact_version_id: Option<String>,
    pub display_name: String,
    pub resource_kind: String,
    pub mime_type: String,
    pub status: String,
    pub error: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct RecentSessionDetail {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub created_at: i64,
    pub activity_at: i64,
    pub last_role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactSearchResult {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub path: String,
    pub ts: i64,
    pub project_id: String,
    pub project_name: String,
    pub project_root: String,
    pub session_id: String,
    pub session_title: String,
    pub size_bytes: Option<i64>,
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSearchResult {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub title: String,
    pub created_at: i64,
    pub activity_at: i64,
    pub last_role: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionContextKind {
    Local,
    Ssh,
    Wsl,
}

impl ExecutionContextKind {
    pub fn from_id(id: &str) -> Result<Self> {
        if id != id.trim() || id.is_empty() {
            anyhow::bail!("Invalid execution context id");
        }
        if id == "local" {
            return Ok(Self::Local);
        }
        if let Some(alias) = id.strip_prefix("ssh:") {
            validate_context_suffix(alias)?;
            return Ok(Self::Ssh);
        }
        if let Some(distro) = id.strip_prefix("wsl:") {
            validate_context_suffix(distro)?;
            return Ok(Self::Wsl);
        }
        anyhow::bail!("Unknown execution context id prefix");
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Ssh => "ssh",
            Self::Wsl => "wsl",
        }
    }

    fn from_storage(s: &str) -> Result<Self> {
        match s {
            "local" => Ok(Self::Local),
            "ssh" => Ok(Self::Ssh),
            "wsl" => Ok(Self::Wsl),
            _ => anyhow::bail!("Unknown execution context kind"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub id: String,
    pub kind: ExecutionContextKind,
    pub label: String,
    pub config_json: String,
    pub capabilities_json: String,
    pub last_probe_at: Option<i64>,
    pub last_probe_status: Option<String>,
    pub last_probe_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Draft,
    Submitted,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Lost,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Submitted => "submitted",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Lost => "lost",
        }
    }

    fn from_storage(s: &str) -> Result<Self> {
        match s {
            "draft" => Ok(Self::Draft),
            "submitted" => Ok(Self::Submitted),
            "running" => Ok(Self::Running),
            "cancelling" => Ok(Self::Cancelling),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "timed_out" => Ok(Self::TimedOut),
            "lost" => Ok(Self::Lost),
            _ => anyhow::bail!("Unknown run status"),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut | Self::Lost
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub project_id: String,
    pub frame_id: Option<String>,
    pub context_id: String,
    pub title: String,
    pub kind: String,
    pub status: RunStatus,
    pub command: Option<String>,
    pub script_path: Option<String>,
    pub input_refs_json: String,
    pub output_specs_json: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i64>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub remote_workdir: Option<String>,
    pub remote_handle_json: Option<String>,
    pub timeout_secs: Option<i64>,
    pub last_polled_at: Option<i64>,
    pub last_poll_error: Option<String>,
    pub progress_json: String,
    pub env_snapshot_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunProgress {
    pub phase: String,
    pub direction: String,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub files_completed: u64,
    pub files_total: u64,
    pub current_file: Option<String>,
    pub bytes_per_second: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchNodeKind {
    Decision,
    Paper,
    DataAsset,
    Run,
    Artifact,
}

impl ResearchNodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Paper => "paper",
            Self::DataAsset => "data_asset",
            Self::Run => "run",
            Self::Artifact => "artifact",
        }
    }

    fn from_storage(s: &str) -> Result<Self> {
        match s {
            "decision" => Ok(Self::Decision),
            "paper" => Ok(Self::Paper),
            "data_asset" => Ok(Self::DataAsset),
            "run" => Ok(Self::Run),
            "artifact" => Ok(Self::Artifact),
            _ => anyhow::bail!("Unknown research node kind"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchNode {
    pub id: String,
    pub project_id: String,
    pub kind: ResearchNodeKind,
    pub title: String,
    pub ref_id: Option<String>,
    pub metadata_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ResearchNode {
    pub fn new(
        id: impl Into<String>,
        project_id: impl Into<String>,
        kind: ResearchNodeKind,
        title: impl Into<String>,
    ) -> Result<Self> {
        let now = chrono::Utc::now().timestamp();
        let node = Self {
            id: id.into(),
            project_id: project_id.into(),
            kind,
            title: title.into(),
            ref_id: None,
            metadata_json: "{}".into(),
            created_at: now,
            updated_at: now,
        };
        node.validate()?;
        Ok(node)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            anyhow::bail!("Research node id is required");
        }
        if self.project_id.trim().is_empty() {
            anyhow::bail!("Research node project_id is required");
        }
        if self.title.trim().is_empty() {
            anyhow::bail!("Research node title is required");
        }
        if serde_json::from_str::<serde_json::Value>(&self.metadata_json).is_err() {
            anyhow::bail!("Research node metadata_json must be valid JSON");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchEdge {
    pub id: String,
    pub project_id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub metadata_json: String,
    pub created_at: i64,
}

impl ResearchEdge {
    pub fn new(
        id: impl Into<String>,
        project_id: impl Into<String>,
        source_id: impl Into<String>,
        target_id: impl Into<String>,
        relation: impl Into<String>,
    ) -> Result<Self> {
        let edge = Self {
            id: id.into(),
            project_id: project_id.into(),
            source_id: source_id.into(),
            target_id: target_id.into(),
            relation: relation.into(),
            metadata_json: "{}".into(),
            created_at: chrono::Utc::now().timestamp(),
        };
        edge.validate()?;
        Ok(edge)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            anyhow::bail!("Research edge id is required");
        }
        if self.project_id.trim().is_empty() {
            anyhow::bail!("Research edge project_id is required");
        }
        if self.source_id.trim().is_empty() || self.target_id.trim().is_empty() {
            anyhow::bail!("Research edge endpoints are required");
        }
        if self.relation.trim().is_empty() {
            anyhow::bail!("Research edge relation is required");
        }
        if serde_json::from_str::<serde_json::Value>(&self.metadata_json).is_err() {
            anyhow::bail!("Research edge metadata_json must be valid JSON");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchGraph {
    pub nodes: Vec<ResearchNode>,
    pub edges: Vec<ResearchEdge>,
}

impl RunRecord {
    pub fn new(
        id: impl Into<String>,
        project_id: impl Into<String>,
        context_id: impl Into<String>,
        title: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id: id.into(),
            project_id: project_id.into(),
            frame_id: None,
            context_id: context_id.into(),
            title: title.into(),
            kind: kind.into(),
            status: RunStatus::Draft,
            command: None,
            script_path: None,
            input_refs_json: "[]".into(),
            output_specs_json: "[]".into(),
            created_at: now,
            started_at: None,
            ended_at: None,
            exit_code: None,
            stdout_tail: None,
            stderr_tail: None,
            remote_workdir: None,
            remote_handle_json: None,
            timeout_secs: None,
            last_polled_at: None,
            last_poll_error: None,
            progress_json: "{}".into(),
            env_snapshot_json: "{}".into(),
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            anyhow::bail!("Run id is required");
        }
        if self.project_id.trim().is_empty() {
            anyhow::bail!("Run project_id is required");
        }
        ExecutionContextKind::from_id(&self.context_id)?;
        if self.title.trim().is_empty() {
            anyhow::bail!("Run title is required");
        }
        if self.kind.trim().is_empty() {
            anyhow::bail!("Run kind is required");
        }
        Ok(())
    }
}

impl ExecutionContext {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Result<Self> {
        let id = id.into();
        let kind = ExecutionContextKind::from_id(&id)?;
        let label = label.into();
        if label.trim().is_empty() {
            anyhow::bail!("Execution context label is required");
        }
        let now = chrono::Utc::now().timestamp();
        Ok(Self {
            id,
            kind,
            label,
            config_json: "{}".into(),
            capabilities_json: "{}".into(),
            last_probe_at: None,
            last_probe_status: None,
            last_probe_error: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let kind = ExecutionContextKind::from_id(&self.id)?;
        if kind != self.kind {
            anyhow::bail!("Execution context kind does not match id");
        }
        if self.label.trim().is_empty() {
            anyhow::bail!("Execution context label is required");
        }
        Ok(())
    }
}

fn validate_context_suffix(s: &str) -> Result<()> {
    if s.is_empty() || s != s.trim() || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        anyhow::bail!("Invalid execution context id suffix");
    }
    Ok(())
}

pub(crate) fn execution_context_from_row(row: SqliteRow) -> Result<ExecutionContext> {
    let kind: String = row.try_get("kind")?;
    Ok(ExecutionContext {
        id: row.try_get("id")?,
        kind: ExecutionContextKind::from_storage(&kind)?,
        label: row.try_get("label")?,
        config_json: row.try_get("config_json")?,
        capabilities_json: row.try_get("capabilities_json")?,
        last_probe_at: row.try_get("last_probe_at")?,
        last_probe_status: row.try_get("last_probe_status")?,
        last_probe_error: row.try_get("last_probe_error")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub(crate) fn run_from_row(row: SqliteRow) -> Result<RunRecord> {
    let status: String = row.try_get("status")?;
    Ok(RunRecord {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        frame_id: row.try_get("frame_id")?,
        context_id: row.try_get("context_id")?,
        title: row.try_get("title")?,
        kind: row.try_get("kind")?,
        status: RunStatus::from_storage(&status)?,
        command: row.try_get("command")?,
        script_path: row.try_get("script_path")?,
        input_refs_json: row.try_get("input_refs_json")?,
        output_specs_json: row.try_get("output_specs_json")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        ended_at: row.try_get("ended_at")?,
        exit_code: row.try_get("exit_code")?,
        stdout_tail: row.try_get("stdout_tail")?,
        stderr_tail: row.try_get("stderr_tail")?,
        remote_workdir: row.try_get("remote_workdir")?,
        remote_handle_json: row.try_get("remote_handle_json")?,
        timeout_secs: row.try_get("timeout_secs")?,
        last_polled_at: row.try_get("last_polled_at")?,
        last_poll_error: row.try_get("last_poll_error")?,
        progress_json: row.try_get("progress_json")?,
        env_snapshot_json: row.try_get("env_snapshot_json")?,
    })
}

pub(crate) fn artifact_version_from_row(row: SqliteRow) -> Result<ArtifactVersion> {
    Ok(ArtifactVersion {
        id: row.try_get("id")?,
        artifact_id: row.try_get("artifact_id")?,
        version_number: row.try_get("version_number")?,
        content_type: row.try_get("content_type")?,
        storage_path: row.try_get("storage_path")?,
        size_bytes: row.try_get("size_bytes")?,
        checksum: row.try_get("checksum")?,
        parent_version_id: row.try_get("parent_version_id")?,
        producing_run_id: row.try_get("producing_run_id")?,
        env_snapshot_hash: row.try_get("env_snapshot_hash")?,
        created_at: row.try_get("created_at")?,
    })
}

pub(crate) fn run_node_id(run_id: &str) -> String {
    format!("run:{run_id}")
}

pub(crate) fn artifact_node_id(artifact_id: &str) -> String {
    format!("artifact:{artifact_id}")
}

pub(crate) fn research_node_from_row(row: SqliteRow) -> Result<ResearchNode> {
    let kind: String = row.try_get("kind")?;
    Ok(ResearchNode {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        kind: ResearchNodeKind::from_storage(&kind)?,
        title: row.try_get("title")?,
        ref_id: row.try_get("ref_id")?,
        metadata_json: row.try_get("metadata_json")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub(crate) fn research_edge_from_row(row: SqliteRow) -> Result<ResearchEdge> {
    Ok(ResearchEdge {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        source_id: row.try_get("source_id")?,
        target_id: row.try_get("target_id")?,
        relation: row.try_get("relation")?,
        metadata_json: row.try_get("metadata_json")?,
        created_at: row.try_get("created_at")?,
    })
}

pub(crate) fn validate_run_transition(from: RunStatus, to: RunStatus) -> Result<()> {
    if from == to {
        return Ok(());
    }
    let ok = match from {
        RunStatus::Draft => matches!(
            to,
            RunStatus::Submitted | RunStatus::Running | RunStatus::Cancelled
        ),
        RunStatus::Submitted => matches!(
            to,
            RunStatus::Running
                | RunStatus::Cancelling
                | RunStatus::Succeeded
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::TimedOut
                | RunStatus::Lost
        ),
        RunStatus::Running => matches!(
            to,
            RunStatus::Cancelling
                | RunStatus::Succeeded
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::TimedOut
                | RunStatus::Lost
        ),
        RunStatus::Cancelling => matches!(
            to,
            RunStatus::Succeeded
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::TimedOut
                | RunStatus::Lost
        ),
        RunStatus::Succeeded
        | RunStatus::Failed
        | RunStatus::Cancelled
        | RunStatus::TimedOut
        | RunStatus::Lost => false,
    };
    if ok {
        Ok(())
    } else {
        anyhow::bail!(
            "Invalid run status transition: {} -> {}",
            from.as_str(),
            to.as_str()
        )
    }
}

pub(crate) fn session_display_title(
    custom_title: Option<String>,
    first_user: Option<String>,
) -> String {
    if let Some(t) = custom_title {
        let t = t.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    first_user
        .and_then(|c| serde_json::from_str::<wisp_llm::Content>(&c).ok())
        .map(|c| c.as_text().chars().take(80).collect::<String>())
        .unwrap_or_default()
}

pub(crate) fn parse_role(s: &str) -> wisp_llm::Role {
    match s {
        "system" => wisp_llm::Role::System,
        "user" | "internal" => wisp_llm::Role::User,
        "assistant" => wisp_llm::Role::Assistant,
        "tool" => wisp_llm::Role::Tool,
        _ => wisp_llm::Role::User,
    }
}
