//! Paper evidence: the papers a project is writing, their versions, and
//! for each figure, table or claim the results and runs it rests on.
//!
//! This is the store's publication model seen from the desktop: papers,
//! versions, items nested as in the WebView's outline, and the evidence
//! bound to them (results, runs, passages of the project's conversations
//! and the exact anchors of the WebView's advanced dialog) with its
//! selection, who may see it, where it came from and whether a newer result
//! exists; the readiness check, waivers,
//! finalizing a version (see `paper_freeze`), starting a new version from
//! one, rerunning a finalized version's runs in a clean folder and saving
//! its evidence capsule. The rerun and the capsule are the WebView host's,
//! shared through `wisp_runs::publication_reproduction` and
//! `wisp_runs::publication_capsule`.

use anyhow::{bail, ensure, Context, Result};
use std::path::Path;
use wisp_runs::publication_capsule::{CapsuleArchive, CapsuleHost};
use wisp_store::{
    canonical_json_sha256, ArtifactCaptureTiming, EvidenceBinding, EvidenceBindingDraft,
    EvidenceSelectionState, EvidenceSourceKind, EvidenceVisibility, LineageConfidence,
    PublicationCapabilityLevel, PublicationEvidenceDrift, PublicationFreezePolicy, PublicationItem,
    PublicationItemKind, PublicationReadinessFinding, PublicationRevisionState, PublicationWaiver,
    Store,
};

const TITLE_LIMIT: usize = 200;
const NOTE_LIMIT: usize = 4_000;
/// What a paper's first version is called when the form leaves it empty.
const FIRST_VERSION: &str = "Initial submission";
/// Who a waiver is recorded as written by; this desktop has one person.
const WAIVER_AUTHOR: &str = "Local user";

/// A paper in the project's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperRow {
    pub id: String,
    pub title: String,
    pub description: String,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperVersionState {
    /// Still being put together; the only state that can be changed.
    Draft,
    /// Locked with its evidence.
    Finalized,
    Published,
    /// Being finalized or deleted by another desktop.
    Busy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperVersion {
    pub id: String,
    pub number: i64,
    pub label: String,
    pub state: PaperVersionState,
    /// The SHA-256 of what a finalized version locked; empty for a draft.
    pub fingerprint: String,
}

/// What an item of a paper is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperItemKind {
    Figure,
    Table,
    Claim,
    Section,
    Methods,
    Supplement,
}

impl PaperItemKind {
    /// The kinds the form offers, in the WebView item editor's order.
    pub const OFFERED: [PaperItemKind; 6] = [
        PaperItemKind::Section,
        PaperItemKind::Claim,
        PaperItemKind::Figure,
        PaperItemKind::Table,
        PaperItemKind::Methods,
        PaperItemKind::Supplement,
    ];

    fn stored(self) -> PublicationItemKind {
        match self {
            PaperItemKind::Figure => PublicationItemKind::Figure,
            PaperItemKind::Table => PublicationItemKind::Table,
            PaperItemKind::Claim => PublicationItemKind::Claim,
            PaperItemKind::Section => PublicationItemKind::Section,
            PaperItemKind::Methods => PublicationItemKind::Methods,
            PaperItemKind::Supplement => PublicationItemKind::Supplement,
        }
    }

    fn of(kind: PublicationItemKind) -> Self {
        match kind {
            PublicationItemKind::Figure => PaperItemKind::Figure,
            PublicationItemKind::Table => PaperItemKind::Table,
            PublicationItemKind::Claim => PaperItemKind::Claim,
            PublicationItemKind::Section => PaperItemKind::Section,
            PublicationItemKind::Methods => PaperItemKind::Methods,
            PublicationItemKind::Supplement => PaperItemKind::Supplement,
        }
    }
}

/// Where a piece of evidence comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperSourceKind {
    /// A registered result of the project, at one exact version.
    Result,
    Run,
    /// A passage of a message of one of the project's conversations, its
    /// text kept as it was when chosen.
    Passage,
    /// A tool call of a message and its result, its arguments and result
    /// kept as they were when chosen.
    ToolCall,
    /// One execution (a cell run and its output), kept by its exact id.
    ExecutionLog,
    /// The code of one execution, kept by its exact id.
    CodeCell,
    /// A data resource registered with the project outside it.
    Resource,
}

impl PaperSourceKind {
    /// How an exact anchor of the advanced dialog names its kind, as the
    /// WebView sends it (`wisp_dto::publication_sources::precise_evidence_source`).
    pub fn of_anchor(kind: &str) -> Option<Self> {
        match kind {
            "message_span" => Some(PaperSourceKind::Passage),
            "tool_call" => Some(PaperSourceKind::ToolCall),
            "execution_log" => Some(PaperSourceKind::ExecutionLog),
            "code_cell" => Some(PaperSourceKind::CodeCell),
            "external_resource" => Some(PaperSourceKind::Resource),
            _ => None,
        }
    }

