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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactMaterialization {
    Snapshot,
    Reference,
    External,
}

impl ArtifactMaterialization {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Reference => "reference",
            Self::External => "external",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "snapshot" => Ok(Self::Snapshot),
            "reference" => Ok(Self::Reference),
            "external" => Ok(Self::External),
            _ => anyhow::bail!("Unknown Artifact materialization '{value}'"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactCaptureTiming {
    AtCreation,
    Late,
    Unknown,
}

impl ArtifactCaptureTiming {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AtCreation => "at_creation",
            Self::Late => "late",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "at_creation" => Ok(Self::AtCreation),
            "late" => Ok(Self::Late),
            "unknown" => Ok(Self::Unknown),
            _ => anyhow::bail!("Unknown Artifact capture timing '{value}'"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactVersionDraft {
    pub version_id: Option<String>,
    pub artifact_id: String,
    pub project_id: String,
    pub root_frame_id: String,
    pub filename: String,
    pub content_type: String,
    pub storage_path: String,
    pub logical_key: Option<String>,
    pub size_bytes: Option<i64>,
    pub checksum: Option<String>,
    pub producing_run_id: Option<String>,
    pub env_snapshot_hash: Option<String>,
    pub materialization: ArtifactMaterialization,
    pub capture_timing: ArtifactCaptureTiming,
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
    pub materialization: ArtifactMaterialization,
    pub capture_timing: ArtifactCaptureTiming,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersionContext {
    pub version: ArtifactVersion,
    pub project_id: String,
    pub root_frame_id: String,
    pub filename: String,
    pub logical_key: Option<String>,
    pub latest_version_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineageBasis {
    Declared,
    Observed,
    Inferred,
    UserAsserted,
}

impl LineageBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Declared => "declared",
            Self::Observed => "observed",
            Self::Inferred => "inferred",
            Self::UserAsserted => "user_asserted",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "declared" => Ok(Self::Declared),
            "observed" => Ok(Self::Observed),
            "inferred" => Ok(Self::Inferred),
            "user_asserted" => Ok(Self::UserAsserted),
            _ => anyhow::bail!("Unknown lineage basis '{value}'"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineageConfidence {
    Exact,
    Likely,
    Uncertain,
}

impl LineageConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Likely => "likely",
            Self::Uncertain => "uncertain",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "exact" => Ok(Self::Exact),
            "likely" => Ok(Self::Likely),
            "uncertain" => Ok(Self::Uncertain),
            _ => anyhow::bail!("Unknown lineage confidence '{value}'"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInput {
    pub id: String,
    pub run_id: String,
    pub artifact_version_id: Option<String>,
    pub external_resource_id: Option<String>,
    pub source_ref: String,
    pub role: String,
    pub required: bool,
    pub basis: LineageBasis,
    pub confidence: LineageConfidence,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOutput {
    pub id: String,
    pub run_id: String,
    pub artifact_version_id: String,
    pub role: String,
    pub logical_output_key: String,
    pub source_path: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDependency {
    pub id: String,
    pub artifact_version_id: String,
    pub depends_on_version_id: String,
    pub reference_name: Option<String>,
    pub basis: LineageBasis,
    pub confidence: LineageConfidence,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCodeSnapshot {
    pub id: String,
    pub run_id: String,
    pub source_kind: String,
    pub source_path: Option<String>,
    pub source_text: String,
    pub checksum: String,
    pub storage_path: Option<String>,
    pub git_commit: Option<String>,
    pub dirty_patch: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentSnapshot {
    pub hash: String,
    pub env_name: Option<String>,
    pub packages_json: String,
    pub snapshot_json: String,
    pub hash_algorithm: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalResource {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub uri: String,
    pub version: Option<String>,
    pub checksum: Option<String>,
    pub size_bytes: Option<i64>,
    pub license: Option<String>,
    pub visibility: String,
    pub access_instructions: Option<String>,
    pub accessed_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationRevisionState {
    Draft,
    Freezing,
    Frozen,
    Published,
    Deleting,
}

impl PublicationRevisionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Freezing => "freezing",
            Self::Frozen => "frozen",
            Self::Published => "published",
            Self::Deleting => "deleting",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "draft" => Ok(Self::Draft),
            "freezing" => Ok(Self::Freezing),
            "frozen" => Ok(Self::Frozen),
            "published" => Ok(Self::Published),
            "deleting" => Ok(Self::Deleting),
            _ => anyhow::bail!("Unknown Publication revision state '{value}'"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationCapabilityLevel {
    Archived,
    Traceable,
    ReExecutable,
    Reproduced,
}

impl PublicationCapabilityLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Archived => "archived",
            Self::Traceable => "traceable",
            Self::ReExecutable => "re_executable",
            Self::Reproduced => "reproduced",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "archived" => Ok(Self::Archived),
            "traceable" => Ok(Self::Traceable),
            "re_executable" => Ok(Self::ReExecutable),
            "reproduced" => Ok(Self::Reproduced),
            _ => anyhow::bail!("Unknown Publication capability level '{value}'"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationItemKind {
    Section,
    Claim,
    Figure,
    Table,
    Methods,
    Supplement,
}

impl PublicationItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Section => "section",
            Self::Claim => "claim",
            Self::Figure => "figure",
            Self::Table => "table",
            Self::Methods => "methods",
            Self::Supplement => "supplement",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "section" => Ok(Self::Section),
            "claim" => Ok(Self::Claim),
            "figure" => Ok(Self::Figure),
            "table" => Ok(Self::Table),
            "methods" => Ok(Self::Methods),
            "supplement" => Ok(Self::Supplement),
            _ => anyhow::bail!("Unknown Publication item kind '{value}'"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSourceKind {
    ArtifactVersion,
    Run,
    ExecutionLog,
    MessageSpan,
    ToolCall,
    CodeCell,
    ExternalResource,
}

impl EvidenceSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ArtifactVersion => "artifact_version",
            Self::Run => "run",
            Self::ExecutionLog => "execution_log",
            Self::MessageSpan => "message_span",
            Self::ToolCall => "tool_call",
            Self::CodeCell => "code_cell",
            Self::ExternalResource => "external_resource",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "artifact_version" => Ok(Self::ArtifactVersion),
            "run" => Ok(Self::Run),
            "execution_log" => Ok(Self::ExecutionLog),
            "message_span" => Ok(Self::MessageSpan),
            "tool_call" => Ok(Self::ToolCall),
            "code_cell" => Ok(Self::CodeCell),
            "external_resource" => Ok(Self::ExternalResource),
            _ => anyhow::bail!("Unknown evidence source kind '{value}'"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSelectionState {
    Candidate,
    Selected,
    Rejected,
}

impl EvidenceSelectionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Selected => "selected",
            Self::Rejected => "rejected",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "candidate" => Ok(Self::Candidate),
            "selected" => Ok(Self::Selected),
            "rejected" => Ok(Self::Rejected),
            _ => anyhow::bail!("Unknown evidence selection state '{value}'"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceReviewState {
    Unreviewed,
    Reviewed,
}

impl EvidenceReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unreviewed => "unreviewed",
            Self::Reviewed => "reviewed",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "unreviewed" => Ok(Self::Unreviewed),
            "reviewed" => Ok(Self::Reviewed),
            _ => anyhow::bail!("Unknown evidence review state '{value}'"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceReproductionState {
    NotRun,
    Passed,
    Failed,
    NotApplicable,
}

impl EvidenceReproductionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRun => "not_run",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotApplicable => "not_applicable",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "not_run" => Ok(Self::NotRun),
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            "not_applicable" => Ok(Self::NotApplicable),
            _ => anyhow::bail!("Unknown evidence reproduction state '{value}'"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceVisibility {
    Public,
    Restricted,
    Private,
}

impl EvidenceVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Restricted => "restricted",
            Self::Private => "private",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "public" => Ok(Self::Public),
            "restricted" => Ok(Self::Restricted),
            "private" => Ok(Self::Private),
            _ => anyhow::bail!("Unknown evidence visibility '{value}'"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publication {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationRevision {
    pub id: String,
    pub publication_id: String,
    pub parent_revision_id: Option<String>,
    pub revision_number: i64,
    pub label: String,
    pub state: PublicationRevisionState,
    pub capability_level: PublicationCapabilityLevel,
    pub manifest_json: Option<String>,
    pub manifest_sha256: Option<String>,
    pub frozen_at: Option<i64>,
    pub published_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationItem {
    pub id: String,
    pub revision_id: String,
    pub parent_item_id: Option<String>,
    pub kind: PublicationItemKind,
    pub title: String,
    pub content: String,
    pub ordinal: i64,
    pub metadata_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationItemLink {
    pub id: String,
    pub revision_id: String,
    pub source_item_id: String,
    pub target_item_id: String,
    pub relation: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceBindingDraft {
    pub id: String,
    pub revision_id: String,
    pub item_id: Option<String>,
    pub source_kind: EvidenceSourceKind,
    pub source_id: String,
    pub purpose: String,
    pub supported_claim_item_id: Option<String>,
    pub selection_state: EvidenceSelectionState,
    pub visibility: EvidenceVisibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceBinding {
    pub id: String,
    pub revision_id: String,
    pub item_id: Option<String>,
    pub source_kind: EvidenceSourceKind,
    pub source_id: String,
    pub artifact_version_id: Option<String>,
    pub run_id: Option<String>,
    pub external_resource_id: Option<String>,
    pub purpose: String,
    pub supported_claim_item_id: Option<String>,
    pub selection_state: EvidenceSelectionState,
    pub review_state: EvidenceReviewState,
    pub reproduction_state: EvidenceReproductionState,
    pub visibility: EvidenceVisibility,
    pub source_snapshot_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceReview {
    pub id: String,
    pub binding_id: String,
    pub reviewer: String,
    pub method: String,
    pub verified_at: i64,
    pub environment_json: String,
    pub comparator_json: String,
    pub tolerance_json: String,
    pub result: String,
    pub report_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceSupersession {
    pub id: String,
    pub revision_id: String,
    pub old_binding_id: String,
    pub new_binding_id: String,
    pub reason: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationReadinessReport {
    pub id: String,
    pub revision_id: String,
    pub capability_level: PublicationCapabilityLevel,
    pub target_visibility: EvidenceVisibility,
    pub policy_json: String,
    pub blockers_json: String,
    pub warnings_json: String,
    pub omissions_json: String,
    pub manifest_json: String,
    pub manifest_sha256: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationFreezePolicy {
    pub target_visibility: EvidenceVisibility,
    pub phi_pii_reviewed: bool,
    pub redistribution_reviewed: bool,
    pub snapshot_restricted_bytes: bool,
}

impl Default for PublicationFreezePolicy {
    fn default() -> Self {
        Self {
            target_visibility: EvidenceVisibility::Public,
            phi_pii_reviewed: false,
            redistribution_reviewed: false,
            snapshot_restricted_bytes: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationReadinessFinding {
    pub code: String,
    pub message: String,
    pub binding_id: Option<String>,
    pub source_kind: Option<EvidenceSourceKind>,
    pub source_id: Option<String>,
    pub waivable: bool,
    pub waived: bool,
    pub waiver: Option<PublicationWaiver>,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationReadiness {
    pub revision_id: String,
    pub target_visibility: EvidenceVisibility,
    pub capability_level: PublicationCapabilityLevel,
    pub blockers: Vec<PublicationReadinessFinding>,
    pub warnings: Vec<PublicationReadinessFinding>,
    pub omissions: Vec<PublicationReadinessFinding>,
    pub manifest_json: String,
    pub manifest_sha256: String,
    pub can_freeze: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationLateCapture {
    pub binding_ids: Vec<String>,
    pub old_version_id: String,
    pub new_version_id: String,
    pub artifact_id: String,
    pub expected_latest_version_id: Option<String>,
    pub version_number: i64,
    pub content_type: String,
    pub storage_path: String,
    pub size_bytes: i64,
    pub checksum: String,
    pub materialization: ArtifactMaterialization,
    pub source_snapshot_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationFreezeCommit {
    pub revision_id: String,
    pub attempt_id: String,
    pub policy_json: String,
    pub readiness: PublicationReadiness,
    pub late_captures: Vec<PublicationLateCapture>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationEvidenceDrift {
    pub binding_id: String,
    pub artifact_id: String,
    pub logical_key: Option<String>,
    pub bound_version_id: String,
    pub bound_version_number: i64,
    pub latest_version_id: String,
    pub latest_version_number: i64,
    pub has_drift: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationWaiver {
    pub id: String,
    pub revision_id: String,
    pub finding_code: String,
    pub author: String,
    pub reason: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleBuild {
    pub id: String,
    pub revision_id: String,
    pub format: String,
    pub visibility: EvidenceVisibility,
    pub status: String,
    pub output_path: Option<String>,
    pub revision_manifest_sha256: String,
    pub archive_sha256: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReproductionComparatorKind {
    Sha256,
    Text,
    Json,
    Numeric,
}

impl ReproductionComparatorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
            Self::Text => "text",
            Self::Json => "json",
            Self::Numeric => "numeric",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self> {
        match value {
            "sha256" => Ok(Self::Sha256),
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "numeric" => Ok(Self::Numeric),
            _ => anyhow::bail!("Unknown reproduction comparator '{value}'"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReproductionRun {
    pub id: String,
    pub revision_id: String,
    pub source_run_id: String,
    pub status: String,
    pub capability_level: PublicationCapabilityLevel,
    pub command_sha256: String,
    pub expected_environment_hash: Option<String>,
    pub actual_environment_json: String,
    pub actual_environment_hash: String,
    pub environment_matched: bool,
    pub workspace_manifest_json: String,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub exit_code: Option<i64>,
    pub error: Option<String>,
    pub created_at: i64,
    pub started_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReproductionResult {
    pub id: String,
    pub reproduction_run_id: String,
    pub output_id: String,
    pub output_path: String,
    pub expected_artifact_version_id: String,
    pub comparator_kind: ReproductionComparatorKind,
    pub required: bool,
    pub expected_json: String,
    pub actual_json: String,
    pub tolerance_json: String,
    pub passed: bool,
    pub report_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReproductionRunStart {
    pub id: String,
    pub revision_id: String,
    pub source_run_id: String,
    pub command_sha256: String,
    pub expected_environment_hash: Option<String>,
    pub actual_environment_json: String,
    pub actual_environment_hash: String,
    pub environment_matched: bool,
    pub workspace_manifest_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReproductionRunCommit {
    pub run_id: String,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub exit_code: i64,
    pub results: Vec<ReproductionResult>,
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
    pub created_artifact: bool,
    pub created_version: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnFileUndo {
    pub frame_id: String,
    pub user_message_seq: i64,
    pub path: String,
    pub before_exists: bool,
    pub before_snapshot_path: Option<String>,
    pub before_checksum: Option<String>,
    pub after_checksum: Option<String>,
    pub reversible: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RecentSessionDetail {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub created_at: i64,
    pub activity_at: i64,
    pub last_role: Option<String>,
    /// Activity newer than the last time the user viewed the session.
    pub unseen: bool,
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
    /// User-facing path derived from a `path:` logical key. Storage paths may
    /// point into Wisp's private content-addressed snapshot tree instead.
    pub logical_path: Option<String>,
    /// Latest version's remote source was abandoned (server discarded or the
    /// persisted remote file was deleted after confirmation).
    pub source_discarded: bool,
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
    /// Activity newer than the last time the user viewed the session.
    pub unseen: bool,
    /// Shelved (archived) out of everyday navigation; only keyword searches
    /// that opt in via [`SessionSearchOptions::include_shelved`] return it.
    pub shelved: bool,
}

/// Widens the default discovery scope of a session search.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionSearchOptions {
    /// Also match shelved sessions when the query is non-empty. The empty
    /// "recent sessions" list never includes them, so a shelved conversation
    /// stays out of everyday navigation but can still be found by name.
    pub include_shelved: bool,
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

    pub(crate) fn from_storage(s: &str) -> Result<Self> {
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
    Paused,
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
            Self::Paused => "paused",
            Self::Cancelling => "cancelling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Lost => "lost",
        }
    }

    pub(crate) fn from_storage(s: &str) -> Result<Self> {
        match s {
            "draft" => Ok(Self::Draft),
            "submitted" => Ok(Self::Submitted),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
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
    /// When declared output specs were registered (downloaded/verified for
    /// remote Runs). NULL means outputs were never harvested.
    pub harvested_at: Option<i64>,
    /// When the server-side run workspace was deleted. NULL means it still
    /// exists (or was never created).
    pub cleaned_at: Option<i64>,
    pub cleanup_error: Option<String>,
    /// Project-relative directory holding the run's full stdout/stderr logs,
    /// pulled back before the server workspace was cleaned. NULL means logs
    /// were never saved locally.
    pub logs_path: Option<String>,
}

/// Polling/list projection for the WebView. Large command, output, remote
/// handle, and environment payloads stay behind `get_run_detail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: String,
    pub frame_id: Option<String>,
    pub context_id: String,
    pub title: String,
    pub kind: String,
    pub status: RunStatus,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i64>,
    pub remote_workdir: Option<String>,
    pub timeout_secs: Option<i64>,
    pub last_polled_at: Option<i64>,
    pub last_poll_error: Option<String>,
    pub progress_json: String,
    pub harvested_at: Option<i64>,
    pub cleaned_at: Option<i64>,
    pub cleanup_error: Option<String>,
    /// When the user moved this Run to the background. NULL means a turn may
    /// still wait on it and no completion notice is due.
    pub background_requested_at: Option<i64>,
    /// Bounded head/tail sample plus byte lengths; changes make visible run
    /// monitors refetch the one full record they display.
    pub output_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunProgress {
    /// The transport cannot report byte-level progress for this phase.
    #[serde(default)]
    pub indeterminate: bool,
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

/// Automatic harvest should skip collect/transfer once outputs are registered.
pub fn skip_auto_harvest(harvested_at: Option<i64>) -> bool {
    harvested_at.is_some()
}

/// `renew_run_lifecycle` / `finish_active_run_owned` return false when the
/// caller no longer holds a live lease. `message` is the hard-error text.
pub fn require_lifecycle_hold(held: bool, message: &str) -> Result<(), String> {
    if held {
        Ok(())
    } else {
        Err(message.to_string())
    }
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
            harvested_at: None,
            cleaned_at: None,
            cleanup_error: None,
            logs_path: None,
        }
    }

    pub fn is_harvested(&self) -> bool {
        skip_auto_harvest(self.harvested_at)
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
        harvested_at: row.try_get("harvested_at")?,
        cleaned_at: row.try_get("cleaned_at")?,
        cleanup_error: row.try_get("cleanup_error")?,
        logs_path: row.try_get("logs_path")?,
    })
}

pub(crate) fn run_summary_from_row(row: SqliteRow) -> Result<RunSummary> {
    let status: String = row.try_get("status")?;
    Ok(RunSummary {
        id: row.try_get("id")?,
        frame_id: row.try_get("frame_id")?,
        context_id: row.try_get("context_id")?,
        title: row.try_get("title")?,
        kind: row.try_get("kind")?,
        status: RunStatus::from_storage(&status)?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        ended_at: row.try_get("ended_at")?,
        exit_code: row.try_get("exit_code")?,
        remote_workdir: row.try_get("remote_workdir")?,
        timeout_secs: row.try_get("timeout_secs")?,
        last_polled_at: row.try_get("last_polled_at")?,
        last_poll_error: row.try_get("last_poll_error")?,
        progress_json: row.try_get("progress_json")?,
        harvested_at: row.try_get("harvested_at")?,
        cleaned_at: row.try_get("cleaned_at")?,
        cleanup_error: row.try_get("cleanup_error")?,
        background_requested_at: row.try_get("background_requested_at")?,
        output_fingerprint: row.try_get("output_fingerprint")?,
    })
}

pub(crate) fn artifact_version_from_row(row: SqliteRow) -> Result<ArtifactVersion> {
    let materialization: String = row.try_get("materialization")?;
    let capture_timing: String = row.try_get("capture_timing")?;
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
        materialization: ArtifactMaterialization::from_storage(&materialization)?,
        capture_timing: ArtifactCaptureTiming::from_storage(&capture_timing)?,
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
