//! Project-scoped Publication Workspace commands.

use crate::publication_reproduction::{
    effective_capability, verify_publication_revision as run_publication_verification,
    ReproductionComparisonRequest,
};
use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;
use wisp_store::{
    canonical_json_sha256, ArtifactCaptureTiming, CapsuleBuild, EvidenceBinding,
    EvidenceBindingDraft, EvidenceReview, EvidenceSelectionState, EvidenceSourceKind,
    EvidenceSupersession, EvidenceVisibility, LineageBasis, LineageConfidence, Publication,
    PublicationEvidenceDrift, PublicationItem, PublicationItemKind, PublicationItemLink,
    PublicationReadiness, PublicationReadinessFinding, PublicationRevision, PublicationWaiver,
    ReproductionResult, ReproductionRun, Store,
};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PublicationLineageSummary {
    pub binding_id: String,
    pub source_label: String,
    pub quality: LineageConfidence,
    pub bases: Vec<LineageBasis>,
    pub exact_version_id: Option<String>,
    pub version_number: Option<i64>,
    pub checksum: Option<String>,
    pub capture_timing: Option<ArtifactCaptureTiming>,
    pub producing_run_id: Option<String>,
    pub producing_run_title: Option<String>,
    pub input_labels: Vec<String>,
    pub code_labels: Vec<String>,
    pub run_input_count: usize,
    pub run_output_count: usize,
    pub code_snapshot_count: usize,
    pub environment_captured: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PublicationWorkspace {
    pub publications: Vec<Publication>,
    pub publication: Option<Publication>,
    pub revisions: Vec<PublicationRevision>,
    pub revision: Option<PublicationRevision>,
    pub items: Vec<PublicationItem>,
    pub item_links: Vec<PublicationItemLink>,
    pub bindings: Vec<EvidenceBinding>,
    pub reviews: Vec<EvidenceReview>,
    pub supersessions: Vec<EvidenceSupersession>,
    pub waivers: Vec<PublicationWaiver>,
    pub readiness: Option<PublicationReadiness>,
    pub drift: Vec<PublicationEvidenceDrift>,
    pub lineage: Vec<PublicationLineageSummary>,
    pub capsule_builds: Vec<CapsuleBuild>,
    pub effective_capability_level: Option<String>,
    pub reproduction_runs: Vec<ReproductionRun>,
    pub reproduction_results: Vec<ReproductionResult>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkspaceSourceKind {
    Artifact,
    ArtifactVersion,
    Run,
    ExecutionLog,
    MessageSpan,
    ToolCall,
    CodeCell,
    ExternalResource,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatePublicationInput {
    title: String,
    #[serde(default)]
    description: String,
    revision_label: String,
}

impl CreatePublicationInput {
    pub(crate) fn new(title: String, description: String, revision_label: String) -> Self {
        Self {
            title,
            description,
            revision_label,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavePublicationItemInput {
    id: Option<String>,
    revision_id: String,
    parent_item_id: Option<String>,
    kind: PublicationItemKind,
    title: String,
    #[serde(default)]
    content: String,
    ordinal: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BindPublicationEvidenceInput {
    revision_id: String,
    item_id: Option<String>,
    source_kind: WorkspaceSourceKind,
    source_id: String,
    #[serde(default)]
    purpose: String,
    supported_claim_item_id: Option<String>,
    selection_state: EvidenceSelectionState,
    visibility: EvidenceVisibility,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateEvidenceBindingInput {
    binding_id: String,
    selection_state: EvidenceSelectionState,
    visibility: EvidenceVisibility,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavePublicationWaiverInput {
    revision_id: String,
    finding_code: String,
    author: String,
    reason: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VerifyPublicationRevisionInput {
    revision_id: String,
    source_run_id: String,
    #[serde(default)]
    comparisons: Vec<ReproductionComparisonRequest>,
}

async fn revision_project(store: &Store, revision_id: &str) -> anyhow::Result<String> {
    let revision = store
        .get_publication_revision(revision_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Publication revision not found"))?;
    let publication = store
        .get_publication(&revision.publication_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Publication not found"))?;
    Ok(publication.project_id)
}

pub(crate) async fn validate_publication_revision(
    state: &AppState,
    window_label: &str,
    revision_id: &str,
) -> Result<(), String> {
    let project = publication_mainline_project(state, window_label).await?;
    if revision_project(&state.store, revision_id)
        .await
        .map_err(|e| e.to_string())?
        != project.id
    {
        return Err("Publication revision does not belong to the active project".into());
    }
    Ok(())
}

fn readiness_from_report(
    report: wisp_store::PublicationReadinessReport,
) -> anyhow::Result<PublicationReadiness> {
    let blockers = serde_json::from_str::<Vec<PublicationReadinessFinding>>(&report.blockers_json)?;
    let warnings = serde_json::from_str::<Vec<PublicationReadinessFinding>>(&report.warnings_json)?;
    let omissions =
        serde_json::from_str::<Vec<PublicationReadinessFinding>>(&report.omissions_json)?;
    let can_freeze = blockers.iter().all(|finding| finding.waived);
    Ok(PublicationReadiness {
        revision_id: report.revision_id,
        target_visibility: report.target_visibility,
        capability_level: report.capability_level,
        blockers,
        warnings,
        omissions,
        manifest_json: report.manifest_json,
        manifest_sha256: report.manifest_sha256,
        can_freeze,
    })
}

fn environment_captured(snapshot: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(snapshot) {
        Ok(serde_json::Value::Object(values)) => !values.is_empty(),
        Ok(serde_json::Value::Array(values)) => !values.is_empty(),
        Ok(value) => !value.is_null(),
        Err(_) => !snapshot.trim().is_empty(),
    }
}

async fn lineage_summary(
    store: &Store,
    binding: &EvidenceBinding,
) -> anyhow::Result<PublicationLineageSummary> {
    let mut source_label = binding.source_id.clone();
    let mut exact_version_id = None;
    let mut version_number = None;
    let mut checksum = None;
    let mut capture_timing = None;
    let mut producing_run_id = binding.run_id.clone();
    let mut producing_run_title = None;
    let mut input_labels = Vec::new();
    let mut code_labels = Vec::new();
    let mut environment = false;
    let mut anchored = false;

    if let Some(version_id) = binding.artifact_version_id.as_deref() {
        if let Some(context) = store.get_artifact_version_context(version_id).await? {
            source_label = context.filename;
            exact_version_id = Some(context.version.id);
            version_number = Some(context.version.version_number);
            checksum = context.version.checksum;
            capture_timing = Some(context.version.capture_timing);
            producing_run_id = context.version.producing_run_id;
            environment = context.version.env_snapshot_hash.is_some();
        }
    } else if matches!(
        binding.source_kind,
        EvidenceSourceKind::ExecutionLog
            | EvidenceSourceKind::MessageSpan
            | EvidenceSourceKind::ToolCall
            | EvidenceSourceKind::CodeCell
            | EvidenceSourceKind::ExternalResource
    ) {
        if let Ok(snapshot) =
            serde_json::from_str::<serde_json::Value>(&binding.source_snapshot_json)
        {
            anchored = snapshot.get("anchor").is_some_and(|anchor| {
                snapshot
                    .get("anchor_sha256")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|hash| hash == canonical_json_sha256(anchor).1)
            });
            source_label = match binding.source_kind {
                EvidenceSourceKind::MessageSpan => snapshot
                    .pointer("/anchor/text_snapshot")
                    .and_then(serde_json::Value::as_str)
                    .map(|text| text.chars().take(80).collect())
                    .unwrap_or(source_label),
                EvidenceSourceKind::ToolCall => snapshot
                    .pointer("/anchor/name")
                    .and_then(serde_json::Value::as_str)
                    .map(|name| format!("Tool result: {name}"))
                    .unwrap_or(source_label),
                EvidenceSourceKind::ExecutionLog | EvidenceSourceKind::CodeCell => {
                    let language = snapshot
                        .pointer("/anchor/language")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("Execution");
                    let cell = snapshot
                        .pointer("/anchor/cell_index")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or_default();
                    format!("{language} cell {cell}")
                }
                EvidenceSourceKind::ExternalResource => snapshot
                    .pointer("/anchor/uri")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .unwrap_or(source_label),
                _ => source_label,
            };
        }
    }

    let mut bases = Vec::new();
    let mut run_input_count = 0;
    let mut run_output_count = 0;
    let mut code_snapshot_count = 0;
    let mut quality = if anchored {
        LineageConfidence::Exact
    } else {
        LineageConfidence::Uncertain
    };
    if let Some(run_id) = producing_run_id.as_deref() {
        if let Some(run) = store.get_run(run_id).await? {
            producing_run_title = Some(run.title.clone());
            source_label = if binding.source_kind == EvidenceSourceKind::Run {
                run.title.clone()
            } else {
                source_label
            };
            environment |= environment_captured(&run.env_snapshot_json);
            let inputs = store.list_run_inputs(run_id).await?;
            let outputs = store.list_run_outputs(run_id).await?;
            let code = store.list_run_code_snapshots(run_id).await?;
            input_labels = inputs
                .iter()
                .map(|input| input.source_ref.clone())
                .collect();
            code_labels = code
                .iter()
                .map(|code| {
                    code.source_path
                        .clone()
                        .unwrap_or_else(|| code.source_kind.clone())
                })
                .collect();
            run_input_count = inputs.len();
            run_output_count = outputs.len();
            code_snapshot_count = code.len();
            for input in &inputs {
                if !bases.contains(&input.basis) {
                    bases.push(input.basis);
                }
            }
            quality = if inputs
                .iter()
                .any(|input| input.confidence == LineageConfidence::Uncertain)
            {
                LineageConfidence::Uncertain
            } else if !inputs.is_empty()
                && inputs
                    .iter()
                    .all(|input| input.confidence == LineageConfidence::Exact)
                && !code.is_empty()
                && environment
            {
                LineageConfidence::Exact
            } else {
                LineageConfidence::Likely
            };
        }
    }

    Ok(PublicationLineageSummary {
        binding_id: binding.id.clone(),
        source_label,
        quality,
        bases,
        exact_version_id,
        version_number,
        checksum,
        capture_timing,
        producing_run_id,
        producing_run_title,
        input_labels,
        code_labels,
        run_input_count,
        run_output_count,
        code_snapshot_count,
        environment_captured: environment,
    })
}

async fn publication_mainline_project(
    state: &AppState,
    window_label: &str,
) -> Result<crate::ActiveProject, String> {
    let (project, scope) =
        crate::exploration_commands::working_project_for_active_frame(state, window_label).await?;
    if matches!(scope, wisp_store::StateScope::Exploration { .. }) {
        return Err(
            "exploration_project_mutation_blocked: Publication Workspace is unavailable inside an exploration."
                .into(),
        );
    }
    Ok(project)
}

async fn writable_publication_mainline_project(
    state: &AppState,
    window_label: &str,
) -> Result<(crate::ActiveProject, tokio::sync::OwnedRwLockReadGuard<()>), String> {
    let project = publication_mainline_project(state, window_label).await?;
    let activity = state.begin_project_activity(&project.id)?;
    crate::exploration_commands::require_writable_scope(
        &state.store,
        &wisp_store::StateScope::mainline(project.id.clone()),
    )
    .await?;
    Ok((project, activity))
}

pub(crate) async fn publication_workspace(
    store: &Store,
    project_id: &str,
    publication_id: Option<&str>,
    revision_id: Option<&str>,
) -> anyhow::Result<PublicationWorkspace> {
    let publications = store.list_publications(project_id).await?;
    let explicit_revision = match revision_id {
        Some(id) => Some(
            store
                .get_publication_revision(id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Publication revision not found"))?,
        ),
        None => None,
    };
    let selected_publication_id = explicit_revision
        .as_ref()
        .map(|revision| revision.publication_id.as_str())
        .or(publication_id)
        .or_else(|| {
            publications
                .first()
                .map(|publication| publication.id.as_str())
        });
    let publication = selected_publication_id
        .and_then(|id| publications.iter().find(|publication| publication.id == id))
        .cloned();
    if selected_publication_id.is_some() && publication.is_none() {
        anyhow::bail!("Publication does not belong to the active project");
    }
    if let (Some(requested), Some(selected)) = (publication_id, publication.as_ref()) {
        if requested != selected.id {
            anyhow::bail!("Publication revision does not belong to the selected Publication");
        }
    }

    let revisions = match publication.as_ref() {
        Some(publication) => store.list_publication_revisions(&publication.id).await?,
        None => Vec::new(),
    };
    let revision = explicit_revision
        .or_else(|| revisions.first().cloned())
        .filter(|selected| {
            publication
                .as_ref()
                .is_some_and(|publication| selected.publication_id == publication.id)
        });
    if revision_id.is_some() && revision.is_none() {
        anyhow::bail!("Publication revision does not belong to the selected Publication");
    }

    let Some(revision) = revision else {
        return Ok(PublicationWorkspace {
            publications,
            publication,
            revisions,
            revision: None,
            items: Vec::new(),
            item_links: Vec::new(),
            bindings: Vec::new(),
            reviews: Vec::new(),
            supersessions: Vec::new(),
            waivers: Vec::new(),
            readiness: None,
            drift: Vec::new(),
            lineage: Vec::new(),
            capsule_builds: Vec::new(),
            effective_capability_level: None,
            reproduction_runs: Vec::new(),
            reproduction_results: Vec::new(),
        });
    };

    let items = store.list_publication_items(&revision.id).await?;
    let item_links = store.list_publication_item_links(&revision.id).await?;
    let bindings = store.list_evidence_bindings(&revision.id).await?;
    let mut reviews = Vec::new();
    let mut lineage = Vec::with_capacity(bindings.len());
    for binding in &bindings {
        reviews.extend(store.list_evidence_reviews(&binding.id).await?);
        lineage.push(lineage_summary(store, binding).await?);
    }
    let supersessions = store.list_evidence_supersessions(&revision.id).await?;
    let waivers = store.list_publication_waivers(&revision.id).await?;
    let readiness = store
        .get_publication_readiness_report(&revision.id)
        .await?
        .map(readiness_from_report)
        .transpose()?;
    let drift = store.list_publication_evidence_drift(&revision.id).await?;
    let capsule_builds = store.list_capsule_builds(&revision.id).await?;
    let reproduction_runs = store.list_reproduction_runs(&revision.id).await?;
    let mut reproduction_results = Vec::new();
    for reproduction in &reproduction_runs {
        reproduction_results.extend(store.list_reproduction_results(&reproduction.id).await?);
    }
    let effective_capability_level = Some(
        effective_capability(
            revision.capability_level,
            revision.manifest_json.as_deref(),
            &reproduction_runs,
        )
        .as_str()
        .to_string(),
    );

    Ok(PublicationWorkspace {
        publications,
        publication,
        revisions,
        revision: Some(revision),
        items,
        item_links,
        bindings,
        reviews,
        supersessions,
        waivers,
        readiness,
        drift,
        lineage,
        capsule_builds,
        effective_capability_level,
        reproduction_runs,
        reproduction_results,
    })
}

pub(crate) async fn create_publication(
    store: &Store,
    project_id: &str,
    input: &CreatePublicationInput,
) -> anyhow::Result<PublicationRevision> {
    let publication_id = uuid::Uuid::new_v4().to_string();
    store
        .create_publication(
            &publication_id,
            project_id,
            &input.title,
            &input.description,
        )
        .await?;
    let revision_id = uuid::Uuid::new_v4().to_string();
    match store
        .create_publication_revision(&revision_id, &publication_id, None, &input.revision_label)
        .await
    {
        Ok(revision) => Ok(revision),
        Err(error) => {
            let _ = store.delete_publication(&publication_id).await;
            Err(error)
        }
    }
}

async fn bind_evidence(
    store: &Store,
    project_id: &str,
    input: &BindPublicationEvidenceInput,
) -> anyhow::Result<EvidenceBinding> {
    if revision_project(store, &input.revision_id).await? != project_id {
        anyhow::bail!("Publication revision does not belong to the active project");
    }
    let (source_kind, source_id) = match input.source_kind {
        // The store validates project ownership while resolving this exact ID.
        WorkspaceSourceKind::ArtifactVersion => {
            (EvidenceSourceKind::ArtifactVersion, input.source_id.clone())
        }
        WorkspaceSourceKind::Artifact => {
            let context = store
                .get_latest_artifact_version_context(&input.source_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Artifact has no exact version to bind"))?;
            if context.project_id != project_id {
                anyhow::bail!("Artifact does not belong to the active project");
            }
            (EvidenceSourceKind::ArtifactVersion, context.version.id)
        }
        WorkspaceSourceKind::Run => {
            let run = store
                .get_run(&input.source_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Run not found"))?;
            if run.project_id != project_id {
                anyhow::bail!("Run does not belong to the active project");
            }
            (EvidenceSourceKind::Run, run.id)
        }
        WorkspaceSourceKind::ExecutionLog => {
            (EvidenceSourceKind::ExecutionLog, input.source_id.clone())
        }
        WorkspaceSourceKind::MessageSpan => {
            (EvidenceSourceKind::MessageSpan, input.source_id.clone())
        }
        WorkspaceSourceKind::ToolCall => (EvidenceSourceKind::ToolCall, input.source_id.clone()),
        WorkspaceSourceKind::CodeCell => (EvidenceSourceKind::CodeCell, input.source_id.clone()),
        WorkspaceSourceKind::ExternalResource => (
            EvidenceSourceKind::ExternalResource,
            input.source_id.clone(),
        ),
    };
    store
        .save_evidence_binding(&EvidenceBindingDraft {
            id: uuid::Uuid::new_v4().to_string(),
            revision_id: input.revision_id.clone(),
            item_id: input.item_id.clone().filter(|id| !id.is_empty()),
            source_kind,
            source_id,
            purpose: input.purpose.clone(),
            supported_claim_item_id: input
                .supported_claim_item_id
                .clone()
                .filter(|id| !id.is_empty()),
            selection_state: input.selection_state,
            visibility: input.visibility,
        })
        .await
}

#[tauri::command]
pub(super) async fn list_publication_sources(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    kind: String,
    query: String,
    offset: u32,
) -> Result<wisp_dto::PublicationSourcePage, String> {
    let project = publication_mainline_project(&state, window.label()).await?;
    state
        .store
        .publication_source_page(&project.id, &kind, &query, offset)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn get_publication_workspace(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    publication_id: Option<String>,
    revision_id: Option<String>,
) -> Result<PublicationWorkspace, String> {
    let project = publication_mainline_project(&state, window.label()).await?;
    publication_workspace(
        &state.store,
        &project.id,
        publication_id.as_deref(),
        revision_id.as_deref(),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn create_publication_workspace(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    input: CreatePublicationInput,
) -> Result<PublicationWorkspace, String> {
    let (project, _activity) =
        writable_publication_mainline_project(&state, window.label()).await?;
    let revision = create_publication(&state.store, &project.id, &input)
        .await
        .map_err(|error| error.to_string())?;
    publication_workspace(&state.store, &project.id, None, Some(&revision.id))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn save_publication_item(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    input: SavePublicationItemInput,
) -> Result<PublicationWorkspace, String> {
    let (project, _activity) =
        writable_publication_mainline_project(&state, window.label()).await?;
    if revision_project(&state.store, &input.revision_id)
        .await
        .map_err(|error| error.to_string())?
        != project.id
    {
        return Err("Publication revision does not belong to the active project".into());
    }
    state
        .store
        .save_publication_item(&PublicationItem {
            id: input
                .id
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            revision_id: input.revision_id.clone(),
            parent_item_id: input.parent_item_id.filter(|id| !id.is_empty()),
            kind: input.kind,
            title: input.title,
            content: input.content,
            ordinal: input.ordinal,
            metadata_json: "{}".into(),
            created_at: 0,
            updated_at: 0,
        })
        .await
        .map_err(|error| error.to_string())?;
    publication_workspace(&state.store, &project.id, None, Some(&input.revision_id))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn bind_publication_evidence(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    input: BindPublicationEvidenceInput,
) -> Result<PublicationWorkspace, String> {
    let (project, _activity) =
        writable_publication_mainline_project(&state, window.label()).await?;
    bind_evidence(&state.store, &project.id, &input)
        .await
        .map_err(|error| error.to_string())?;
    publication_workspace(&state.store, &project.id, None, Some(&input.revision_id))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn update_publication_evidence_binding(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    input: UpdateEvidenceBindingInput,
) -> Result<PublicationWorkspace, String> {
    let (project, _activity) =
        writable_publication_mainline_project(&state, window.label()).await?;
    let binding = state
        .store
        .get_evidence_binding(&input.binding_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Evidence binding not found".to_string())?;
    if revision_project(&state.store, &binding.revision_id)
        .await
        .map_err(|error| error.to_string())?
        != project.id
    {
        return Err("Evidence binding does not belong to the active project".into());
    }
    state
        .store
        .update_evidence_binding_selection(
            &input.binding_id,
            input.selection_state,
            input.visibility,
        )
        .await
        .map_err(|error| error.to_string())?;
    publication_workspace(&state.store, &project.id, None, Some(&binding.revision_id))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn clone_publication_revision(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    revision_id: String,
    label: String,
) -> Result<PublicationWorkspace, String> {
    let (project, _activity) =
        writable_publication_mainline_project(&state, window.label()).await?;
    if revision_project(&state.store, &revision_id)
        .await
        .map_err(|error| error.to_string())?
        != project.id
    {
        return Err("Publication revision does not belong to the active project".into());
    }
    let revision = state
        .store
        .clone_publication_revision(&revision_id, &uuid::Uuid::new_v4().to_string(), &label)
        .await
        .map_err(|error| error.to_string())?;
    publication_workspace(&state.store, &project.id, None, Some(&revision.id))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn save_publication_waiver(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    input: SavePublicationWaiverInput,
) -> Result<PublicationWorkspace, String> {
    let (project, _activity) =
        writable_publication_mainline_project(&state, window.label()).await?;
    if revision_project(&state.store, &input.revision_id)
        .await
        .map_err(|error| error.to_string())?
        != project.id
    {
        return Err("Publication revision does not belong to the active project".into());
    }
    state
        .store
        .save_publication_waiver(&PublicationWaiver {
            id: uuid::Uuid::new_v4().to_string(),
            revision_id: input.revision_id.clone(),
            finding_code: input.finding_code,
            author: input.author,
            reason: input.reason,
            created_at: 0,
        })
        .await
        .map_err(|error| error.to_string())?;
    publication_workspace(&state.store, &project.id, None, Some(&input.revision_id))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn verify_publication_revision(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    input: VerifyPublicationRevisionInput,
) -> Result<PublicationWorkspace, String> {
    let (project, _activity) =
        writable_publication_mainline_project(&state, window.label()).await?;
    if revision_project(&state.store, &input.revision_id)
        .await
        .map_err(|error| error.to_string())?
        != project.id
    {
        return Err("Publication revision does not belong to the active project".into());
    }
    run_publication_verification(
        &state.store,
        &input.revision_id,
        &input.source_run_id,
        &input.comparisons,
    )
    .await?;
    publication_workspace(&state.store, &project.id, None, Some(&input.revision_id))
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_store::{ArtifactMaterialization, ArtifactVersionDraft, RunRecord, RunStatus};

    async fn fixture(name: &str) -> (std::path::PathBuf, Store) {
        let root = std::env::temp_dir().join(format!(
            "wisp_publication_commands_{name}_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        // These command tests exercise metadata, not database persistence. Keep
        // the database in memory so cleanup never removes an open SQLite file
        // on Windows; the project directory still validates path ownership.
        let store = Store::open(std::path::Path::new(":memory:")).await.unwrap();
        store
            .create_project("project", "Project", &root.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("frame", "project", "OPERON", "model")
            .await
            .unwrap();
        (root, store)
    }

    async fn draft_revision(store: &Store) -> PublicationRevision {
        create_publication(
            store,
            "project",
            &CreatePublicationInput {
                title: "Paper".into(),
                description: String::new(),
                revision_label: "Submission".into(),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn artifact_binding_resolves_latest_version_once() {
        let (root, store) = fixture("artifact").await;
        let revision = draft_revision(&store).await;
        for (version_id, checksum) in [
            ("artifact-v1", "1".repeat(64)),
            ("artifact-v2", "2".repeat(64)),
        ] {
            store
                .save_artifact_version(&ArtifactVersionDraft {
                    version_id: Some(version_id.into()),
                    artifact_id: "artifact".into(),
                    project_id: "project".into(),
                    root_frame_id: "frame".into(),
                    filename: "figure.png".into(),
                    content_type: "image/png".into(),
                    storage_path: "figure.png".into(),
                    logical_key: Some("figure".into()),
                    size_bytes: Some(3),
                    checksum: Some(checksum),
                    producing_run_id: None,
                    env_snapshot_hash: None,
                    materialization: ArtifactMaterialization::Snapshot,
                    capture_timing: ArtifactCaptureTiming::AtCreation,
                })
                .await
                .unwrap();
        }

        let binding = bind_evidence(
            &store,
            "project",
            &BindPublicationEvidenceInput {
                revision_id: revision.id.clone(),
                item_id: None,
                source_kind: WorkspaceSourceKind::Artifact,
                source_id: "artifact".into(),
                purpose: "Figure 2B".into(),
                supported_claim_item_id: None,
                selection_state: EvidenceSelectionState::Selected,
                visibility: EvidenceVisibility::Public,
            },
        )
        .await
        .unwrap();

        assert_eq!(binding.source_id, "artifact-v2");
        assert_eq!(binding.artifact_version_id.as_deref(), Some("artifact-v2"));
        let workspace = publication_workspace(&store, "project", None, Some(&revision.id))
            .await
            .unwrap();
        assert_eq!(
            workspace.lineage[0].exact_version_id.as_deref(),
            Some("artifact-v2")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn run_binding_and_workspace_are_project_scoped() {
        let (root, store) = fixture("run").await;
        let revision = draft_revision(&store).await;
        store
            .create_run(&RunRecord {
                id: "run".into(),
                project_id: "project".into(),
                frame_id: Some("frame".into()),
                context_id: "local".into(),
                title: "Analysis".into(),
                kind: "command".into(),
                status: RunStatus::Succeeded,
                command: Some("python analysis.py".into()),
                script_path: None,
                input_refs_json: "[]".into(),
                output_specs_json: "[]".into(),
                created_at: 1,
                started_at: Some(1),
                ended_at: Some(2),
                exit_code: Some(0),
                stdout_tail: None,
                stderr_tail: None,
                remote_workdir: None,
                remote_handle_json: None,
                timeout_secs: None,
                last_polled_at: None,
                last_poll_error: None,
                progress_json: "{}".into(),
                env_snapshot_json: r#"{"python":"3.12"}"#.into(),
                harvested_at: None,
                cleaned_at: None,
                cleanup_error: None,
                logs_path: None,
            })
            .await
            .unwrap();
        let binding = bind_evidence(
            &store,
            "project",
            &BindPublicationEvidenceInput {
                revision_id: revision.id.clone(),
                item_id: None,
                source_kind: WorkspaceSourceKind::Run,
                source_id: "run".into(),
                purpose: "Methods".into(),
                supported_claim_item_id: None,
                selection_state: EvidenceSelectionState::Candidate,
                visibility: EvidenceVisibility::Restricted,
            },
        )
        .await
        .unwrap();
        assert_eq!(binding.run_id.as_deref(), Some("run"));

        assert!(
            publication_workspace(&store, "another-project", None, Some(&revision.id))
                .await
                .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