    fn of(kind: EvidenceSourceKind) -> Self {
        match kind {
            EvidenceSourceKind::ArtifactVersion => PaperSourceKind::Result,
            EvidenceSourceKind::Run => PaperSourceKind::Run,
            EvidenceSourceKind::MessageSpan => PaperSourceKind::Passage,
            EvidenceSourceKind::ToolCall => PaperSourceKind::ToolCall,
            EvidenceSourceKind::ExecutionLog => PaperSourceKind::ExecutionLog,
            EvidenceSourceKind::CodeCell => PaperSourceKind::CodeCell,
            EvidenceSourceKind::ExternalResource => PaperSourceKind::Resource,
        }
    }
}

/// Whether a piece of evidence is meant for the paper. Only selected
/// evidence is locked when the version is finalized.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaperSelection {
    Candidate,
    #[default]
    Selected,
    Rejected,
}

impl PaperSelection {
    pub const ALL: [PaperSelection; 3] = [
        PaperSelection::Candidate,
        PaperSelection::Selected,
        PaperSelection::Rejected,
    ];

    fn of(state: EvidenceSelectionState) -> Self {
        match state {
            EvidenceSelectionState::Candidate => PaperSelection::Candidate,
            EvidenceSelectionState::Selected => PaperSelection::Selected,
            EvidenceSelectionState::Rejected => PaperSelection::Rejected,
        }
    }

    fn stored(self) -> EvidenceSelectionState {
        match self {
            PaperSelection::Candidate => EvidenceSelectionState::Candidate,
            PaperSelection::Selected => EvidenceSelectionState::Selected,
            PaperSelection::Rejected => EvidenceSelectionState::Rejected,
        }
    }
}

/// How sure the record is of where a piece of evidence came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperLineageQuality {
    Exact,
    Likely,
    Uncertain,
}

/// When a result's bytes were kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperCapture {
    AtCreation,
    /// Kept after the fact; the bytes may not be what the run wrote.
    Late,
    Unknown,
}

/// Where a piece of evidence came from, as the WebView's card and its
/// "Source and environment details" tell it
/// (`wisp_store::publication_lineage::lineage_summary`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperLineage {
    /// The files the run read, as recorded.
    pub inputs: Vec<String>,
    /// The run that made it, by name and by id.
    pub run_title: Option<String>,
    pub run_id: Option<String>,
    /// The code the run kept.
    pub code: Vec<String>,
    pub quality: PaperLineageQuality,
    /// For a result: its version, checksum and when it was kept.
    pub version: Option<i64>,
    pub checksum: Option<String>,
    pub capture: Option<PaperCapture>,
    pub input_count: usize,
    pub output_count: usize,
    pub code_count: usize,
    pub environment: bool,
    /// How the inputs were known (`declared`, `observed`, `inferred`,
    /// `user_asserted`), as stored.
    pub bases: Vec<String>,
}

impl PaperLineage {
    fn of(summary: &wisp_store::publication_lineage::PublicationLineageSummary) -> Self {
        PaperLineage {
            inputs: summary.input_labels.clone(),
            run_title: summary.producing_run_title.clone(),
            run_id: summary.producing_run_id.clone(),
            code: summary.code_labels.clone(),
            quality: match summary.quality {
                LineageConfidence::Exact => PaperLineageQuality::Exact,
                LineageConfidence::Likely => PaperLineageQuality::Likely,
                LineageConfidence::Uncertain => PaperLineageQuality::Uncertain,
            },
            version: summary.version_number,
            checksum: summary.checksum.clone(),
            capture: summary.capture_timing.map(|timing| match timing {
                ArtifactCaptureTiming::AtCreation => PaperCapture::AtCreation,
                ArtifactCaptureTiming::Late => PaperCapture::Late,
                ArtifactCaptureTiming::Unknown => PaperCapture::Unknown,
            }),
            input_count: summary.run_input_count,
            output_count: summary.run_output_count,
            code_count: summary.code_snapshot_count,
            environment: summary.environment_captured,
            bases: summary
                .bases
                .iter()
                .map(|basis| basis.as_str().to_owned())
                .collect(),
        }
    }
}

/// A result bound at one version when the project has a newer one; the
/// binding keeps the version it was bound at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaperDrift {
    pub bound: i64,
    pub latest: i64,
}

