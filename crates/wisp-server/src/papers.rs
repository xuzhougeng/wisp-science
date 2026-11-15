//! Paper evidence: the papers a project is writing, their versions, and
//! for each figure, table or claim the results and runs it rests on.
//!
//! This is the store's publication model seen from the desktop: papers,
//! versions, items and the evidence bound to them, the readiness check,
//! waivers, finalizing a version (see `paper_freeze`) and starting a new
//! version from one. Capsules and reproduction runs are not done from here.

use anyhow::{bail, ensure, Context, Result};
use wisp_store::{
    EvidenceBinding, EvidenceBindingDraft, EvidenceSelectionState, EvidenceSourceKind,
    EvidenceVisibility, PublicationCapabilityLevel, PublicationFreezePolicy, PublicationItem,
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

/// What an item of a paper is. The store knows more kinds than the form
/// offers; those are shown under their own name.
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
    /// The kinds the form offers.
    pub const OFFERED: [PaperItemKind; 3] = [
        PaperItemKind::Figure,
        PaperItemKind::Table,
        PaperItemKind::Claim,
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
    /// A passage of a conversation, a tool call and the like, bound by
    /// the WebView desktop.
    Other,
}

/// One result or run a figure, table or claim rests on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperSource {
    /// The binding, to remove it.
    pub id: String,
    pub kind: PaperSourceKind,
    /// The file or run as it is called, when it can still be told.
    pub name: String,
    /// Why it was chosen.
    pub purpose: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperItem {
    pub id: String,
    pub kind: PaperItemKind,
    pub title: String,
    pub note: String,
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
}

/// A result or run offered as evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperSourceChoice {
    pub kind: PaperSourceKind,
    pub id: String,
    pub name: String,
    /// The result's version, or the run's state.
    pub detail: String,
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

async fn source(store: &Store, binding: EvidenceBinding) -> PaperSource {
    let (kind, name) = match binding.source_kind {
        EvidenceSourceKind::Run => (
            PaperSourceKind::Run,
            store
                .get_run(&binding.source_id)
                .await
                .ok()
                .flatten()
                .map(|run| run.title),
        ),
        EvidenceSourceKind::ArtifactVersion => (
            PaperSourceKind::Result,
            store
                .get_artifact_version_context(&binding.source_id)
                .await
                .ok()
                .flatten()
                .map(|context| context.filename),
        ),
        _ => (PaperSourceKind::Other, None),
    };
    PaperSource {
        id: binding.id,
        kind,
        name: name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| binding.source_kind.as_str().replace('_', " ")),
        purpose: binding.purpose,
    }
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
                })
            }
        },
    };
    let mut items: Vec<PaperItem> = store
        .list_publication_items(&shown.id)
        .await?
        .into_iter()
        .map(|item| PaperItem {
            id: item.id,
            kind: PaperItemKind::of(item.kind),
            title: item.title,
            note: item.content,
            sources: vec![],
        })
        .collect();
    let mut loose = vec![];
    for binding in store.list_evidence_bindings(&shown.id).await? {
        let item = binding.item_id.clone();
        let bound = source(store, binding).await;
        match items
            .iter_mut()
            .find(|candidate| Some(&candidate.id) == item.as_ref())
        {
            Some(owner) => owner.sources.push(bound),
            None => loose.push(bound),
        }
    }
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

/// Adds a figure, table or claim to a draft version, or changes one.
pub(crate) async fn save_item(
    store: &Store,
    project: &str,
    version: &str,
    input: PaperItemInput,
) -> Result<()> {
    draft(store, project, version).await?;
    let existing = store.list_publication_items(version).await?;
    let item = match input.id.as_str() {
        "" => PublicationItem {
            id: uuid::Uuid::new_v4().to_string(),
            revision_id: version.to_owned(),
            parent_item_id: None,
            kind: input.kind.stored(),
            title: title(&input.title)?,
            content: note(&input.note)?,
            ordinal: existing
                .iter()
                .map(|item| item.ordinal)
                .max()
                .map_or(0, |last| last + 1),
            metadata_json: "{}".into(),
            created_at: 0,
            updated_at: 0,
        },
        id => {
            let Some(kept) = existing.into_iter().find(|item| item.id == id) else {
                bail!("That item is not in this version");
            };
            PublicationItem {
                kind: input.kind.stored(),
                title: title(&input.title)?,
                content: note(&input.note)?,
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

/// The project's results or runs to choose evidence from, 50 at a time.
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
        PaperSourceKind::Other => bail!("Only results and runs can be chosen here"),
    };
    let page = store
        .publication_source_page(project, category, query, offset)
        .await?;
    Ok((
        page.sources
            .into_iter()
            .map(|choice| PaperSourceChoice {
                kind,
                id: choice.id,
                name: choice.title,
                detail: choice.detail,
            })
            .collect(),
        page.has_more,
    ))
}

/// Binds a result (at the version it has now) or a run to an item of a
/// draft version, with why it was chosen.
pub(crate) async fn bind(
    store: &Store,
    project: &str,
    version: &str,
    item: &str,
    kind: PaperSourceKind,
    source: &str,
    purpose: &str,
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
        PaperSourceKind::Other => bail!("Only results and runs can be chosen here"),
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
            selection_state: EvidenceSelectionState::Selected,
            visibility: EvidenceVisibility::Private,
        })
        .await?;
    Ok(())
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
        level: match readiness.capability_level {
            PublicationCapabilityLevel::Archived => PaperLevel::Archived,
            PublicationCapabilityLevel::Traceable => PaperLevel::Traceable,
            PublicationCapabilityLevel::ReExecutable => PaperLevel::Rerunnable,
            PublicationCapabilityLevel::Reproduced => PaperLevel::Reproduced,
        },
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