/// One result, run or passage a figure, table or claim rests on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperSource {
    /// The binding, to remove it.
    pub id: String,
    pub kind: PaperSourceKind,
    /// The file or run as it is called, when it can still be told; for a
    /// passage its first 80 characters.
    pub name: String,
    /// Why it was chosen.
    pub purpose: String,
    /// The run a clean rerun of a finalized version repeats for this
    /// evidence: the run itself, or the one that made the result.
    pub run: Option<String>,
    pub selection: PaperSelection,
    /// Who it may be shown to.
    pub audience: PaperAudience,
    pub lineage: PaperLineage,
    /// For a result the project has a newer version of.
    pub drift: Option<PaperDrift>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperItem {
    pub id: String,
    pub kind: PaperItemKind,
    pub title: String,
    pub note: String,
    /// The item it sits under; `None` at the top level.
    pub parent: Option<String>,
    /// How deep it sits in the outline; 0 at the top level.
    pub depth: usize,
    pub sources: Vec<PaperSource>,
}

/// One paper as its page shows it, at one of its versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paper {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Newest first.
    pub versions: Vec<PaperVersion>,
    /// The version whose items are shown; empty for a paper without one.
    pub shown: String,
    /// Whether the shown version can be changed.
    pub editable: bool,
    pub items: Vec<PaperItem>,
    /// Evidence bound to the version as a whole.
    pub loose: Vec<PaperSource>,
    /// For a finalized version: how far its evidence goes, counting clean
    /// reruns; `None` for a draft.
    pub level: Option<PaperLevel>,
    /// The clean reruns of the shown version, as recorded.
    pub reruns: Vec<PaperRerun>,
    /// The capsules built from the shown version, as recorded.
    pub capsules: Vec<PaperCapsule>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperRerunStatus {
    Running,
    Completed,
    Failed,
}

/// One output of a rerun compared with the finalized one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperRerunOutput {
    pub path: String,
    /// `sha256`, `text`, `json` or `numeric`.
    pub comparator: String,
    pub passed: bool,
}

/// A clean rerun of one run of a finalized version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperRerun {
    pub id: String,
    /// The run that was repeated.
    pub run: String,
    pub status: PaperRerunStatus,
    pub level: PaperLevel,
    pub environment_matched: bool,
    pub exit_code: Option<i64>,
    pub error: Option<String>,
    pub outputs: Vec<PaperRerunOutput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperCapsuleStatus {
    Building,
    Succeeded,
    Failed,
}

/// One attempt at saving a finalized version's evidence capsule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperCapsule {
    pub id: String,
    pub status: PaperCapsuleStatus,
    pub audience: PaperAudience,
    /// Where it was written.
    pub path: String,
    /// The SHA-256 of the archive, once written.
    pub archive: Option<String>,
    pub error: Option<String>,
}

/// The form for a new paper.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaperInput {
    pub title: String,
    pub description: String,
    /// What its first version is called; empty takes the usual name.
    pub version: String,
}

/// The form for a figure, table or claim; `id` empty adds one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperItemInput {
    pub id: String,
    pub kind: PaperItemKind,
    pub title: String,
    pub note: String,
    /// The item it sits under; `None` at the top level.
    pub parent: Option<String>,
}

/// A result, run or conversation message offered as evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperSourceChoice {
    pub kind: PaperSourceKind,
    pub id: String,
    /// The file, the run, or the conversation the message is in.
    pub name: String,
    /// The result's version, the run's state, or who wrote the message
    /// (`user` or `assistant`).
    pub detail: String,
    /// For a message: the store's choice with its text (up to 16 KiB) and
    /// what pins it, to pick a passage from and bind exactly.
    pub message: Option<wisp_dto::PublicationSourceChoice>,
}

/// Who the finalized evidence may be shown to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaperAudience {
    #[default]
    Private,
    Restricted,
    Public,
}

impl PaperAudience {
    pub const ALL: [PaperAudience; 3] = [
        PaperAudience::Private,
        PaperAudience::Restricted,
        PaperAudience::Public,
    ];

    fn of(visibility: EvidenceVisibility) -> Self {
        match visibility {
            EvidenceVisibility::Private => PaperAudience::Private,
            EvidenceVisibility::Restricted => PaperAudience::Restricted,
            EvidenceVisibility::Public => PaperAudience::Public,
        }
    }

    fn stored(self) -> EvidenceVisibility {
        match self {
            PaperAudience::Private => EvidenceVisibility::Private,
            PaperAudience::Restricted => EvidenceVisibility::Restricted,
            PaperAudience::Public => EvidenceVisibility::Public,
        }
    }
}

/// What finalizing a version is checked against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaperPolicy {
    pub audience: PaperAudience,
    /// For the public: the evidence was looked over for health and
    /// personal details.
    pub personal_reviewed: bool,
    /// For the public: licences and the right to pass it on were looked over.
    pub rights_reviewed: bool,
    /// Not for the public: copy restricted files into the record instead
    /// of only pointing at them.
    pub keep_restricted: bool,
}

impl PaperPolicy {
    fn stored(self) -> PublicationFreezePolicy {
        PublicationFreezePolicy {
            target_visibility: self.audience.stored(),
            phi_pii_reviewed: self.personal_reviewed,
            redistribution_reviewed: self.rights_reviewed,
            snapshot_restricted_bytes: self.keep_restricted,
        }
    }
}

/// How far the locked evidence goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperLevel {
    /// Kept, without a way back to what made it.
    Archived,
    /// Each result can be traced to what made it.
    Traceable,
    /// The runs behind it have their code and environment, to run again.
    Rerunnable,
    Reproduced,
}

impl PaperLevel {
    fn of(level: PublicationCapabilityLevel) -> Self {
        match level {
            PublicationCapabilityLevel::Archived => PaperLevel::Archived,
            PublicationCapabilityLevel::Traceable => PaperLevel::Traceable,
            PublicationCapabilityLevel::ReExecutable => PaperLevel::Rerunnable,
            PublicationCapabilityLevel::Reproduced => PaperLevel::Reproduced,
        }
    }
}

/// One thing the readiness check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperFinding {
    /// The store's code for it, which a waiver names.
    pub code: String,
    pub message: String,
    pub waivable: bool,
    /// Why it was waived, when it was.
    pub waived: Option<String>,
}

/// What checking or finalizing a version found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperCheck {
    /// Whether the version is now finalized.
    pub finalized: bool,
    pub level: PaperLevel,
    /// Each stops finalizing until it is fixed or waived.
    pub blockers: Vec<PaperFinding>,
    pub warnings: Vec<PaperFinding>,
    /// What the record leaves out.
    pub omissions: Vec<PaperFinding>,
    pub fingerprint: String,
    pub can_finalize: bool,
}

fn finding(found: PublicationReadinessFinding) -> PaperFinding {
    PaperFinding {
        code: found.code,
        message: found.message,
        waivable: found.waivable,
        waived: found
            .waived
            .then(|| found.waiver.map(|waiver| waiver.reason).unwrap_or_default()),
    }
}

fn title(text: &str) -> Result<String> {
    let text = text.trim();
    ensure!(!text.is_empty(), "A title is needed");
    ensure!(
        text.chars().count() <= TITLE_LIMIT,
        "A title is at most {TITLE_LIMIT} characters"
    );
    Ok(text.to_owned())
}

fn note(text: &str) -> Result<String> {
    let text = text.trim();
    ensure!(
        text.chars().count() <= NOTE_LIMIT,
        "A description is at most {NOTE_LIMIT} characters"
    );
    Ok(text.to_owned())
}

pub(crate) async fn list(store: &Store, project: &str) -> Result<Vec<PaperRow>> {
    let mut papers: Vec<_> = store
        .list_publications(project)
        .await?
        .into_iter()
        .map(|paper| PaperRow {
            id: paper.id,
            title: paper.title,
            description: paper.description,
            updated_at: paper.updated_at,
        })
        .collect();
    papers.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
    Ok(papers)
}

/// The paper when it is this project's.
async fn owned(store: &Store, project: &str, paper: &str) -> Result<wisp_store::Publication> {
    store
        .get_publication(paper)
        .await?
        .filter(|found| found.project_id == project)
        .context("That paper is not in this project")
}

/// The version when it is a draft of a paper of this project.
async fn draft(store: &Store, project: &str, version: &str) -> Result<()> {
    let found = store
        .get_publication_revision(version)
        .await?
        .context("That version is gone")?;
    owned(store, project, &found.publication_id).await?;
    ensure!(
        found.state == PublicationRevisionState::Draft,
        "A finalized version cannot be changed"
    );
    Ok(())
}

/// The text a passage was bound with, from its binding's snapshot when the
/// snapshot is intact: the first 80 characters, as the WebView card shows.
fn passage_text(snapshot_json: &str) -> Option<String> {
    let snapshot: serde_json::Value = serde_json::from_str(snapshot_json).ok()?;
    let anchor = snapshot.get("anchor")?;
    let intact = snapshot
        .get("anchor_sha256")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|hash| hash == canonical_json_sha256(anchor).1);
    intact.then_some(())?;
    anchor
        .get("text_snapshot")
        .and_then(serde_json::Value::as_str)
        .map(|text| text.chars().take(80).collect())
}

/// A piece of evidence as its card shows it: named as the WebView's card
/// names it, traced to the run that made it, and with a newer version of
/// its result noted when `drift` has one.
async fn source(
    store: &Store,
    binding: EvidenceBinding,
    drift: &[PublicationEvidenceDrift],
) -> Result<PaperSource> {
    let summary = wisp_store::publication_lineage::lineage_summary(store, &binding).await?;
    let kind = PaperSourceKind::of(binding.source_kind);
    // A passage shows its kept text only while the kept snapshot is intact.
    let name = match kind {
        PaperSourceKind::Passage => passage_text(&binding.source_snapshot_json),
        _ => Some(summary.source_label.clone()),
    };
    // As the WebView's card: the run that made a result, or the binding's
    // own run, or the run itself.
    let run = summary
        .producing_run_id
        .clone()
        .or_else(|| (kind == PaperSourceKind::Run).then(|| binding.source_id.clone()));
    Ok(PaperSource {
        kind,
        name: name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| binding.source_kind.as_str().replace('_', " ")),
        purpose: binding.purpose,
        run,
        selection: PaperSelection::of(binding.selection_state),
        audience: PaperAudience::of(binding.visibility),
        lineage: PaperLineage::of(&summary),
        drift: drift
            .iter()
            .find(|found| found.binding_id == binding.id && found.has_drift)
            .map(|found| PaperDrift {
                bound: found.bound_version_number,
                latest: found.latest_version_number,
            }),
        id: binding.id,
    })
}

/// The paper at `version`, or at its newest version.
pub(crate) async fn open(
    store: &Store,
    project: &str,
    paper: &str,
    version: Option<&str>,
) -> Result<Paper> {
    let found = owned(store, project, paper).await?;
    let mut revisions = store.list_publication_revisions(paper).await?;
    revisions.sort_by(|a, b| b.revision_number.cmp(&a.revision_number));
    let shown = match version {
        Some(id) => revisions
            .iter()
            .find(|revision| revision.id == id)
            .context("That version is not of this paper")?,
        None => match revisions.first() {
            Some(newest) => newest,
            None => {
                return Ok(Paper {
                    id: found.id,
                    title: found.title,
                    description: found.description,
                    versions: vec![],
                    shown: String::new(),
                    editable: false,
                    items: vec![],
                    loose: vec![],
                    level: None,
                    reruns: vec![],
                    capsules: vec![],
                })
            }
        },
    };
    // In the outline's order, as the WebView's manuscript tree lists them.
    let stored = store.list_publication_items(&shown.id).await?;
    let outline: Vec<_> = stored
        .iter()
        .map(|item| wisp_dto::PublicationItemInfo {
            id: item.id.clone(),
            revision_id: item.revision_id.clone(),
            parent_item_id: item.parent_item_id.clone(),
            kind: item.kind.as_str().to_owned(),
            title: item.title.clone(),
            content: String::new(),
            ordinal: item.ordinal,
        })
        .collect();
    let mut items: Vec<PaperItem> = wisp_dto::publication_sources::manuscript_rows(&outline)
        .into_iter()
        .filter_map(|(row, depth)| {
            let item = stored.iter().find(|item| item.id == row.id)?;
            Some(PaperItem {
                id: item.id.clone(),
                kind: PaperItemKind::of(item.kind),
                title: item.title.clone(),
                note: item.content.clone(),
                parent: item.parent_item_id.clone(),
                depth,
                sources: vec![],
            })
        })
        .collect();
    let drift = store.list_publication_evidence_drift(&shown.id).await?;
    let mut loose = vec![];
    for binding in store.list_evidence_bindings(&shown.id).await? {
        let item = binding.item_id.clone();
        let bound = source(store, binding, &drift).await?;
        match items
            .iter_mut()
            .find(|candidate| Some(&candidate.id) == item.as_ref())
        {
            Some(owner) => owner.sources.push(bound),
            None => loose.push(bound),
        }
    }
    let runs = store.list_reproduction_runs(&shown.id).await?;
    let mut reruns = Vec::with_capacity(runs.len());
    for rerun in &runs {
        let outputs = store
            .list_reproduction_results(&rerun.id)
            .await?
            .into_iter()
            .map(|result| PaperRerunOutput {
                path: result.output_path,
                comparator: result.comparator_kind.as_str().to_owned(),
                passed: result.passed,
            })
            .collect();
        reruns.push(PaperRerun {
            id: rerun.id.clone(),
            run: rerun.source_run_id.clone(),
            status: match rerun.status.as_str() {
                "running" => PaperRerunStatus::Running,
                "completed" => PaperRerunStatus::Completed,
                _ => PaperRerunStatus::Failed,
            },
            level: PaperLevel::of(rerun.capability_level),
            environment_matched: rerun.environment_matched,
            exit_code: rerun.exit_code,
            error: rerun.error.clone(),
            outputs,
        });
    }
    let capsules = store
        .list_capsule_builds(&shown.id)
        .await?
        .into_iter()
        .map(|build| PaperCapsule {
            id: build.id,
            status: match build.status.as_str() {
                "building" => PaperCapsuleStatus::Building,
                "succeeded" => PaperCapsuleStatus::Succeeded,
                _ => PaperCapsuleStatus::Failed,
            },
            audience: PaperAudience::of(build.visibility),
            path: build.output_path.unwrap_or_default(),
            archive: build.archive_sha256,
            error: build.error,
        })
        .collect();
    let level = (shown.state != PublicationRevisionState::Draft).then(|| {
        PaperLevel::of(wisp_runs::publication_reproduction::effective_capability(
            shown.capability_level,
            shown.manifest_json.as_deref(),
            &runs,
        ))
    });
    Ok(Paper {
        id: found.id,
        title: found.title,
        description: found.description,
        shown: shown.id.clone(),
        editable: shown.state == PublicationRevisionState::Draft,
        versions: revisions
            .iter()
            .map(|revision| PaperVersion {
                id: revision.id.clone(),
                number: revision.revision_number,
                label: revision.label.clone(),
                state: match revision.state {
                    PublicationRevisionState::Draft => PaperVersionState::Draft,
                    PublicationRevisionState::Frozen => PaperVersionState::Finalized,
                    PublicationRevisionState::Published => PaperVersionState::Published,
                    PublicationRevisionState::Freezing | PublicationRevisionState::Deleting => {
                        PaperVersionState::Busy
                    }
                },
                fingerprint: revision.manifest_sha256.clone().unwrap_or_default(),
            })
            .collect(),
        items,
        loose,
        level,
        reruns,
        capsules,
    })
}

/// Starts a paper with its first version.
pub(crate) async fn create(store: &Store, project: &str, input: PaperInput) -> Result<Paper> {
    let name = title(&input.title)?;
    let description = note(&input.description)?;
    let version = match input.version.trim() {
        "" => FIRST_VERSION.to_owned(),
        typed => title(typed)?,
    };
    let id = uuid::Uuid::new_v4().to_string();
    store
        .create_publication(&id, project, &name, &description)
        .await?;
    let revision = uuid::Uuid::new_v4().to_string();
    if let Err(error) = store
        .create_publication_revision(&revision, &id, None, &version)
        .await
    {
        // A paper without a version could not be worked on.
        let _ = store.delete_publication(&id).await;
        return Err(error);
    }
    open(store, project, &id, None).await
}

pub(crate) async fn rename(
    store: &Store,
    project: &str,
    paper: &str,
    name: &str,
    description: &str,
) -> Result<()> {
    owned(store, project, paper).await?;
    store
        .update_publication(paper, &title(name)?, &note(description)?)
        .await
}

/// Removes a paper with its drafts. One with a finalized version stays.
pub(crate) async fn remove(store: &Store, project: &str, paper: &str) -> Result<()> {
    owned(store, project, paper).await?;
    let finalized = store
        .list_publication_revisions(paper)
        .await?
        .iter()
        .any(|revision| revision.state != PublicationRevisionState::Draft);
    ensure!(
        !finalized,
        "A paper with a finalized version cannot be removed"
    );
    store.delete_publication(paper).await
}

/// Adds a figure, table or claim to a draft version, or changes one,
/// under `input.parent` or at the top level. The store refuses a parent of
/// another version and one that would put the item inside itself.
pub(crate) async fn save_item(
    store: &Store,
    project: &str,
    version: &str,
    input: PaperItemInput,
) -> Result<()> {
    draft(store, project, version).await?;
    let existing = store.list_publication_items(version).await?;
    // After every item, so it never takes a sibling's place.
    let last = existing
        .iter()
        .map(|item| item.ordinal)
        .max()
        .map_or(0, |last| last + 1);
    let parent = input.parent.filter(|parent| !parent.trim().is_empty());
    let item = match input.id.as_str() {
        "" => PublicationItem {
            id: uuid::Uuid::new_v4().to_string(),
            revision_id: version.to_owned(),
            parent_item_id: parent,
            kind: input.kind.stored(),
            title: title(&input.title)?,
            content: note(&input.note)?,
            ordinal: last,
            metadata_json: "{}".into(),
            created_at: 0,
            updated_at: 0,
        },
        id => {
            let Some(kept) = existing.into_iter().find(|item| item.id == id) else {
                bail!("That item is not in this version");
            };
            // Moved under another item, it goes last among its new siblings.
            let ordinal = if kept.parent_item_id == parent {
                kept.ordinal
            } else {
                last
            };
            PublicationItem {
                kind: input.kind.stored(),
                title: title(&input.title)?,
                content: note(&input.note)?,
                parent_item_id: parent,
                ordinal,
                ..kept
            }
        }
    };
    store.save_publication_item(&item).await
}

pub(crate) async fn remove_item(
    store: &Store,
    project: &str,
    version: &str,
    item: &str,
) -> Result<()> {
    draft(store, project, version).await?;
    ensure!(
        store
            .list_publication_items(version)
            .await?
            .iter()
            .any(|found| found.id == item),
        "That item is not in this version"
    );
    store.delete_publication_item(item).await
}

/// The project's results, runs or conversation messages to choose evidence
/// from, 50 at a time. Messages are the users' and the assistant's in the
/// project's own conversations (not explorations), newest first, matched
/// by the conversation's name or the text.
pub(crate) async fn choices(
    store: &Store,
    project: &str,
    kind: PaperSourceKind,
    query: &str,
    offset: u32,
) -> Result<(Vec<PaperSourceChoice>, bool)> {
    let category = match kind {
        PaperSourceKind::Result => "files",
        PaperSourceKind::Run => "runs",
        PaperSourceKind::Passage => "messages",
        _ => bail!("Only results, runs and conversation passages can be chosen here"),
    };
    let page = store
        .publication_source_page(project, category, query, offset)
        .await?;
    Ok((
        page.sources
            .into_iter()
            .map(|choice| PaperSourceChoice {
                kind,
                id: choice.id.clone(),
                name: choice.title.clone(),
                detail: choice.detail.clone(),
                message: (kind == PaperSourceKind::Passage).then_some(choice),
            })
            .collect(),
        page.has_more,
    ))
}

/// Binds a result (at the version it has now), a run, a passage of a
/// conversation message, or an exact anchor (a tool call, an execution, its
/// code, an outside resource) to an item of a draft version, with why it
/// was chosen, whether it is meant for the paper and who may see it. A
/// passage's `source` is its exact locator
/// (`wisp_dto::publication_sources::message_span_locator`); a message
/// changed since it was shown is refused. The store checks an anchor is of
/// this project and copies what it names into the binding.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn bind(
    store: &Store,
    project: &str,
    version: &str,
    item: &str,
    kind: PaperSourceKind,
    source: &str,
    purpose: &str,
    selection: PaperSelection,
    audience: PaperAudience,
) -> Result<()> {
    draft(store, project, version).await?;
    ensure!(
        store
            .list_publication_items(version)
            .await?
            .iter()
            .any(|found| found.id == item),
        "That item is not in this version"
    );
    let source_kind = match kind {
        PaperSourceKind::Result => {
            let context = store
                .get_artifact_version_context(source)
                .await?
                .context("That result is gone")?;
            ensure!(
                context.project_id == project,
                "That result is not in this project"
            );
            EvidenceSourceKind::ArtifactVersion
        }
        PaperSourceKind::Run => {
            let run = store.get_run(source).await?.context("That run is gone")?;
            ensure!(run.project_id == project, "That run is not in this project");
            EvidenceSourceKind::Run
        }
        // The store checks the message is of this project and unchanged,
        // and keeps the passage's text with the binding; it checks and
        // keeps the exact anchors the same way.
        PaperSourceKind::Passage => EvidenceSourceKind::MessageSpan,
        PaperSourceKind::ToolCall => EvidenceSourceKind::ToolCall,
        PaperSourceKind::ExecutionLog => EvidenceSourceKind::ExecutionLog,
        PaperSourceKind::CodeCell => EvidenceSourceKind::CodeCell,
        PaperSourceKind::Resource => EvidenceSourceKind::ExternalResource,
    };
    store
        .save_evidence_binding(&EvidenceBindingDraft {
            id: uuid::Uuid::new_v4().to_string(),
            revision_id: version.to_owned(),
            item_id: Some(item.to_owned()),
            source_kind,
            source_id: source.to_owned(),
            purpose: note(purpose)?,
            supported_claim_item_id: None,
            selection_state: selection.stored(),
            visibility: audience.stored(),
        })
        .await?;
    Ok(())
}

/// Changes whether a piece of evidence of a draft version is meant for the
/// paper and who may see it, as the WebView card's two choices do.
pub(crate) async fn mark(
    store: &Store,
    project: &str,
    version: &str,
    binding: &str,
    selection: PaperSelection,
    audience: PaperAudience,
) -> Result<()> {
    draft(store, project, version).await?;
    ensure!(
        store
            .list_evidence_bindings(version)
            .await?
            .iter()
            .any(|found| found.id == binding),
        "That evidence is not in this version"
    );
    store
        .update_evidence_binding_selection(binding, selection.stored(), audience.stored())
        .await
}

pub(crate) async fn unbind(
    store: &Store,
    project: &str,
    version: &str,
    binding: &str,
) -> Result<()> {
    draft(store, project, version).await?;
    ensure!(
        store
            .list_evidence_bindings(version)
            .await?
            .iter()
            .any(|found| found.id == binding),
        "That evidence is not in this version"
    );
    store.delete_evidence_binding(binding).await
}

/// Checks a draft version against `policy`, and when `finalize` is set
/// and nothing blocks it, locks the version with its evidence.
pub(crate) async fn check(
    store: &Store,
    project: &str,
    version: &str,
    policy: PaperPolicy,
    finalize: bool,
) -> Result<PaperCheck> {
    draft(store, project, version).await?;
    let outcome = crate::paper_freeze::prepare_or_freeze_publication(
        store,
        version,
        policy.stored(),
        !finalize,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    let readiness = outcome.readiness;
    Ok(PaperCheck {
        finalized: outcome.frozen,
        level: PaperLevel::of(readiness.capability_level),
        blockers: readiness.blockers.into_iter().map(finding).collect(),
        warnings: readiness.warnings.into_iter().map(finding).collect(),
        omissions: readiness.omissions.into_iter().map(finding).collect(),
        fingerprint: readiness.manifest_sha256,
        can_finalize: readiness.can_freeze,
    })
}

/// Records why a finding of a draft version may stand; the next check
/// counts it as waived.
pub(crate) async fn waive(
    store: &Store,
    project: &str,
    version: &str,
    code: &str,
    reason: &str,
) -> Result<()> {
    draft(store, project, version).await?;
    let reason = note(reason)?;
    ensure!(!reason.is_empty(), "Say why this can stand");
    store
        .save_publication_waiver(&PublicationWaiver {
            id: uuid::Uuid::new_v4().to_string(),
            revision_id: version.to_owned(),
            finding_code: code.to_owned(),
            author: WAIVER_AUTHOR.to_owned(),
            reason,
            created_at: 0,
        })
        .await
}

/// Starts a draft from `version` with its items and evidence, called
/// `label` or, when that is empty, after its number. Returns the new
/// version.
pub(crate) async fn new_version(
    store: &Store,
    project: &str,
    version: &str,
    label: &str,
) -> Result<String> {
    let found = store
        .get_publication_revision(version)
        .await?
        .context("That version is gone")?;
    owned(store, project, &found.publication_id).await?;
    let label = match label.trim() {
        "" => {
            let next = store
                .list_publication_revisions(&found.publication_id)
                .await?
                .iter()
                .map(|revision| revision.revision_number)
                .max()
                .unwrap_or(0)
                + 1;
            format!("Version {next}")
        }
        typed => title(typed)?,
    };
    let id = uuid::Uuid::new_v4().to_string();
    store
        .clone_publication_revision(version, &id, &label)
        .await?;
    Ok(id)
}

/// Whether the version is a finalized (or published) version of a paper of
/// this project.
async fn finalized(store: &Store, project: &str, version: &str) -> Result<()> {
    let found = store
        .get_publication_revision(version)
        .await?
        .context("That version is gone")?;
    owned(store, project, &found.publication_id).await?;
    ensure!(
        matches!(
            found.state,
            PublicationRevisionState::Frozen | PublicationRevisionState::Published
        ),
        "Only a finalized version can be rerun or saved as a capsule"
    );
    Ok(())
}

/// Reruns `run` of a finalized version in a fresh temporary folder that
/// holds only the version's frozen inputs and code, through `runner`, and
/// records how each declared output compares (SHA-256, as the WebView's
/// button asks). The version itself never changes. A rerun that could
/// not start is refused; one that ran and failed is recorded.
pub(crate) async fn verify(
    store: &Store,
    project: &str,
    version: &str,
    run: &str,
    runner: &dyn wisp_runs::RunCommandRunner,
) -> Result<()> {
    finalized(store, project, version).await?;
    wisp_runs::publication_reproduction::verify_publication_revision_with_runner(
        store,
        version,
        run,
        &[],
        runner,
        crate::paper_freeze::capsule_security_violations,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    Ok(())
}

/// Writes the evidence capsule of a finalized version to `destination`, a
/// new `.zip` file where the person chose to save it, and records the
/// attempt with the version. Only the version's frozen snapshots are read.
pub(crate) async fn build_capsule(
    store: &Store,
    project: &str,
    version: &str,
    destination: &Path,
) -> Result<()> {
    finalized(store, project, version).await?;
    ensure!(
        destination.is_absolute()
            && destination
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("zip")),
        "Choose where to save the capsule as a .zip file"
    );
    wisp_runs::publication_capsule::build_publication_capsule_to(
        store,
        version,
        destination,
        CAPSULE_HOST,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    Ok(())
}

/// Writes a capsule's entries as the WebView host does: deflated at level
/// 6, with the zero timestamp and mode 0644, so a version gives the same
/// bytes each time.
struct ZipCapsule(zip::ZipWriter<std::fs::File>);

impl CapsuleArchive for ZipCapsule {
    fn start_file(&mut self, path: &str, size_bytes: u64) -> Result<(), String> {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(6))
            .last_modified_time(zip::DateTime::default())
            .large_file(size_bytes > u32::MAX as u64)
            .unix_permissions(0o644);
        self.0
            .start_file(path, options)
            .map_err(|error| error.to_string())
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), String> {
        std::io::Write::write_all(&mut self.0, bytes).map_err(|error| error.to_string())
    }

    fn finish(self: Box<Self>) -> Result<std::fs::File, String> {
        self.0.finish().map_err(|error| error.to_string())
    }
}

fn open_zip(file: std::fs::File) -> Box<dyn CapsuleArchive> {
    Box::new(ZipCapsule(zip::ZipWriter::new(file)))
}

const CAPSULE_HOST: CapsuleHost = CapsuleHost {
    scan: crate::paper_freeze::capsule_security_violations,
    open: open_zip,
};
