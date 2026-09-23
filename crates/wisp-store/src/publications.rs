use super::{
    artifact_node_id, canonical_json, canonical_json_sha256, run_node_id, ArtifactMaterialization,
    CapsuleBuild, EvidenceBinding, EvidenceBindingDraft, EvidenceReproductionState, EvidenceReview,
    EvidenceReviewState, EvidenceSelectionState, EvidenceSourceKind, EvidenceSupersession,
    EvidenceVisibility, Publication, PublicationCapabilityLevel, PublicationEvidenceDrift,
    PublicationFreezeCommit, PublicationFreezePolicy, PublicationItem, PublicationItemKind,
    PublicationItemLink, PublicationReadinessReport, PublicationRevision, PublicationRevisionState,
    PublicationWaiver, ReproductionResult, ReproductionRun, ReproductionRunCommit,
    ReproductionRunStart, ResearchEdge, ResearchNode, ResearchNodeKind, Store,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, Transaction};
use std::collections::HashMap;

fn publication_node_id(publication_id: &str) -> String {
    format!("publication:{publication_id}")
}

fn publication_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Publication> {
    Ok(Publication {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn publication_revision_from_row(row: sqlx::sqlite::SqliteRow) -> Result<PublicationRevision> {
    let state: String = row.try_get("state")?;
    let capability: String = row.try_get("capability_level")?;
    Ok(PublicationRevision {
        id: row.try_get("id")?,
        publication_id: row.try_get("publication_id")?,
        parent_revision_id: row.try_get("parent_revision_id")?,
        revision_number: row.try_get("revision_number")?,
        label: row.try_get("label")?,
        state: PublicationRevisionState::from_storage(&state)?,
        capability_level: PublicationCapabilityLevel::from_storage(&capability)?,
        manifest_json: row.try_get("manifest_json")?,
        manifest_sha256: row.try_get("manifest_sha256")?,
        frozen_at: row.try_get("frozen_at")?,
        published_at: row.try_get("published_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn publication_item_from_row(row: sqlx::sqlite::SqliteRow) -> Result<PublicationItem> {
    let kind: String = row.try_get("kind")?;
    Ok(PublicationItem {
        id: row.try_get("id")?,
        revision_id: row.try_get("revision_id")?,
        parent_item_id: row.try_get("parent_item_id")?,
        kind: PublicationItemKind::from_storage(&kind)?,
        title: row.try_get("title")?,
        content: row.try_get("content")?,
        ordinal: row.try_get("ordinal")?,
        metadata_json: row.try_get("metadata_json")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn publication_item_link_from_row(row: sqlx::sqlite::SqliteRow) -> Result<PublicationItemLink> {
    Ok(PublicationItemLink {
        id: row.try_get("id")?,
        revision_id: row.try_get("revision_id")?,
        source_item_id: row.try_get("source_item_id")?,
        target_item_id: row.try_get("target_item_id")?,
        relation: row.try_get("relation")?,
        created_at: row.try_get("created_at")?,
    })
}

fn capsule_build_from_row(row: sqlx::sqlite::SqliteRow) -> Result<CapsuleBuild> {
    let visibility: String = row.try_get("visibility")?;
    Ok(CapsuleBuild {
        id: row.try_get("id")?,
        revision_id: row.try_get("revision_id")?,
        format: row.try_get("format")?,
        visibility: EvidenceVisibility::from_storage(&visibility)?,
        status: row.try_get("status")?,
        output_path: row.try_get("output_path")?,
        revision_manifest_sha256: row.try_get("revision_manifest_sha256")?,
        archive_sha256: row.try_get("archive_sha256")?,
        error: row.try_get("error")?,
        created_at: row.try_get("created_at")?,
        completed_at: row.try_get("completed_at")?,
    })
}

fn reproduction_run_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ReproductionRun> {
    let capability: String = row.try_get("capability_level")?;
    Ok(ReproductionRun {
        id: row.try_get("id")?,
        revision_id: row.try_get("revision_id")?,
        source_run_id: row.try_get("source_run_id")?,
        status: row.try_get("status")?,
        capability_level: PublicationCapabilityLevel::from_storage(&capability)?,
        command_sha256: row.try_get("command_sha256")?,
        expected_environment_hash: row.try_get("expected_environment_hash")?,
        actual_environment_json: row.try_get("actual_environment_json")?,
        actual_environment_hash: row.try_get("actual_environment_hash")?,
        environment_matched: row.try_get::<i64, _>("environment_matched")? != 0,
        workspace_manifest_json: row.try_get("workspace_manifest_json")?,
        stdout_tail: row.try_get("stdout_tail")?,
        stderr_tail: row.try_get("stderr_tail")?,
        exit_code: row.try_get("exit_code")?,
        error: row.try_get("error")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        completed_at: row.try_get("completed_at")?,
    })
}

fn reproduction_result_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ReproductionResult> {
    let comparator: String = row.try_get("comparator_kind")?;
    Ok(ReproductionResult {
        id: row.try_get("id")?,
        reproduction_run_id: row.try_get("reproduction_run_id")?,
        output_id: row.try_get("output_id")?,
        output_path: row.try_get("output_path")?,
        expected_artifact_version_id: row.try_get("expected_artifact_version_id")?,
        comparator_kind: super::ReproductionComparatorKind::from_storage(&comparator)?,
        required: row.try_get::<i64, _>("required")? != 0,
        expected_json: row.try_get("expected_json")?,
        actual_json: row.try_get("actual_json")?,
        tolerance_json: row.try_get("tolerance_json")?,
        passed: row.try_get::<i64, _>("passed")? != 0,
        report_json: row.try_get("report_json")?,
        created_at: row.try_get("created_at")?,
    })
}

fn evidence_binding_from_row(row: sqlx::sqlite::SqliteRow) -> Result<EvidenceBinding> {
    let source_kind: String = row.try_get("source_kind")?;
    let selection_state: String = row.try_get("selection_state")?;
    let review_state: String = row.try_get("review_state")?;
    let reproduction_state: String = row.try_get("reproduction_state")?;
    let visibility: String = row.try_get("visibility")?;
    Ok(EvidenceBinding {
        id: row.try_get("id")?,
        revision_id: row.try_get("revision_id")?,
        item_id: row.try_get("item_id")?,
        source_kind: EvidenceSourceKind::from_storage(&source_kind)?,
        source_id: row.try_get("source_id")?,
        artifact_version_id: row.try_get("artifact_version_id")?,
        run_id: row.try_get("run_id")?,
        external_resource_id: row.try_get("external_resource_id")?,
        purpose: row.try_get("purpose")?,
        supported_claim_item_id: row.try_get("supported_claim_item_id")?,
        selection_state: EvidenceSelectionState::from_storage(&selection_state)?,
        review_state: EvidenceReviewState::from_storage(&review_state)?,
        reproduction_state: EvidenceReproductionState::from_storage(&reproduction_state)?,
        visibility: EvidenceVisibility::from_storage(&visibility)?,
        source_snapshot_json: row.try_get("source_snapshot_json")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

const PUBLICATION_REVISION_COLUMNS: &str = "id,publication_id,parent_revision_id,revision_number,\
    label,state,capability_level,manifest_json,manifest_sha256,frozen_at,published_at,created_at,\
    updated_at";
const EVIDENCE_BINDING_COLUMNS: &str = "id,revision_id,item_id,source_kind,source_id,\
    artifact_version_id,run_id,external_resource_id,purpose,supported_claim_item_id,\
    selection_state,review_state,reproduction_state,visibility,source_snapshot_json,created_at,\
    updated_at";

async fn draft_revision_project(
    tx: &mut Transaction<'_, Sqlite>,
    revision_id: &str,
) -> Result<String> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT publication.project_id,revision.state \
         FROM publication_revisions revision \
         JOIN publications publication ON publication.id=revision.publication_id \
         WHERE revision.id=?",
    )
    .bind(revision_id)
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        Some((project_id, state)) if state == "draft" => Ok(project_id),
        Some(_) => anyhow::bail!("Publication revision is immutable"),
        None => anyhow::bail!("Publication revision not found"),
    }
}

struct ResolvedEvidenceSource {
    artifact_version_id: Option<String>,
    run_id: Option<String>,
    external_resource_id: Option<String>,
    snapshot_json: String,
    target_node_id: String,
    target_kind: ResearchNodeKind,
    target_title: String,
}

const MAX_INLINE_EVIDENCE_BYTES: usize = 1024 * 1024;
const MAX_MESSAGE_SPAN_BYTES: usize = 64 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageSpanLocator {
    frame_id: String,
    message_seq: i64,
    byte_start: usize,
    byte_end: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message_content_sha256: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCallLocator {
    frame_id: String,
    message_seq: i64,
    tool_call_id: String,
}

fn parse_canonical_locator<T>(source_id: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let locator: T = serde_json::from_str(source_id)
        .map_err(|_| anyhow::anyhow!("Fine-grained evidence locator is invalid"))?;
    if canonical_json(&serde_json::to_value(&locator)?) != source_id {
        anyhow::bail!("Fine-grained evidence locator must use canonical JSON");
    }
    Ok(locator)
}

fn anchor_snapshot(anchor: serde_json::Value) -> String {
    let (_, anchor_sha256) = canonical_json_sha256(&anchor);
    canonical_json(&serde_json::json!({
        "anchor": anchor,
        "anchor_sha256": anchor_sha256,
    }))
}

fn source_node_id(kind: EvidenceSourceKind, source_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(kind.as_str().as_bytes());
    digest.update([0]);
    digest.update(source_id.as_bytes());
    format!(
        "publication-source:{}:{}",
        kind.as_str(),
        hex::encode(digest.finalize())
    )
}

fn checked_inline_evidence(label: &str, values: &[&str]) -> Result<()> {
    let size = values
        .iter()
        .fold(0_usize, |size, value| size.saturating_add(value.len()));
    if size > MAX_INLINE_EVIDENCE_BYTES {
        anyhow::bail!("{label} is too large for inline evidence; capture it as an ArtifactVersion");
    }
    Ok(())
}

async fn resolve_evidence_source(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    kind: EvidenceSourceKind,
    source_id: &str,
) -> Result<ResolvedEvidenceSource> {
    match kind {
        EvidenceSourceKind::ArtifactVersion => {
            let row = sqlx::query(
                "SELECT version.id,version.artifact_id,version.version_number,\
                        version.content_type,version.size_bytes,version.checksum,\
                        version.producing_run_id,version.env_snapshot_hash,\
                        version.materialization,version.capture_timing,\
                        artifact.filename,artifact.logical_key \
                 FROM artifact_versions version \
                 JOIN artifacts artifact ON artifact.id=version.artifact_id \
                 WHERE version.id=? AND artifact.project_id=?",
            )
            .bind(source_id)
            .bind(project_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("ArtifactVersion evidence must belong to the Publication project")
            })?;
            let artifact_id: String = row.try_get("artifact_id")?;
            let filename: String = row.try_get("filename")?;
            let snapshot = serde_json::json!({
                "source_kind": "artifact_version",
                "source_id": source_id,
                "artifact_id": artifact_id,
                "version_number": row.try_get::<i64, _>("version_number")?,
                "filename": filename,
                "content_type": row.try_get::<String, _>("content_type")?,
                "size_bytes": row.try_get::<Option<i64>, _>("size_bytes")?,
                "checksum": row.try_get::<Option<String>, _>("checksum")?,
                "producing_run_id": row.try_get::<Option<String>, _>("producing_run_id")?,
                "env_snapshot_hash": row.try_get::<Option<String>, _>("env_snapshot_hash")?,
                "materialization": row.try_get::<String, _>("materialization")?,
                "capture_timing": row.try_get::<String, _>("capture_timing")?,
                "logical_key": row.try_get::<Option<String>, _>("logical_key")?,
            });
            Ok(ResolvedEvidenceSource {
                artifact_version_id: Some(source_id.to_string()),
                run_id: None,
                external_resource_id: None,
                snapshot_json: canonical_json(&snapshot),
                target_node_id: artifact_node_id(&artifact_id),
                target_kind: ResearchNodeKind::Artifact,
                target_title: filename,
            })
        }
        EvidenceSourceKind::Run => {
            let row = sqlx::query(
                "SELECT id,title,kind,status,context_id,command,created_at \
                 FROM runs WHERE id=? AND project_id=?",
            )
            .bind(source_id)
            .bind(project_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("Run evidence must belong to the Publication project")
            })?;
            let title: String = row.try_get("title")?;
            let command: Option<String> = row.try_get("command")?;
            let command_sha256 = command.map(|command| {
                let mut digest = Sha256::new();
                digest.update(command.as_bytes());
                hex::encode(digest.finalize())
            });
            let snapshot = serde_json::json!({
                "source_kind": "run",
                "source_id": source_id,
                "title": title,
                "kind": row.try_get::<String, _>("kind")?,
                "status": row.try_get::<String, _>("status")?,
                "context_id": row.try_get::<String, _>("context_id")?,
                "command_sha256": command_sha256,
                "created_at": row.try_get::<i64, _>("created_at")?,
            });
            Ok(ResolvedEvidenceSource {
                artifact_version_id: None,
                run_id: Some(source_id.to_string()),
                external_resource_id: None,
                snapshot_json: canonical_json(&snapshot),
                target_node_id: run_node_id(source_id),
                target_kind: ResearchNodeKind::Run,
                target_title: title,
            })
        }
        EvidenceSourceKind::ExecutionLog | EvidenceSourceKind::CodeCell => {
            let row = sqlx::query(
                "SELECT execution.id,execution.frame_id,execution.cell_index,execution.tool,\
                        execution.language,execution.source,execution.stdout,execution.stderr,\
                        execution.exit_status,execution.wall_s,execution.files_written,\
                        execution.files_read,execution.env_hash,execution.created_at \
                 FROM execution_log execution \
                 JOIN frames frame ON frame.id=execution.frame_id \
                 WHERE execution.id=? AND frame.project_id=?",
            )
            .bind(source_id)
            .bind(project_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} evidence must belong to the Publication project",
                    kind.as_str()
                )
            })?;
            let source: String = row.try_get("source")?;
            let stdout: String = row
                .try_get::<Option<String>, _>("stdout")?
                .unwrap_or_default();
            let stderr: String = row
                .try_get::<Option<String>, _>("stderr")?
                .unwrap_or_default();
            checked_inline_evidence(kind.as_str(), &[&source, &stdout, &stderr])?;
            let frame_id: String = row.try_get("frame_id")?;
            let cell_index: i64 = row.try_get("cell_index")?;
            let language: String = row.try_get("language")?;
            let anchor = if kind == EvidenceSourceKind::CodeCell {
                serde_json::json!({
                    "cell_index": cell_index,
                    "execution_log_id": source_id,
                    "frame_id": frame_id,
                    "language": language,
                    "source": source,
                    "source_sha256": hex::encode(Sha256::digest(source.as_bytes())),
                    "source_kind": kind.as_str(),
                })
            } else {
                serde_json::json!({
                    "cell_index": cell_index,
                    "created_at": row.try_get::<i64, _>("created_at")?,
                    "env_hash": row.try_get::<Option<String>, _>("env_hash")?,
                    "execution_log_id": source_id,
                    "exit_status": row.try_get::<String, _>("exit_status")?,
                    "files_read": serde_json::from_str::<serde_json::Value>(
                        &row.try_get::<String, _>("files_read")?,
                    ).unwrap_or_else(|_| serde_json::json!([])),
                    "files_written": serde_json::from_str::<serde_json::Value>(
                        &row.try_get::<String, _>("files_written")?,
                    ).unwrap_or_else(|_| serde_json::json!([])),
                    "frame_id": frame_id,
                    "language": language,
                    "source": source,
                    "source_sha256": hex::encode(Sha256::digest(source.as_bytes())),
                    "source_kind": kind.as_str(),
                    "stderr": stderr,
                    "stderr_sha256": hex::encode(Sha256::digest(stderr.as_bytes())),
                    "stdout": stdout,
                    "stdout_sha256": hex::encode(Sha256::digest(stdout.as_bytes())),
                    "tool": row.try_get::<String, _>("tool")?,
                    "wall_s": row.try_get::<Option<f64>, _>("wall_s")?,
                })
            };
            Ok(ResolvedEvidenceSource {
                artifact_version_id: None,
                run_id: None,
                external_resource_id: None,
                snapshot_json: anchor_snapshot(anchor),
                target_node_id: source_node_id(kind, source_id),
                target_kind: ResearchNodeKind::Run,
                target_title: format!(
                    "{} cell {}",
                    if language.is_empty() {
                        "Execution"
                    } else {
                        &language
                    },
                    cell_index
                ),
            })
        }
        EvidenceSourceKind::MessageSpan => {
            let locator: MessageSpanLocator = parse_canonical_locator(source_id)?;
            if locator.frame_id.trim().is_empty()
                || locator.message_seq < 1
                || locator.byte_start >= locator.byte_end
                || locator.byte_end - locator.byte_start > MAX_MESSAGE_SPAN_BYTES
            {
                anyhow::bail!("MessageSpan locator has an invalid UTF-8 byte range");
            }
            let row = sqlx::query(
                "SELECT message.role,message.content \
                 FROM messages message \
                 JOIN frames frame ON frame.id=message.frame_id \
                 WHERE message.frame_id=? AND message.seq=? AND frame.project_id=?",
            )
            .bind(&locator.frame_id)
            .bind(locator.message_seq)
            .bind(project_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("MessageSpan evidence must belong to the Publication project")
            })?;
            let content_json: String = row.try_get("content")?;
            let content = serde_json::from_str::<wisp_llm::Content>(&content_json)
                .map_err(|_| anyhow::anyhow!("Message content is invalid"))?
                .as_text();
            if locator
                .message_content_sha256
                .as_ref()
                .is_some_and(|expected| {
                    expected != &hex::encode(Sha256::digest(content.as_bytes()))
                })
            {
                anyhow::bail!("Message changed after preview; select the passage again");
            }
            if locator.byte_end > content.len()
                || !content.is_char_boundary(locator.byte_start)
                || !content.is_char_boundary(locator.byte_end)
            {
                anyhow::bail!("MessageSpan locator is not a valid UTF-8 byte range");
            }
            let text = &content[locator.byte_start..locator.byte_end];
            let anchor = serde_json::json!({
                "byte_end": locator.byte_end,
                "byte_start": locator.byte_start,
                "frame_id": locator.frame_id,
                "message_content_sha256": hex::encode(Sha256::digest(content.as_bytes())),
                "message_seq": locator.message_seq,
                "role": row.try_get::<String, _>("role")?,
                "source_kind": kind.as_str(),
                "text_snapshot": text,
                "text_snapshot_sha256": hex::encode(Sha256::digest(text.as_bytes())),
            });
            Ok(ResolvedEvidenceSource {
                artifact_version_id: None,
                run_id: None,
                external_resource_id: None,
                snapshot_json: anchor_snapshot(anchor),
                target_node_id: source_node_id(kind, source_id),
                target_kind: ResearchNodeKind::Artifact,
                target_title: format!("Message {} excerpt", locator.message_seq),
            })
        }
        EvidenceSourceKind::ToolCall => {
            let locator: ToolCallLocator = parse_canonical_locator(source_id)?;
            if locator.frame_id.trim().is_empty()
                || locator.message_seq < 1
                || locator.tool_call_id.trim().is_empty()
            {
                anyhow::bail!("ToolCall locator is invalid");
            }
            let row = sqlx::query(
                "SELECT message.tool_calls \
                 FROM messages message \
                 JOIN frames frame ON frame.id=message.frame_id \
                 WHERE message.frame_id=? AND message.seq=? AND frame.project_id=?",
            )
            .bind(&locator.frame_id)
            .bind(locator.message_seq)
            .bind(project_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("ToolCall evidence must belong to the Publication project")
            })?;
            let calls_json: Option<String> = row.try_get("tool_calls")?;
            let calls = calls_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<Vec<wisp_llm::ToolCall>>(value).ok())
                .unwrap_or_default();
            let call = calls
                .iter()
                .find(|call| call.id == locator.tool_call_id)
                .ok_or_else(|| anyhow::anyhow!("ToolCall was not found at the exact message"))?;
            let result = sqlx::query(
                "SELECT seq,content,tool_name FROM messages \
                 WHERE frame_id=? AND seq>? AND tool_call_id=? ORDER BY seq LIMIT 1",
            )
            .bind(&locator.frame_id)
            .bind(locator.message_seq)
            .bind(&locator.tool_call_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| anyhow::anyhow!("ToolCall has no exact persisted result"))?;
            let result_json: String = result.try_get("content")?;
            let result_text = serde_json::from_str::<wisp_llm::Content>(&result_json)
                .map_err(|_| anyhow::anyhow!("ToolCall result content is invalid"))?
                .as_text();
            checked_inline_evidence("ToolCall", &[&call.function.arguments, &result_text])?;
            let anchor = serde_json::json!({
                "arguments": call.function.arguments,
                "arguments_sha256": hex::encode(Sha256::digest(call.function.arguments.as_bytes())),
                "frame_id": locator.frame_id,
                "message_seq": locator.message_seq,
                "name": call.function.name,
                "result": result_text,
                "result_message_seq": result.try_get::<i64, _>("seq")?,
                "result_sha256": hex::encode(Sha256::digest(result_text.as_bytes())),
                "source_kind": kind.as_str(),
                "tool_call_id": locator.tool_call_id,
                "tool_name": result.try_get::<Option<String>, _>("tool_name")?,
            });
            Ok(ResolvedEvidenceSource {
                artifact_version_id: None,
                run_id: None,
                external_resource_id: None,
                snapshot_json: anchor_snapshot(anchor),
                target_node_id: source_node_id(kind, source_id),
                target_kind: ResearchNodeKind::Artifact,
                target_title: format!("Tool result: {}", call.function.name),
            })
        }
        EvidenceSourceKind::ExternalResource => {
            let row = sqlx::query(
                "SELECT id,kind,uri,version,checksum,size_bytes,license,visibility,\
                        access_instructions,accessed_at,created_at,updated_at \
                 FROM external_resources WHERE id=? AND project_id=?",
            )
            .bind(source_id)
            .bind(project_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("ExternalResource evidence must belong to the Publication project")
            })?;
            let uri: String = row.try_get("uri")?;
            let resource_kind: String = row.try_get("kind")?;
            let anchor = serde_json::json!({
                "access_instructions": row.try_get::<Option<String>, _>("access_instructions")?,
                "accessed_at": row.try_get::<Option<i64>, _>("accessed_at")?,
                "checksum": row.try_get::<Option<String>, _>("checksum")?,
                "created_at": row.try_get::<i64, _>("created_at")?,
                "kind": resource_kind,
                "license": row.try_get::<Option<String>, _>("license")?,
                "size_bytes": row.try_get::<Option<i64>, _>("size_bytes")?,
                "source_id": source_id,
                "source_kind": kind.as_str(),
                "updated_at": row.try_get::<i64, _>("updated_at")?,
                "uri": uri,
                "version": row.try_get::<Option<String>, _>("version")?,
                "visibility": row.try_get::<String, _>("visibility")?,
            });
            Ok(ResolvedEvidenceSource {
                artifact_version_id: None,
                run_id: None,
                external_resource_id: Some(source_id.to_string()),
                snapshot_json: anchor_snapshot(anchor),
                target_node_id: source_node_id(kind, source_id),
                target_kind: ResearchNodeKind::DataAsset,
                target_title: format!("{}: {}", resource_kind, uri),
            })
        }
    }
}

async fn delete_revision_children(
    tx: &mut Transaction<'_, Sqlite>,
    revision_id: &str,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM research_edges WHERE id IN (\
           SELECT 'publication-evidence:' || id FROM evidence_bindings WHERE revision_id=?\
         )",
    )
    .bind(revision_id)
    .execute(&mut **tx)
    .await?;
    for statement in [
        "DELETE FROM publication_freeze_attempts WHERE revision_id=?",
        "DELETE FROM reproduction_results WHERE reproduction_run_id IN \
           (SELECT id FROM reproduction_runs WHERE revision_id=?)",
        "DELETE FROM reproduction_runs WHERE revision_id=?",
        "DELETE FROM capsule_builds WHERE revision_id=?",
        "DELETE FROM publication_readiness_reports WHERE revision_id=?",
        "DELETE FROM publication_waivers WHERE revision_id=?",
        "DELETE FROM evidence_reviews WHERE binding_id IN \
           (SELECT id FROM evidence_bindings WHERE revision_id=?)",
        "DELETE FROM evidence_supersessions WHERE revision_id=?",
        "DELETE FROM evidence_bindings WHERE revision_id=?",
        "DELETE FROM publication_item_links WHERE revision_id=?",
        "DELETE FROM publication_items WHERE revision_id=?",
    ] {
        sqlx::query(statement)
            .bind(revision_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

impl Store {
    pub async fn create_publication(
        &self,
        id: &str,
        project_id: &str,
        title: &str,
        description: &str,
    ) -> Result<Publication> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.create_publication(id, project_id, title, description)).await;
        }
        if id.trim().is_empty() || project_id.trim().is_empty() || title.trim().is_empty() {
            anyhow::bail!("Publication requires identity, project, and title");
        }
        let now = chrono::Utc::now().timestamp();
        let inserted = sqlx::query(
            "INSERT INTO publications(id,project_id,title,description,created_at,updated_at) \
             SELECT ?,id,?,?,?,? FROM projects WHERE id=?",
        )
        .bind(id)
        .bind(title.trim())
        .bind(description)
        .bind(now)
        .bind(now)
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        if inserted.rows_affected() != 1 {
            anyhow::bail!("Publication project not found");
        }

        let mut node = ResearchNode::new(
            publication_node_id(id),
            project_id,
            ResearchNodeKind::Paper,
            title.trim(),
        )?;
        node.ref_id = Some(id.to_string());
        node.metadata_json = r#"{"projection":"publication"}"#.into();
        self.save_research_node(&node).await?;
        self.get_publication(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Publication was not persisted"))
    }

    pub async fn get_publication(&self, id: &str) -> Result<Option<Publication>> {
        if let Some(store) = self.route_entity("publications", "id", id).await? {
            return Box::pin(store.get_publication(id)).await;
        }
        let row = sqlx::query(
            "SELECT id,project_id,title,description,created_at,updated_at \
             FROM publications WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(publication_from_row).transpose()
    }

    pub async fn list_publications(&self, project_id: &str) -> Result<Vec<Publication>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_publications(project_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,project_id,title,description,created_at,updated_at \
             FROM publications WHERE project_id=? ORDER BY updated_at DESC,id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(publication_from_row).collect()
    }

    pub async fn update_publication(&self, id: &str, title: &str, description: &str) -> Result<()> {
        if let Some(store) = self.route_entity("publications", "id", id).await? {
            return Box::pin(store.update_publication(id, title, description)).await;
        }
        if title.trim().is_empty() {
            anyhow::bail!("Publication title cannot be empty");
        }
        let updated =
            sqlx::query("UPDATE publications SET title=?,description=?,updated_at=? WHERE id=?")
                .bind(title.trim())
                .bind(description)
                .bind(chrono::Utc::now().timestamp())
                .bind(id)
                .execute(&self.pool)
                .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Publication not found");
        }
        if let Some(publication) = self.get_publication(id).await? {
            let mut node = ResearchNode::new(
                publication_node_id(id),
                &publication.project_id,
                ResearchNodeKind::Paper,
                &publication.title,
            )?;
            node.ref_id = Some(id.to_string());
            node.metadata_json = r#"{"projection":"publication"}"#.into();
            self.save_research_node(&node).await?;
        }
        Ok(())
    }

    pub async fn delete_publication(&self, id: &str) -> Result<()> {
        if let Some(store) = self.route_entity("publications", "id", id).await? {
            return Box::pin(store.delete_publication(id)).await;
        }
        let mut tx = self.begin_write().await?;
        let publication: Option<String> =
            sqlx::query_scalar("SELECT project_id FROM publications WHERE id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        if publication.is_none() {
            anyhow::bail!("Publication not found");
        }
        let revisions = sqlx::query(
            "SELECT id,state FROM publication_revisions \
             WHERE publication_id=? ORDER BY revision_number DESC",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        for row in &revisions {
            let state: String = row.try_get("state")?;
            if state != "draft" {
                anyhow::bail!("Publication with immutable revisions cannot be deleted");
            }
        }
        for row in revisions {
            let revision_id: String = row.try_get("id")?;
            delete_revision_children(&mut tx, &revision_id).await?;
            sqlx::query("DELETE FROM publication_revisions WHERE id=?")
                .bind(revision_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM research_edges WHERE source_id=? OR target_id=?")
            .bind(publication_node_id(id))
            .bind(publication_node_id(id))
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM research_nodes WHERE id=?")
            .bind(publication_node_id(id))
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM publications WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn create_publication_revision(
        &self,
        id: &str,
        publication_id: &str,
        parent_revision_id: Option<&str>,
        label: &str,
    ) -> Result<PublicationRevision> {
        if let Some(store) = self
            .route_entity("publications", "id", publication_id)
            .await?
        {
            return Box::pin(store.create_publication_revision(
                id,
                publication_id,
                parent_revision_id,
                label,
            ))
            .await;
        }
        if id.trim().is_empty() || publication_id.trim().is_empty() || label.trim().is_empty() {
            anyhow::bail!("Publication revision requires identity, Publication, and label");
        }
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let publication_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM publications WHERE id=?)")
                .bind(publication_id)
                .fetch_one(&mut *tx)
                .await?;
        if !publication_exists {
            anyhow::bail!("Publication not found");
        }
        if let Some(parent_id) = parent_revision_id {
            let parent_publication: Option<String> =
                sqlx::query_scalar("SELECT publication_id FROM publication_revisions WHERE id=?")
                    .bind(parent_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if parent_publication.as_deref() != Some(publication_id) {
                anyhow::bail!("Parent revision must belong to the same Publication");
            }
        }
        let revision_number: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(revision_number),0)+1 FROM publication_revisions \
             WHERE publication_id=?",
        )
        .bind(publication_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO publication_revisions(\
               id,publication_id,parent_revision_id,revision_number,label,state,capability_level,\
               manifest_json,manifest_sha256,frozen_at,published_at,created_at,updated_at\
             ) VALUES(?,?,?,?,?,'draft','archived',NULL,NULL,NULL,NULL,?,?)",
        )
        .bind(id)
        .bind(publication_id)
        .bind(parent_revision_id)
        .bind(revision_number)
        .bind(label.trim())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_publication_revision(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Publication revision was not persisted"))
    }

    pub async fn get_publication_revision(&self, id: &str) -> Result<Option<PublicationRevision>> {
        if let Some(store) = self.route_entity("publication_revisions", "id", id).await? {
            return Box::pin(store.get_publication_revision(id)).await;
        }
        let row = sqlx::query(&format!(
            "SELECT {PUBLICATION_REVISION_COLUMNS} FROM publication_revisions WHERE id=?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(publication_revision_from_row).transpose()
    }

    pub async fn list_publication_revisions(
        &self,
        publication_id: &str,
    ) -> Result<Vec<PublicationRevision>> {
        if let Some(store) = self
            .route_entity("publications", "id", publication_id)
            .await?
        {
            return Box::pin(store.list_publication_revisions(publication_id)).await;
        }
        let rows = sqlx::query(&format!(
            "SELECT {PUBLICATION_REVISION_COLUMNS} FROM publication_revisions \
             WHERE publication_id=? ORDER BY revision_number DESC"
        ))
        .bind(publication_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(publication_revision_from_row)
            .collect()
    }

    pub async fn update_draft_publication_revision(&self, id: &str, label: &str) -> Result<()> {
        if let Some(store) = self.route_entity("publication_revisions", "id", id).await? {
            return Box::pin(store.update_draft_publication_revision(id, label)).await;
        }
        if label.trim().is_empty() {
            anyhow::bail!("Publication revision label cannot be empty");
        }
        let updated = sqlx::query(
            "UPDATE publication_revisions SET label=?,updated_at=? \
             WHERE id=? AND state='draft'",
        )
        .bind(label.trim())
        .bind(chrono::Utc::now().timestamp())
        .bind(id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Draft Publication revision not found");
        }
        Ok(())
    }

    pub async fn delete_draft_publication_revision(&self, id: &str) -> Result<()> {
        if let Some(store) = self.route_entity("publication_revisions", "id", id).await? {
            return Box::pin(store.delete_draft_publication_revision(id)).await;
        }
        let mut tx = self.begin_write().await?;
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM publication_revisions WHERE id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        match state.as_deref() {
            Some("draft") => {}
            Some(_) => anyhow::bail!("Publication revision is immutable"),
            None => anyhow::bail!("Publication revision not found"),
        }
        let has_children: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM publication_revisions WHERE parent_revision_id=?)",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if has_children {
            anyhow::bail!("Publication revision with descendants cannot be deleted");
        }
        delete_revision_children(&mut tx, id).await?;
        sqlx::query("DELETE FROM publication_revisions WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn save_publication_item(&self, item: &PublicationItem) -> Result<()> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", &item.revision_id)
            .await?
        {
            return Box::pin(store.save_publication_item(item)).await;
        }
        if item.id.trim().is_empty()
            || item.revision_id.trim().is_empty()
            || item.title.trim().is_empty()
            || item.ordinal < 0
        {
            anyhow::bail!("Publication item requires identity, revision, title, and ordinal");
        }
        serde_json::from_str::<serde_json::Value>(&item.metadata_json)
            .map_err(|_| anyhow::anyhow!("Publication item metadata must be valid JSON"))?;
        if item.parent_item_id.as_deref() == Some(item.id.as_str()) {
            anyhow::bail!("Publication item cannot parent itself");
        }

        let mut tx = self.begin_write().await?;
        draft_revision_project(&mut tx, &item.revision_id).await?;
        let existing_revision: Option<String> =
            sqlx::query_scalar("SELECT revision_id FROM publication_items WHERE id=?")
                .bind(&item.id)
                .fetch_optional(&mut *tx)
                .await?;
        if existing_revision
            .as_deref()
            .is_some_and(|revision_id| revision_id != item.revision_id)
        {
            anyhow::bail!("Publication item cannot move between revisions");
        }
        if let Some(parent_id) = item.parent_item_id.as_deref() {
            let parent_revision: Option<String> =
                sqlx::query_scalar("SELECT revision_id FROM publication_items WHERE id=?")
                    .bind(parent_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if parent_revision.as_deref() != Some(item.revision_id.as_str()) {
                anyhow::bail!("Publication item parent must belong to the revision");
            }
            let cycle: bool = sqlx::query_scalar(
                "WITH RECURSIVE ancestors(id) AS (\
                   SELECT ? \
                   UNION \
                   SELECT item.parent_item_id FROM publication_items item \
                   JOIN ancestors parent ON item.id=parent.id \
                   WHERE item.parent_item_id IS NOT NULL\
                 ) SELECT EXISTS(SELECT 1 FROM ancestors WHERE id=?)",
            )
            .bind(parent_id)
            .bind(&item.id)
            .fetch_one(&mut *tx)
            .await?;
            if cycle {
                anyhow::bail!("Publication item hierarchy cannot contain a cycle");
            }
        }
        let now = chrono::Utc::now().timestamp();
        let created_at = if item.created_at == 0 {
            now
        } else {
            item.created_at
        };
        sqlx::query(
            "INSERT INTO publication_items(\
               id,revision_id,parent_item_id,kind,title,content,ordinal,metadata_json,\
               created_at,updated_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
               parent_item_id=excluded.parent_item_id,kind=excluded.kind,title=excluded.title,\
               content=excluded.content,ordinal=excluded.ordinal,\
               metadata_json=excluded.metadata_json,updated_at=excluded.updated_at",
        )
        .bind(&item.id)
        .bind(&item.revision_id)
        .bind(item.parent_item_id.as_deref())
        .bind(item.kind.as_str())
        .bind(item.title.trim())
        .bind(&item.content)
        .bind(item.ordinal)
        .bind(canonical_json(
            &serde_json::from_str(&item.metadata_json).expect("validated JSON"),
        ))
        .bind(created_at)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_publication_items(&self, revision_id: &str) -> Result<Vec<PublicationItem>> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.list_publication_items(revision_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,revision_id,parent_item_id,kind,title,content,ordinal,metadata_json,\
                    created_at,updated_at \
             FROM publication_items WHERE revision_id=? \
             ORDER BY COALESCE(parent_item_id,''),ordinal,id",
        )
        .bind(revision_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(publication_item_from_row).collect()
    }

    pub async fn delete_publication_item(&self, id: &str) -> Result<()> {
        if let Some(store) = self.route_entity("publication_items", "id", id).await? {
            return Box::pin(store.delete_publication_item(id)).await;
        }
        let mut tx = self.begin_write().await?;
        let revision_id: Option<String> =
            sqlx::query_scalar("SELECT revision_id FROM publication_items WHERE id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let revision_id =
            revision_id.ok_or_else(|| anyhow::anyhow!("Publication item not found"))?;
        draft_revision_project(&mut tx, &revision_id).await?;
        sqlx::query(
            "WITH RECURSIVE descendants(id) AS (\
               SELECT id FROM publication_items WHERE id=? \
               UNION ALL \
               SELECT child.id FROM publication_items child \
               JOIN descendants parent ON child.parent_item_id=parent.id\
             ) \
             DELETE FROM research_edges WHERE id IN (\
               SELECT 'publication-evidence:' || binding.id \
               FROM evidence_bindings binding \
               WHERE binding.item_id IN (SELECT id FROM descendants)\
             )",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM publication_items WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn save_publication_item_link(&self, link: &PublicationItemLink) -> Result<()> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", &link.revision_id)
            .await?
        {
            return Box::pin(store.save_publication_item_link(link)).await;
        }
        if link.id.trim().is_empty()
            || link.revision_id.trim().is_empty()
            || link.source_item_id.trim().is_empty()
            || link.target_item_id.trim().is_empty()
            || link.relation.trim().is_empty()
            || link.source_item_id == link.target_item_id
        {
            anyhow::bail!("Publication item link requires distinct items and a relation");
        }
        let mut tx = self.begin_write().await?;
        draft_revision_project(&mut tx, &link.revision_id).await?;
        let endpoint_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM publication_items \
             WHERE revision_id=? AND id IN (?,?)",
        )
        .bind(&link.revision_id)
        .bind(&link.source_item_id)
        .bind(&link.target_item_id)
        .fetch_one(&mut *tx)
        .await?;
        if endpoint_count != 2 {
            anyhow::bail!("Publication item link must stay inside one revision");
        }
        let existing_revision: Option<String> =
            sqlx::query_scalar("SELECT revision_id FROM publication_item_links WHERE id=?")
                .bind(&link.id)
                .fetch_optional(&mut *tx)
                .await?;
        if existing_revision
            .as_deref()
            .is_some_and(|revision_id| revision_id != link.revision_id)
        {
            anyhow::bail!("Publication item link cannot move between revisions");
        }
        sqlx::query(
            "INSERT INTO publication_item_links(\
               id,revision_id,source_item_id,target_item_id,relation,created_at\
             ) VALUES(?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
               source_item_id=excluded.source_item_id,target_item_id=excluded.target_item_id,\
               relation=excluded.relation",
        )
        .bind(&link.id)
        .bind(&link.revision_id)
        .bind(&link.source_item_id)
        .bind(&link.target_item_id)
        .bind(link.relation.trim())
        .bind(if link.created_at == 0 {
            chrono::Utc::now().timestamp()
        } else {
            link.created_at
        })
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_publication_item_links(
        &self,
        revision_id: &str,
    ) -> Result<Vec<PublicationItemLink>> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.list_publication_item_links(revision_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,revision_id,source_item_id,target_item_id,relation,created_at \
             FROM publication_item_links WHERE revision_id=? ORDER BY created_at,id",
        )
        .bind(revision_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(publication_item_link_from_row)
            .collect()
    }

    pub async fn delete_publication_item_link(&self, id: &str) -> Result<()> {
        if let Some(store) = self
            .route_entity("publication_item_links", "id", id)
            .await?
        {
            return Box::pin(store.delete_publication_item_link(id)).await;
        }
        let mut tx = self.begin_write().await?;
        let revision_id: Option<String> =
            sqlx::query_scalar("SELECT revision_id FROM publication_item_links WHERE id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let revision_id =
            revision_id.ok_or_else(|| anyhow::anyhow!("Publication item link not found"))?;
        draft_revision_project(&mut tx, &revision_id).await?;
        sqlx::query("DELETE FROM publication_item_links WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn save_evidence_binding(
        &self,
        draft: &EvidenceBindingDraft,
    ) -> Result<EvidenceBinding> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", &draft.revision_id)
            .await?
        {
            return Box::pin(store.save_evidence_binding(draft)).await;
        }
        if draft.id.trim().is_empty()
            || draft.revision_id.trim().is_empty()
            || draft.source_id.trim().is_empty()
        {
            anyhow::bail!("Evidence binding requires identity, revision, and exact source");
        }
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let project_id = draft_revision_project(&mut tx, &draft.revision_id).await?;
        let source =
            resolve_evidence_source(&mut tx, &project_id, draft.source_kind, &draft.source_id)
                .await?;
        let existing = sqlx::query(
            "SELECT revision_id,source_kind,source_id,review_state,reproduction_state,\
                    source_snapshot_json,created_at \
             FROM evidence_bindings WHERE id=?",
        )
        .bind(&draft.id)
        .fetch_optional(&mut *tx)
        .await?;
        let (review_state, reproduction_state, source_snapshot_json, created_at) =
            if let Some(row) = existing {
                let revision_id: String = row.try_get("revision_id")?;
                let source_kind: String = row.try_get("source_kind")?;
                let source_id: String = row.try_get("source_id")?;
                if revision_id != draft.revision_id
                    || source_kind != draft.source_kind.as_str()
                    || source_id != draft.source_id
                {
                    anyhow::bail!("Evidence binding exact source and revision cannot be changed");
                }
                (
                    row.try_get::<String, _>("review_state")?,
                    row.try_get::<String, _>("reproduction_state")?,
                    row.try_get::<String, _>("source_snapshot_json")?,
                    row.try_get::<i64, _>("created_at")?,
                )
            } else {
                (
                    EvidenceReviewState::Unreviewed.as_str().to_string(),
                    EvidenceReproductionState::NotRun.as_str().to_string(),
                    source.snapshot_json,
                    now,
                )
            };
        sqlx::query(
            "INSERT INTO evidence_bindings(\
               id,revision_id,item_id,source_kind,source_id,artifact_version_id,run_id,\
               external_resource_id,purpose,supported_claim_item_id,selection_state,review_state,\
               reproduction_state,visibility,source_snapshot_json,created_at,updated_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
               item_id=excluded.item_id,purpose=excluded.purpose,\
               supported_claim_item_id=excluded.supported_claim_item_id,\
               selection_state=excluded.selection_state,visibility=excluded.visibility,\
               updated_at=excluded.updated_at",
        )
        .bind(&draft.id)
        .bind(&draft.revision_id)
        .bind(draft.item_id.as_deref())
        .bind(draft.source_kind.as_str())
        .bind(&draft.source_id)
        .bind(source.artifact_version_id.as_deref())
        .bind(source.run_id.as_deref())
        .bind(source.external_resource_id.as_deref())
        .bind(draft.purpose.trim())
        .bind(draft.supported_claim_item_id.as_deref())
        .bind(draft.selection_state.as_str())
        .bind(review_state)
        .bind(reproduction_state)
        .bind(draft.visibility.as_str())
        .bind(source_snapshot_json)
        .bind(created_at)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        self.sync_evidence_projection(
            &draft.id,
            &project_id,
            &draft.revision_id,
            draft.item_id.as_deref(),
            draft.source_kind,
            &draft.source_id,
            &source.target_node_id,
            source.target_kind,
            &source.target_title,
        )
        .await?;
        self.get_evidence_binding(&draft.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Evidence binding was not persisted"))
    }

    #[allow(clippy::too_many_arguments)]
    async fn sync_evidence_projection(
        &self,
        binding_id: &str,
        project_id: &str,
        revision_id: &str,
        item_id: Option<&str>,
        source_kind: EvidenceSourceKind,
        source_id: &str,
        target_node_id: &str,
        target_kind: ResearchNodeKind,
        target_title: &str,
    ) -> Result<()> {
        let publication: (String, String) = sqlx::query_as(
            "SELECT publication.id,publication.title \
             FROM publication_revisions revision \
             JOIN publications publication ON publication.id=revision.publication_id \
             WHERE revision.id=?",
        )
        .bind(revision_id)
        .fetch_one(&self.pool)
        .await?;
        let mut publication_node = ResearchNode::new(
            publication_node_id(&publication.0),
            project_id,
            ResearchNodeKind::Paper,
            &publication.1,
        )?;
        publication_node.ref_id = Some(publication.0);
        publication_node.metadata_json = r#"{"projection":"publication"}"#.into();
        self.save_research_node(&publication_node).await?;

        let mut target = ResearchNode::new(target_node_id, project_id, target_kind, target_title)?;
        target.ref_id = Some(match source_kind {
            EvidenceSourceKind::ArtifactVersion => {
                sqlx::query_scalar("SELECT artifact_id FROM artifact_versions WHERE id=?")
                    .bind(source_id)
                    .fetch_one(&self.pool)
                    .await?
            }
            EvidenceSourceKind::Run => source_id.to_string(),
            _ => source_id.to_string(),
        });
        self.save_research_node(&target).await?;

        let mut edge = ResearchEdge::new(
            format!("publication-evidence:{binding_id}"),
            project_id,
            &publication_node.id,
            target_node_id,
            "uses_evidence",
        )?;
        edge.metadata_json = canonical_json(&serde_json::json!({
            "binding_id": binding_id,
            "revision_id": revision_id,
            "item_id": item_id,
            "source_kind": source_kind.as_str(),
            "source_id": source_id,
        }));
        self.save_research_edge(&edge).await
    }

    pub async fn get_evidence_binding(&self, id: &str) -> Result<Option<EvidenceBinding>> {
        if let Some(store) = self.route_entity("evidence_bindings", "id", id).await? {
            return Box::pin(store.get_evidence_binding(id)).await;
        }
        let row = sqlx::query(&format!(
            "SELECT {EVIDENCE_BINDING_COLUMNS} FROM evidence_bindings WHERE id=?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(evidence_binding_from_row).transpose()
    }

    pub async fn list_evidence_bindings(&self, revision_id: &str) -> Result<Vec<EvidenceBinding>> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.list_evidence_bindings(revision_id)).await;
        }
        let rows = sqlx::query(&format!(
            "SELECT {EVIDENCE_BINDING_COLUMNS} FROM evidence_bindings \
             WHERE revision_id=? ORDER BY created_at,id"
        ))
        .bind(revision_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(evidence_binding_from_row).collect()
    }

    pub async fn update_evidence_binding_selection(
        &self,
        id: &str,
        selection_state: EvidenceSelectionState,
        visibility: EvidenceVisibility,
    ) -> Result<()> {
        if let Some(store) = self.route_entity("evidence_bindings", "id", id).await? {
            return Box::pin(store.update_evidence_binding_selection(
                id,
                selection_state,
                visibility,
            ))
            .await;
        }
        let updated = sqlx::query(
            "UPDATE evidence_bindings SET \
               selection_state=?,visibility=?,updated_at=? \
             WHERE id=? AND EXISTS(\
               SELECT 1 FROM publication_revisions revision \
               WHERE revision.id=evidence_bindings.revision_id AND revision.state='draft')",
        )
        .bind(selection_state.as_str())
        .bind(visibility.as_str())
        .bind(chrono::Utc::now().timestamp())
        .bind(id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Draft evidence binding not found");
        }
        Ok(())
    }

    pub async fn delete_evidence_binding(&self, id: &str) -> Result<()> {
        if let Some(store) = self.route_entity("evidence_bindings", "id", id).await? {
            return Box::pin(store.delete_evidence_binding(id)).await;
        }
        let mut tx = self.begin_write().await?;
        let revision_id: Option<String> =
            sqlx::query_scalar("SELECT revision_id FROM evidence_bindings WHERE id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let revision_id =
            revision_id.ok_or_else(|| anyhow::anyhow!("Evidence binding not found"))?;
        draft_revision_project(&mut tx, &revision_id).await?;
        sqlx::query("DELETE FROM research_edges WHERE id=?")
            .bind(format!("publication-evidence:{id}"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM evidence_bindings WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn save_evidence_review(&self, review: &EvidenceReview) -> Result<()> {
        if let Some(store) = self
            .route_entity("evidence_bindings", "id", &review.binding_id)
            .await?
        {
            return Box::pin(store.save_evidence_review(review)).await;
        }
        if review.id.trim().is_empty()
            || review.binding_id.trim().is_empty()
            || review.reviewer.trim().is_empty()
            || review.method.trim().is_empty()
            || review.result.trim().is_empty()
        {
            anyhow::bail!("Evidence review requires identity, reviewer, method, and result");
        }
        for (label, value) in [
            ("environment", &review.environment_json),
            ("comparator", &review.comparator_json),
            ("tolerance", &review.tolerance_json),
            ("report", &review.report_json),
        ] {
            if serde_json::from_str::<serde_json::Value>(value).is_err() {
                anyhow::bail!("Evidence review {label} must be valid JSON");
            }
        }
        let mut tx = self.begin_write().await?;
        let revision_id: Option<String> =
            sqlx::query_scalar("SELECT revision_id FROM evidence_bindings WHERE id=?")
                .bind(&review.binding_id)
                .fetch_optional(&mut *tx)
                .await?;
        let revision_id =
            revision_id.ok_or_else(|| anyhow::anyhow!("Evidence binding not found"))?;
        draft_revision_project(&mut tx, &revision_id).await?;
        let existing_binding: Option<String> =
            sqlx::query_scalar("SELECT binding_id FROM evidence_reviews WHERE id=?")
                .bind(&review.id)
                .fetch_optional(&mut *tx)
                .await?;
        if existing_binding
            .as_deref()
            .is_some_and(|binding_id| binding_id != review.binding_id)
        {
            anyhow::bail!("Evidence review cannot move between bindings");
        }
        sqlx::query(
            "INSERT INTO evidence_reviews(\
               id,binding_id,reviewer,method,verified_at,environment_json,comparator_json,\
               tolerance_json,result,report_json,created_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
               reviewer=excluded.reviewer,method=excluded.method,verified_at=excluded.verified_at,\
               environment_json=excluded.environment_json,\
               comparator_json=excluded.comparator_json,\
               tolerance_json=excluded.tolerance_json,result=excluded.result,\
               report_json=excluded.report_json",
        )
        .bind(&review.id)
        .bind(&review.binding_id)
        .bind(review.reviewer.trim())
        .bind(review.method.trim())
        .bind(review.verified_at)
        .bind(canonical_json(
            &serde_json::from_str(&review.environment_json).expect("validated JSON"),
        ))
        .bind(canonical_json(
            &serde_json::from_str(&review.comparator_json).expect("validated JSON"),
        ))
        .bind(canonical_json(
            &serde_json::from_str(&review.tolerance_json).expect("validated JSON"),
        ))
        .bind(review.result.trim())
        .bind(canonical_json(
            &serde_json::from_str(&review.report_json).expect("validated JSON"),
        ))
        .bind(if review.created_at == 0 {
            chrono::Utc::now().timestamp()
        } else {
            review.created_at
        })
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE evidence_bindings SET review_state='reviewed',updated_at=? WHERE id=?")
            .bind(chrono::Utc::now().timestamp())
            .bind(&review.binding_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_evidence_reviews(&self, binding_id: &str) -> Result<Vec<EvidenceReview>> {
        if let Some(store) = self
            .route_entity("evidence_bindings", "id", binding_id)
            .await?
        {
            return Box::pin(store.list_evidence_reviews(binding_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,binding_id,reviewer,method,verified_at,environment_json,comparator_json,\
                    tolerance_json,result,report_json,created_at \
             FROM evidence_reviews WHERE binding_id=? ORDER BY verified_at,id",
        )
        .bind(binding_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(EvidenceReview {
                    id: row.try_get("id")?,
                    binding_id: row.try_get("binding_id")?,
                    reviewer: row.try_get("reviewer")?,
                    method: row.try_get("method")?,
                    verified_at: row.try_get("verified_at")?,
                    environment_json: row.try_get("environment_json")?,
                    comparator_json: row.try_get("comparator_json")?,
                    tolerance_json: row.try_get("tolerance_json")?,
                    result: row.try_get("result")?,
                    report_json: row.try_get("report_json")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn save_evidence_supersession(
        &self,
        supersession: &EvidenceSupersession,
    ) -> Result<()> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", &supersession.revision_id)
            .await?
        {
            return Box::pin(store.save_evidence_supersession(supersession)).await;
        }
        if supersession.id.trim().is_empty()
            || supersession.revision_id.trim().is_empty()
            || supersession.old_binding_id.trim().is_empty()
            || supersession.new_binding_id.trim().is_empty()
            || supersession.old_binding_id == supersession.new_binding_id
        {
            anyhow::bail!("Evidence supersession requires two distinct bindings");
        }
        let mut tx = self.begin_write().await?;
        draft_revision_project(&mut tx, &supersession.revision_id).await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM evidence_bindings \
             WHERE revision_id=? AND id IN (?,?)",
        )
        .bind(&supersession.revision_id)
        .bind(&supersession.old_binding_id)
        .bind(&supersession.new_binding_id)
        .fetch_one(&mut *tx)
        .await?;
        if count != 2 {
            anyhow::bail!("Evidence supersession must stay inside one revision");
        }
        sqlx::query(
            "INSERT INTO evidence_supersessions(\
               id,revision_id,old_binding_id,new_binding_id,reason,created_at\
             ) VALUES(?,?,?,?,?,?) \
             ON CONFLICT(revision_id,old_binding_id) DO UPDATE SET \
               new_binding_id=excluded.new_binding_id,reason=excluded.reason",
        )
        .bind(&supersession.id)
        .bind(&supersession.revision_id)
        .bind(&supersession.old_binding_id)
        .bind(&supersession.new_binding_id)
        .bind(supersession.reason.trim())
        .bind(if supersession.created_at == 0 {
            chrono::Utc::now().timestamp()
        } else {
            supersession.created_at
        })
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_evidence_supersessions(
        &self,
        revision_id: &str,
    ) -> Result<Vec<EvidenceSupersession>> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.list_evidence_supersessions(revision_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,revision_id,old_binding_id,new_binding_id,reason,created_at \
             FROM evidence_supersessions WHERE revision_id=? ORDER BY created_at,id",
        )
        .bind(revision_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(EvidenceSupersession {
                    id: row.try_get("id")?,
                    revision_id: row.try_get("revision_id")?,
                    old_binding_id: row.try_get("old_binding_id")?,
                    new_binding_id: row.try_get("new_binding_id")?,
                    reason: row.try_get("reason")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn save_publication_waiver(&self, waiver: &PublicationWaiver) -> Result<()> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", &waiver.revision_id)
            .await?
        {
            return Box::pin(store.save_publication_waiver(waiver)).await;
        }
        if waiver.id.trim().is_empty()
            || waiver.revision_id.trim().is_empty()
            || waiver.finding_code.trim().is_empty()
            || waiver.author.trim().is_empty()
            || waiver.reason.trim().is_empty()
        {
            anyhow::bail!("Publication waiver requires finding, author, and reason");
        }
        let mut tx = self.begin_write().await?;
        draft_revision_project(&mut tx, &waiver.revision_id).await?;
        sqlx::query(
            "INSERT INTO publication_waivers(\
               id,revision_id,finding_code,author,reason,created_at\
             ) VALUES(?,?,?,?,?,?) \
             ON CONFLICT(revision_id,finding_code) DO UPDATE SET \
               author=excluded.author,reason=excluded.reason,created_at=excluded.created_at",
        )
        .bind(&waiver.id)
        .bind(&waiver.revision_id)
        .bind(waiver.finding_code.trim())
        .bind(waiver.author.trim())
        .bind(waiver.reason.trim())
        .bind(if waiver.created_at == 0 {
            chrono::Utc::now().timestamp()
        } else {
            waiver.created_at
        })
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_publication_waivers(
        &self,
        revision_id: &str,
    ) -> Result<Vec<PublicationWaiver>> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.list_publication_waivers(revision_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,revision_id,finding_code,author,reason,created_at \
             FROM publication_waivers WHERE revision_id=? ORDER BY finding_code,id",
        )
        .bind(revision_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PublicationWaiver {
                    id: row.try_get("id")?,
                    revision_id: row.try_get("revision_id")?,
                    finding_code: row.try_get("finding_code")?,
                    author: row.try_get("author")?,
                    reason: row.try_get("reason")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn begin_publication_freeze(
        &self,
        revision_id: &str,
        attempt_id: &str,
        policy: &PublicationFreezePolicy,
    ) -> Result<()> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.begin_publication_freeze(revision_id, attempt_id, policy)).await;
        }
        if revision_id.trim().is_empty() || attempt_id.trim().is_empty() {
            anyhow::bail!("Publication freeze requires revision and attempt identities");
        }
        let policy_json = canonical_json(&serde_json::to_value(policy)?);
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let updated = sqlx::query(
            "UPDATE publication_revisions SET state='freezing',updated_at=? \
             WHERE id=? AND state='draft'",
        )
        .bind(now)
        .bind(revision_id)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Draft Publication revision not found or already being frozen");
        }
        sqlx::query(
            "INSERT INTO publication_freeze_attempts(\
               id,revision_id,target_visibility,policy_json,started_at\
             ) VALUES(?,?,?,?,?)",
        )
        .bind(attempt_id)
        .bind(revision_id)
        .bind(policy.target_visibility.as_str())
        .bind(policy_json)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn abort_publication_freeze(
        &self,
        revision_id: &str,
        attempt_id: &str,
    ) -> Result<bool> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.abort_publication_freeze(revision_id, attempt_id)).await;
        }
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let removed =
            sqlx::query("DELETE FROM publication_freeze_attempts WHERE id=? AND revision_id=?")
                .bind(attempt_id)
                .bind(revision_id)
                .execute(&mut *tx)
                .await?;
        if removed.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(false);
        }
        let updated = sqlx::query(
            "UPDATE publication_revisions SET state='draft',updated_at=? \
             WHERE id=? AND state='freezing'",
        )
        .bind(now)
        .bind(revision_id)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Publication freeze state changed before abort");
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn recover_stale_publication_freezes(
        &self,
        started_before: i64,
    ) -> Result<Vec<String>> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = Vec::new();
            for store in stores {
                let value =
                    Box::pin(store.recover_stale_publication_freezes(started_before)).await?;
                result.extend(value);
            }
            return Ok(result);
        }
        let mut tx = self.begin_write().await?;
        let revisions: Vec<String> = sqlx::query_scalar(
            "SELECT revision_id FROM publication_freeze_attempts \
             WHERE started_at<=? ORDER BY revision_id",
        )
        .bind(started_before)
        .fetch_all(&mut *tx)
        .await?;
        for revision_id in &revisions {
            sqlx::query("DELETE FROM publication_freeze_attempts WHERE revision_id=?")
                .bind(revision_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE publication_revisions SET state='draft',updated_at=? \
                 WHERE id=? AND state='freezing'",
            )
            .bind(chrono::Utc::now().timestamp())
            .bind(revision_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(revisions)
    }

    pub async fn commit_publication_freeze(
        &self,
        commit: &PublicationFreezeCommit,
    ) -> Result<PublicationRevision> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", &commit.revision_id)
            .await?
        {
            return Box::pin(store.commit_publication_freeze(commit)).await;
        }
        if commit.revision_id != commit.readiness.revision_id
            || !commit.readiness.can_freeze
            || commit
                .readiness
                .blockers
                .iter()
                .any(|finding| !finding.waived || !finding.waivable)
        {
            anyhow::bail!("Publication readiness contains unresolved blockers");
        }
        let policy_value: serde_json::Value = serde_json::from_str(&commit.policy_json)
            .map_err(|_| anyhow::anyhow!("Publication freeze policy must be valid JSON"))?;
        let canonical_policy = canonical_json(&policy_value);
        if canonical_policy != commit.policy_json {
            anyhow::bail!("Publication freeze policy must be canonical JSON");
        }
        let manifest_value: serde_json::Value =
            serde_json::from_str(&commit.readiness.manifest_json)
                .map_err(|_| anyhow::anyhow!("Publication manifest must be valid JSON"))?;
        let (canonical_manifest, manifest_sha256) = canonical_json_sha256(&manifest_value);
        if canonical_manifest != commit.readiness.manifest_json
            || manifest_sha256 != commit.readiness.manifest_sha256
        {
            anyhow::bail!("Publication manifest hash or canonical form is invalid");
        }
        if manifest_value
            .get("schema_version")
            .and_then(|value| value.as_i64())
            != Some(1)
            || manifest_value
                .get("publication_revision_id")
                .and_then(|value| value.as_str())
                != Some(commit.revision_id.as_str())
            || manifest_value
                .get("target_visibility")
                .and_then(|value| value.as_str())
                != Some(commit.readiness.target_visibility.as_str())
            || manifest_value
                .get("capability_level")
                .and_then(|value| value.as_str())
                != Some(commit.readiness.capability_level.as_str())
            || manifest_value.get("policy") != Some(&policy_value)
            || manifest_value.get("blockers")
                != Some(&serde_json::to_value(&commit.readiness.blockers)?)
            || manifest_value.get("warnings")
                != Some(&serde_json::to_value(&commit.readiness.warnings)?)
            || manifest_value.get("omissions")
                != Some(&serde_json::to_value(&commit.readiness.omissions)?)
        {
            anyhow::bail!("Publication manifest does not match its prepared readiness");
        }

        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let attempt = sqlx::query(
            "SELECT attempt.revision_id,attempt.target_visibility,attempt.policy_json,\
                    publication.project_id \
             FROM publication_freeze_attempts attempt \
             JOIN publication_revisions revision ON revision.id=attempt.revision_id \
             JOIN publications publication ON publication.id=revision.publication_id \
             WHERE attempt.id=? AND revision.state='freezing'",
        )
        .bind(&commit.attempt_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Publication freeze attempt is no longer active"))?;
        let attempt_revision: String = attempt.try_get("revision_id")?;
        let target_visibility: String = attempt.try_get("target_visibility")?;
        let stored_policy: String = attempt.try_get("policy_json")?;
        let project_id: String = attempt.try_get("project_id")?;
        if attempt_revision != commit.revision_id
            || target_visibility != commit.readiness.target_visibility.as_str()
            || stored_policy != canonical_policy
        {
            anyhow::bail!("Publication freeze attempt no longer matches its prepared policy");
        }

        let mut captures = commit.late_captures.clone();
        captures.sort_by(|left, right| left.new_version_id.cmp(&right.new_version_id));
        for capture in captures {
            if capture.binding_ids.is_empty()
                || capture.version_number <= 0
                || capture.size_bytes < 0
                || capture.checksum.len() != 64
                || !capture
                    .checksum
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                || matches!(capture.materialization, ArtifactMaterialization::External)
            {
                anyhow::bail!("Prepared late capture is invalid");
            }
            let snapshot_value: serde_json::Value =
                serde_json::from_str(&capture.source_snapshot_json)
                    .map_err(|_| anyhow::anyhow!("Late-capture source snapshot is invalid"))?;
            if canonical_json(&snapshot_value) != capture.source_snapshot_json {
                anyhow::bail!("Late-capture source snapshot must be canonical JSON");
            }
            let artifact =
                sqlx::query("SELECT project_id,latest_version_id FROM artifacts WHERE id=?")
                    .bind(&capture.artifact_id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("Late-capture Artifact no longer exists"))?;
            let artifact_project: String = artifact.try_get("project_id")?;
            let latest_version_id: Option<String> = artifact.try_get("latest_version_id")?;
            if artifact_project != project_id
                || latest_version_id != capture.expected_latest_version_id
            {
                anyhow::bail!("Artifact changed while Publication freeze was prepared");
            }
            let expected_number: i64 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(version_number),0)+1 FROM artifact_versions \
                 WHERE artifact_id=?",
            )
            .bind(&capture.artifact_id)
            .fetch_one(&mut *tx)
            .await?;
            if expected_number != capture.version_number {
                anyhow::bail!("Artifact version sequence changed during Publication freeze");
            }
            for binding_id in &capture.binding_ids {
                let exact_binding: bool = sqlx::query_scalar(
                    "SELECT EXISTS(\
                       SELECT 1 FROM evidence_bindings binding \
                       JOIN artifact_versions version \
                         ON version.id=binding.artifact_version_id \
                       WHERE binding.id=? AND binding.revision_id=? \
                         AND binding.artifact_version_id=? AND version.artifact_id=?\
                     )",
                )
                .bind(binding_id)
                .bind(&commit.revision_id)
                .bind(&capture.old_version_id)
                .bind(&capture.artifact_id)
                .fetch_one(&mut *tx)
                .await?;
                if !exact_binding {
                    anyhow::bail!("Evidence binding changed while Publication freeze was prepared");
                }
            }
            sqlx::query(
                "INSERT INTO artifact_versions(\
                   id,artifact_id,version_number,content_type,storage_path,size_bytes,checksum,\
                   parent_version_id,producing_run_id,env_snapshot_hash,materialization,\
                   capture_timing,created_at\
                 ) VALUES(?,?,?,?,?,?,?,?,NULL,NULL,?,'late',?)",
            )
            .bind(&capture.new_version_id)
            .bind(&capture.artifact_id)
            .bind(capture.version_number)
            .bind(&capture.content_type)
            .bind(&capture.storage_path)
            .bind(capture.size_bytes)
            .bind(&capture.checksum)
            .bind(capture.expected_latest_version_id.as_deref())
            .bind(capture.materialization.as_str())
            .bind(now)
            .execute(&mut *tx)
            .await?;
            let updated_artifact = sqlx::query(
                "UPDATE artifacts SET latest_version_id=?,storage_path=?,content_type=? \
                 WHERE id=? AND latest_version_id IS ?",
            )
            .bind(&capture.new_version_id)
            .bind(&capture.storage_path)
            .bind(&capture.content_type)
            .bind(&capture.artifact_id)
            .bind(capture.expected_latest_version_id.as_deref())
            .execute(&mut *tx)
            .await?;
            if updated_artifact.rows_affected() != 1 {
                anyhow::bail!("Artifact changed while Publication freeze was committed");
            }
            for binding_id in &capture.binding_ids {
                let updated = sqlx::query(
                    "UPDATE evidence_bindings SET source_id=?,artifact_version_id=?,\
                       source_snapshot_json=?,updated_at=? \
                     WHERE id=? AND revision_id=? AND artifact_version_id=?",
                )
                .bind(&capture.new_version_id)
                .bind(&capture.new_version_id)
                .bind(&capture.source_snapshot_json)
                .bind(now)
                .bind(binding_id)
                .bind(&commit.revision_id)
                .bind(&capture.old_version_id)
                .execute(&mut *tx)
                .await?;
                if updated.rows_affected() != 1 {
                    anyhow::bail!(
                        "Evidence binding changed while Publication freeze was committed"
                    );
                }
                let item_id: Option<String> =
                    sqlx::query_scalar("SELECT item_id FROM evidence_bindings WHERE id=?")
                        .bind(binding_id)
                        .fetch_one(&mut *tx)
                        .await?;
                let metadata = canonical_json(&serde_json::json!({
                    "binding_id": binding_id,
                    "revision_id": commit.revision_id,
                    "item_id": item_id,
                    "source_kind": "artifact_version",
                    "source_id": capture.new_version_id,
                }));
                sqlx::query("UPDATE research_edges SET metadata_json=? WHERE id=?")
                    .bind(metadata)
                    .bind(format!("publication-evidence:{binding_id}"))
                    .execute(&mut *tx)
                    .await?;
            }
        }

        let blockers_json = canonical_json(&serde_json::to_value(&commit.readiness.blockers)?);
        let warnings_json = canonical_json(&serde_json::to_value(&commit.readiness.warnings)?);
        let omissions_json = canonical_json(&serde_json::to_value(&commit.readiness.omissions)?);
        sqlx::query(
            "INSERT INTO publication_readiness_reports(\
               id,revision_id,capability_level,target_visibility,policy_json,blockers_json,\
               warnings_json,omissions_json,manifest_json,manifest_sha256,created_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(format!("publication-readiness:{}", commit.revision_id))
        .bind(&commit.revision_id)
        .bind(commit.readiness.capability_level.as_str())
        .bind(commit.readiness.target_visibility.as_str())
        .bind(&canonical_policy)
        .bind(blockers_json)
        .bind(warnings_json)
        .bind(omissions_json)
        .bind(&commit.readiness.manifest_json)
        .bind(&commit.readiness.manifest_sha256)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let removed_attempt = sqlx::query("DELETE FROM publication_freeze_attempts WHERE id=?")
            .bind(&commit.attempt_id)
            .execute(&mut *tx)
            .await?;
        if removed_attempt.rows_affected() != 1 {
            anyhow::bail!("Publication freeze attempt disappeared before commit");
        }
        let frozen = sqlx::query(
            "UPDATE publication_revisions SET \
               state='frozen',capability_level=?,manifest_json=?,manifest_sha256=?,\
               frozen_at=?,updated_at=? \
             WHERE id=? AND state='freezing'",
        )
        .bind(commit.readiness.capability_level.as_str())
        .bind(&commit.readiness.manifest_json)
        .bind(&commit.readiness.manifest_sha256)
        .bind(now)
        .bind(now)
        .bind(&commit.revision_id)
        .execute(&mut *tx)
        .await?;
        if frozen.rows_affected() != 1 {
            anyhow::bail!("Publication revision changed before freeze commit");
        }
        tx.commit().await?;
        self.get_publication_revision(&commit.revision_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Frozen Publication revision was not persisted"))
    }

    pub async fn get_publication_readiness_report(
        &self,
        revision_id: &str,
    ) -> Result<Option<PublicationReadinessReport>> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.get_publication_readiness_report(revision_id)).await;
        }
        let row = sqlx::query(
            "SELECT id,revision_id,capability_level,target_visibility,policy_json,\
                    blockers_json,warnings_json,omissions_json,manifest_json,manifest_sha256,\
                    created_at \
             FROM publication_readiness_reports WHERE revision_id=?",
        )
        .bind(revision_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let capability: String = row.try_get("capability_level")?;
            let visibility: String = row.try_get("target_visibility")?;
            Ok(PublicationReadinessReport {
                id: row.try_get("id")?,
                revision_id: row.try_get("revision_id")?,
                capability_level: PublicationCapabilityLevel::from_storage(&capability)?,
                target_visibility: EvidenceVisibility::from_storage(&visibility)?,
                policy_json: row.try_get("policy_json")?,
                blockers_json: row.try_get("blockers_json")?,
                warnings_json: row.try_get("warnings_json")?,
                omissions_json: row.try_get("omissions_json")?,
                manifest_json: row.try_get("manifest_json")?,
                manifest_sha256: row.try_get("manifest_sha256")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .transpose()
    }

    pub async fn list_publication_evidence_drift(
        &self,
        revision_id: &str,
    ) -> Result<Vec<PublicationEvidenceDrift>> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.list_publication_evidence_drift(revision_id)).await;
        }
        let rows = sqlx::query(
            "SELECT binding.id AS binding_id,artifact.id AS artifact_id,\
                    artifact.logical_key AS logical_key,bound.id AS bound_version_id,\
                    bound.version_number AS bound_version_number,\
                    latest.id AS latest_version_id,latest.version_number AS latest_version_number \
             FROM evidence_bindings binding \
             JOIN artifact_versions bound ON bound.id=binding.artifact_version_id \
             JOIN artifacts artifact ON artifact.id=bound.artifact_id \
             JOIN artifact_versions latest ON latest.id=artifact.latest_version_id \
             WHERE binding.revision_id=? AND binding.source_kind='artifact_version' \
             ORDER BY binding.id",
        )
        .bind(revision_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let bound_version_id: String = row.try_get("bound_version_id")?;
                let latest_version_id: String = row.try_get("latest_version_id")?;
                Ok(PublicationEvidenceDrift {
                    binding_id: row.try_get("binding_id")?,
                    artifact_id: row.try_get("artifact_id")?,
                    logical_key: row.try_get("logical_key")?,
                    has_drift: bound_version_id != latest_version_id,
                    bound_version_id,
                    bound_version_number: row.try_get("bound_version_number")?,
                    latest_version_id,
                    latest_version_number: row.try_get("latest_version_number")?,
                })
            })
            .collect()
    }

    pub async fn publish_publication_revision(&self, revision_id: &str) -> Result<()> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.publish_publication_revision(revision_id)).await;
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE publication_revisions SET state='published',published_at=?,updated_at=? \
             WHERE id=? AND state='frozen'",
        )
        .bind(now)
        .bind(now)
        .bind(revision_id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Frozen Publication revision not found");
        }
        Ok(())
    }

    pub async fn start_capsule_build(
        &self,
        id: &str,
        revision_id: &str,
        format: &str,
        visibility: EvidenceVisibility,
        output_path: &str,
        revision_manifest_sha256: &str,
    ) -> Result<CapsuleBuild> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.start_capsule_build(
                id,
                revision_id,
                format,
                visibility,
                output_path,
                revision_manifest_sha256,
            ))
            .await;
        }
        if id.trim().is_empty()
            || format != "zip"
            || output_path.trim().is_empty()
            || revision_manifest_sha256.len() != 64
            || !revision_manifest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            anyhow::bail!("Invalid Capsule Build request");
        }
        let now = chrono::Utc::now().timestamp();
        let inserted = sqlx::query(
            "INSERT INTO capsule_builds(\
               id,revision_id,format,visibility,status,output_path,revision_manifest_sha256,\
               archive_sha256,error,created_at,completed_at\
             ) \
             SELECT ?,revision.id,?,?,'building',?,?,NULL,NULL,?,NULL \
             FROM publication_revisions revision \
             JOIN publication_readiness_reports report ON report.revision_id=revision.id \
             WHERE revision.id=? AND revision.state IN ('frozen','published') \
               AND revision.manifest_sha256=? AND report.manifest_sha256=? \
               AND report.target_visibility=?",
        )
        .bind(id)
        .bind(format)
        .bind(visibility.as_str())
        .bind(output_path)
        .bind(revision_manifest_sha256)
        .bind(now)
        .bind(revision_id)
        .bind(revision_manifest_sha256)
        .bind(revision_manifest_sha256)
        .bind(visibility.as_str())
        .execute(&self.pool)
        .await?;
        if inserted.rows_affected() != 1 {
            anyhow::bail!("Frozen Publication manifest is unavailable or changed");
        }
        self.get_capsule_build(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Capsule Build was not persisted"))
    }

    pub async fn complete_capsule_build(
        &self,
        id: &str,
        archive_sha256: &str,
    ) -> Result<CapsuleBuild> {
        if let Some(store) = self.route_entity("capsule_builds", "id", id).await? {
            return Box::pin(store.complete_capsule_build(id, archive_sha256)).await;
        }
        if archive_sha256.len() != 64
            || !archive_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            anyhow::bail!("Capsule archive SHA-256 is invalid");
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE capsule_builds \
             SET status='succeeded',archive_sha256=?,error=NULL,completed_at=? \
             WHERE id=? AND status='building'",
        )
        .bind(archive_sha256)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Capsule Build is no longer active");
        }
        self.get_capsule_build(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Completed Capsule Build was not found"))
    }

    pub async fn fail_capsule_build(&self, id: &str, error: &str) -> Result<CapsuleBuild> {
        if let Some(store) = self.route_entity("capsule_builds", "id", id).await? {
            return Box::pin(store.fail_capsule_build(id, error)).await;
        }
        let now = chrono::Utc::now().timestamp();
        let error = error.chars().take(2_000).collect::<String>();
        let updated = sqlx::query(
            "UPDATE capsule_builds \
             SET status='failed',archive_sha256=NULL,error=?,completed_at=? \
             WHERE id=? AND status='building'",
        )
        .bind(error)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Capsule Build is no longer active");
        }
        self.get_capsule_build(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Failed Capsule Build was not found"))
    }

    pub async fn get_capsule_build(&self, id: &str) -> Result<Option<CapsuleBuild>> {
        if let Some(store) = self.route_entity("capsule_builds", "id", id).await? {
            return Box::pin(store.get_capsule_build(id)).await;
        }
        let row = sqlx::query(
            "SELECT id,revision_id,format,visibility,status,output_path,\
                    revision_manifest_sha256,archive_sha256,error,created_at,completed_at \
             FROM capsule_builds WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(capsule_build_from_row).transpose()
    }

    pub async fn list_capsule_builds(&self, revision_id: &str) -> Result<Vec<CapsuleBuild>> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.list_capsule_builds(revision_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,revision_id,format,visibility,status,output_path,\
                    revision_manifest_sha256,archive_sha256,error,created_at,completed_at \
             FROM capsule_builds WHERE revision_id=? ORDER BY created_at DESC,id DESC",
        )
        .bind(revision_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(capsule_build_from_row).collect()
    }

    pub async fn start_reproduction_run(
        &self,
        start: &ReproductionRunStart,
    ) -> Result<ReproductionRun> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", &start.revision_id)
            .await?
        {
            return Box::pin(store.start_reproduction_run(start)).await;
        }
        let valid_hash = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        };
        if start.id.trim().is_empty()
            || start.revision_id.trim().is_empty()
            || start.source_run_id.trim().is_empty()
            || !valid_hash(&start.command_sha256)
            || !valid_hash(&start.actual_environment_hash)
            || start
                .expected_environment_hash
                .as_deref()
                .is_some_and(|hash| !valid_hash(hash))
        {
            anyhow::bail!("Invalid reproduction run request");
        }
        let actual_environment: serde_json::Value =
            serde_json::from_str(&start.actual_environment_json)?;
        let (actual_json, actual_hash) = canonical_json_sha256(&actual_environment);
        if actual_json != start.actual_environment_json
            || actual_hash != start.actual_environment_hash
        {
            anyhow::bail!("Actual reproduction environment is not canonical or hash-valid");
        }
        let workspace: serde_json::Value = serde_json::from_str(&start.workspace_manifest_json)?;
        if canonical_json(&workspace) != start.workspace_manifest_json {
            anyhow::bail!("Reproduction workspace manifest must be canonical JSON");
        }
        let now = chrono::Utc::now().timestamp();
        let inserted = sqlx::query(
            "INSERT INTO reproduction_runs(\
               id,revision_id,source_run_id,status,capability_level,command_sha256,\
               expected_environment_hash,actual_environment_json,actual_environment_hash,\
               environment_matched,workspace_manifest_json,stdout_tail,stderr_tail,exit_code,\
               error,created_at,started_at,completed_at\
             ) \
             SELECT ?,revision.id,run.id,'running','re_executable',?,?,?,?,?, ?,\
                    NULL,NULL,NULL,NULL,?,?,NULL \
             FROM publication_revisions revision \
             JOIN publications publication ON publication.id=revision.publication_id \
             JOIN runs run ON run.id=? AND run.project_id=publication.project_id \
             WHERE revision.id=? AND revision.state IN ('frozen','published') \
               AND revision.manifest_json IS NOT NULL",
        )
        .bind(&start.id)
        .bind(&start.command_sha256)
        .bind(start.expected_environment_hash.as_deref())
        .bind(&start.actual_environment_json)
        .bind(&start.actual_environment_hash)
        .bind(i64::from(start.environment_matched))
        .bind(&start.workspace_manifest_json)
        .bind(now)
        .bind(now)
        .bind(&start.source_run_id)
        .bind(&start.revision_id)
        .execute(&self.pool)
        .await?;
        if inserted.rows_affected() != 1 {
            anyhow::bail!("Frozen Publication revision or source Run is unavailable");
        }
        self.get_reproduction_run(&start.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Reproduction run was not persisted"))
    }

    pub async fn complete_reproduction_run(
        &self,
        commit: &ReproductionRunCommit,
    ) -> Result<ReproductionRun> {
        if let Some(store) = self
            .route_entity("reproduction_runs", "id", &commit.run_id)
            .await?
        {
            return Box::pin(store.complete_reproduction_run(commit)).await;
        }
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let environment_matched: Option<i64> = sqlx::query_scalar(
            "SELECT environment_matched FROM reproduction_runs \
             WHERE id=? AND status='running'",
        )
        .bind(&commit.run_id)
        .fetch_optional(&mut *tx)
        .await?;
        let environment_matched = environment_matched
            .ok_or_else(|| anyhow::anyhow!("Reproduction run is no longer active"))?
            != 0;
        let mut output_ids = std::collections::HashSet::new();
        for result in &commit.results {
            if result.id.trim().is_empty()
                || result.reproduction_run_id != commit.run_id
                || result.output_id.trim().is_empty()
                || result.output_path.trim().is_empty()
                || result.expected_artifact_version_id.trim().is_empty()
                || !output_ids.insert(result.output_id.as_str())
            {
                anyhow::bail!("Invalid or duplicate reproduction result");
            }
            for json in [
                &result.expected_json,
                &result.actual_json,
                &result.tolerance_json,
                &result.report_json,
            ] {
                serde_json::from_str::<serde_json::Value>(json)?;
            }
            sqlx::query(
                "INSERT INTO reproduction_results(\
                   id,reproduction_run_id,output_id,output_path,\
                   expected_artifact_version_id,comparator_kind,required,expected_json,\
                   actual_json,tolerance_json,passed,report_json,created_at\
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&result.id)
            .bind(&result.reproduction_run_id)
            .bind(&result.output_id)
            .bind(&result.output_path)
            .bind(&result.expected_artifact_version_id)
            .bind(result.comparator_kind.as_str())
            .bind(i64::from(result.required))
            .bind(&result.expected_json)
            .bind(&result.actual_json)
            .bind(&result.tolerance_json)
            .bind(i64::from(result.passed))
            .bind(&result.report_json)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        let reproduced = environment_matched
            && commit.exit_code == 0
            && !commit.results.is_empty()
            && commit
                .results
                .iter()
                .filter(|result| result.required)
                .all(|result| result.passed);
        let updated = sqlx::query(
            "UPDATE reproduction_runs SET status='completed',capability_level=?,\
             stdout_tail=?,stderr_tail=?,exit_code=?,error=NULL,completed_at=? \
             WHERE id=? AND status='running'",
        )
        .bind(if reproduced {
            PublicationCapabilityLevel::Reproduced.as_str()
        } else {
            PublicationCapabilityLevel::ReExecutable.as_str()
        })
        .bind(
            commit
                .stdout_tail
                .chars()
                .take(64 * 1024)
                .collect::<String>(),
        )
        .bind(
            commit
                .stderr_tail
                .chars()
                .take(64 * 1024)
                .collect::<String>(),
        )
        .bind(commit.exit_code)
        .bind(now)
        .bind(&commit.run_id)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Reproduction run changed while completing");
        }
        tx.commit().await?;
        self.get_reproduction_run(&commit.run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Completed reproduction run was not found"))
    }

    pub async fn fail_reproduction_run(&self, id: &str, error: &str) -> Result<ReproductionRun> {
        if let Some(store) = self.route_entity("reproduction_runs", "id", id).await? {
            return Box::pin(store.fail_reproduction_run(id, error)).await;
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE reproduction_runs SET status='failed',capability_level='re_executable',\
             error=?,completed_at=? WHERE id=? AND status='running'",
        )
        .bind(error.chars().take(2_000).collect::<String>())
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Reproduction run is no longer active");
        }
        self.get_reproduction_run(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Failed reproduction run was not found"))
    }

    pub async fn get_reproduction_run(&self, id: &str) -> Result<Option<ReproductionRun>> {
        if let Some(store) = self.route_entity("reproduction_runs", "id", id).await? {
            return Box::pin(store.get_reproduction_run(id)).await;
        }
        let row = sqlx::query(
            "SELECT id,revision_id,source_run_id,status,capability_level,command_sha256,\
                    expected_environment_hash,actual_environment_json,actual_environment_hash,\
                    environment_matched,workspace_manifest_json,stdout_tail,stderr_tail,exit_code,\
                    error,created_at,started_at,completed_at \
             FROM reproduction_runs WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(reproduction_run_from_row).transpose()
    }

    pub async fn list_reproduction_runs(&self, revision_id: &str) -> Result<Vec<ReproductionRun>> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", revision_id)
            .await?
        {
            return Box::pin(store.list_reproduction_runs(revision_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,revision_id,source_run_id,status,capability_level,command_sha256,\
                    expected_environment_hash,actual_environment_json,actual_environment_hash,\
                    environment_matched,workspace_manifest_json,stdout_tail,stderr_tail,exit_code,\
                    error,created_at,started_at,completed_at \
             FROM reproduction_runs WHERE revision_id=? ORDER BY created_at DESC,id DESC",
        )
        .bind(revision_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(reproduction_run_from_row).collect()
    }

    pub async fn list_reproduction_results(&self, run_id: &str) -> Result<Vec<ReproductionResult>> {
        if let Some(store) = self.route_entity("reproduction_runs", "id", run_id).await? {
            return Box::pin(store.list_reproduction_results(run_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,reproduction_run_id,output_id,output_path,\
                    expected_artifact_version_id,comparator_kind,required,expected_json,\
                    actual_json,tolerance_json,passed,report_json,created_at \
             FROM reproduction_results WHERE reproduction_run_id=? ORDER BY output_id,id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(reproduction_result_from_row).collect()
    }

    pub async fn clone_publication_revision(
        &self,
        source_revision_id: &str,
        new_revision_id: &str,
        label: &str,
    ) -> Result<PublicationRevision> {
        if let Some(store) = self
            .route_entity("publication_revisions", "id", source_revision_id)
            .await?
        {
            return Box::pin(store.clone_publication_revision(
                source_revision_id,
                new_revision_id,
                label,
            ))
            .await;
        }
        if new_revision_id.trim().is_empty() || label.trim().is_empty() {
            anyhow::bail!("Cloned revision requires identity and label");
        }
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let source: Option<(String, String)> =
            sqlx::query_as("SELECT publication_id,state FROM publication_revisions WHERE id=?")
                .bind(source_revision_id)
                .fetch_optional(&mut *tx)
                .await?;
        let (publication_id, source_state) =
            source.ok_or_else(|| anyhow::anyhow!("Source revision not found"))?;
        if matches!(source_state.as_str(), "freezing" | "deleting") {
            anyhow::bail!("Publication revision cannot be cloned while {source_state}");
        }
        let revision_number: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(revision_number),0)+1 FROM publication_revisions \
             WHERE publication_id=?",
        )
        .bind(&publication_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO publication_revisions(\
               id,publication_id,parent_revision_id,revision_number,label,state,capability_level,\
               manifest_json,manifest_sha256,frozen_at,published_at,created_at,updated_at\
             ) VALUES(?,?,?,?,?,'draft','archived',NULL,NULL,NULL,NULL,?,?)",
        )
        .bind(new_revision_id)
        .bind(&publication_id)
        .bind(source_revision_id)
        .bind(revision_number)
        .bind(label.trim())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        let source_items = sqlx::query(
            "SELECT id,parent_item_id,kind,title,content,ordinal,metadata_json,created_at \
             FROM publication_items WHERE revision_id=? ORDER BY created_at,id",
        )
        .bind(source_revision_id)
        .fetch_all(&mut *tx)
        .await?;
        let item_ids = source_items
            .iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("id")?,
                    uuid::Uuid::new_v4().to_string(),
                ))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        for (index, row) in source_items.iter().enumerate() {
            let source_id: String = row.try_get("id")?;
            sqlx::query(
                "INSERT INTO publication_items(\
                   id,revision_id,parent_item_id,kind,title,content,ordinal,metadata_json,\
                   created_at,updated_at\
                 ) VALUES(?,?,NULL,?,?,?,?,?,?,?)",
            )
            .bind(&item_ids[&source_id])
            .bind(new_revision_id)
            .bind(row.try_get::<String, _>("kind")?)
            .bind(row.try_get::<String, _>("title")?)
            .bind(row.try_get::<String, _>("content")?)
            .bind(-i64::try_from(index + 1).unwrap_or(i64::MAX))
            .bind(row.try_get::<String, _>("metadata_json")?)
            .bind(row.try_get::<i64, _>("created_at")?)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        for row in &source_items {
            let source_id: String = row.try_get("id")?;
            let parent_id: Option<String> = row.try_get("parent_item_id")?;
            if let Some(parent_id) = parent_id {
                let mapped_parent = item_ids
                    .get(&parent_id)
                    .ok_or_else(|| anyhow::anyhow!("Source item parent is missing"))?;
                sqlx::query("UPDATE publication_items SET parent_item_id=? WHERE id=?")
                    .bind(mapped_parent)
                    .bind(&item_ids[&source_id])
                    .execute(&mut *tx)
                    .await?;
            }
        }
        for row in &source_items {
            let source_id: String = row.try_get("id")?;
            sqlx::query("UPDATE publication_items SET ordinal=? WHERE id=?")
                .bind(row.try_get::<i64, _>("ordinal")?)
                .bind(&item_ids[&source_id])
                .execute(&mut *tx)
                .await?;
        }

        let source_links = sqlx::query(
            "SELECT source_item_id,target_item_id,relation,created_at \
             FROM publication_item_links WHERE revision_id=? ORDER BY created_at,id",
        )
        .bind(source_revision_id)
        .fetch_all(&mut *tx)
        .await?;
        for row in source_links {
            let source_item: String = row.try_get("source_item_id")?;
            let target_item: String = row.try_get("target_item_id")?;
            sqlx::query(
                "INSERT INTO publication_item_links(\
                   id,revision_id,source_item_id,target_item_id,relation,created_at\
                 ) VALUES(?,?,?,?,?,?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(new_revision_id)
            .bind(
                item_ids
                    .get(&source_item)
                    .ok_or_else(|| anyhow::anyhow!("Source item link is incomplete"))?,
            )
            .bind(
                item_ids
                    .get(&target_item)
                    .ok_or_else(|| anyhow::anyhow!("Source item link is incomplete"))?,
            )
            .bind(row.try_get::<String, _>("relation")?)
            .bind(row.try_get::<i64, _>("created_at")?)
            .execute(&mut *tx)
            .await?;
        }

        let source_bindings = sqlx::query(&format!(
            "SELECT {EVIDENCE_BINDING_COLUMNS} FROM evidence_bindings \
             WHERE revision_id=? ORDER BY created_at,id"
        ))
        .bind(source_revision_id)
        .fetch_all(&mut *tx)
        .await?;
        let binding_ids = source_bindings
            .iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("id")?,
                    uuid::Uuid::new_v4().to_string(),
                ))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        for row in &source_bindings {
            let source_id: String = row.try_get("id")?;
            let item_id = row
                .try_get::<Option<String>, _>("item_id")?
                .map(|id| {
                    item_ids
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("Evidence item is missing"))
                })
                .transpose()?;
            let claim_id = row
                .try_get::<Option<String>, _>("supported_claim_item_id")?
                .map(|id| {
                    item_ids
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("Evidence claim is missing"))
                })
                .transpose()?;
            sqlx::query(
                "INSERT INTO evidence_bindings(\
                   id,revision_id,item_id,source_kind,source_id,artifact_version_id,run_id,\
                   external_resource_id,purpose,supported_claim_item_id,selection_state,\
                   review_state,reproduction_state,visibility,source_snapshot_json,\
                   created_at,updated_at\
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&binding_ids[&source_id])
            .bind(new_revision_id)
            .bind(item_id)
            .bind(row.try_get::<String, _>("source_kind")?)
            .bind(row.try_get::<String, _>("source_id")?)
            .bind(row.try_get::<Option<String>, _>("artifact_version_id")?)
            .bind(row.try_get::<Option<String>, _>("run_id")?)
            .bind(row.try_get::<Option<String>, _>("external_resource_id")?)
            .bind(row.try_get::<String, _>("purpose")?)
            .bind(claim_id)
            .bind(row.try_get::<String, _>("selection_state")?)
            .bind(row.try_get::<String, _>("review_state")?)
            .bind(row.try_get::<String, _>("reproduction_state")?)
            .bind(row.try_get::<String, _>("visibility")?)
            .bind(row.try_get::<String, _>("source_snapshot_json")?)
            .bind(row.try_get::<i64, _>("created_at")?)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }

        let source_reviews = sqlx::query(
            "SELECT review.id,review.binding_id,review.reviewer,review.method,\
                    review.verified_at,review.environment_json,review.comparator_json,\
                    review.tolerance_json,review.result,review.report_json,review.created_at \
             FROM evidence_reviews review \
             JOIN evidence_bindings binding ON binding.id=review.binding_id \
             WHERE binding.revision_id=? ORDER BY review.created_at,review.id",
        )
        .bind(source_revision_id)
        .fetch_all(&mut *tx)
        .await?;
        for row in source_reviews {
            let binding_id: String = row.try_get("binding_id")?;
            sqlx::query(
                "INSERT INTO evidence_reviews(\
                   id,binding_id,reviewer,method,verified_at,environment_json,comparator_json,\
                   tolerance_json,result,report_json,created_at\
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(
                binding_ids
                    .get(&binding_id)
                    .ok_or_else(|| anyhow::anyhow!("Evidence review binding is missing"))?,
            )
            .bind(row.try_get::<String, _>("reviewer")?)
            .bind(row.try_get::<String, _>("method")?)
            .bind(row.try_get::<i64, _>("verified_at")?)
            .bind(row.try_get::<String, _>("environment_json")?)
            .bind(row.try_get::<String, _>("comparator_json")?)
            .bind(row.try_get::<String, _>("tolerance_json")?)
            .bind(row.try_get::<String, _>("result")?)
            .bind(row.try_get::<String, _>("report_json")?)
            .bind(row.try_get::<i64, _>("created_at")?)
            .execute(&mut *tx)
            .await?;
        }

        let source_supersessions = sqlx::query(
            "SELECT old_binding_id,new_binding_id,reason,created_at \
             FROM evidence_supersessions WHERE revision_id=? ORDER BY created_at,id",
        )
        .bind(source_revision_id)
        .fetch_all(&mut *tx)
        .await?;
        for row in source_supersessions {
            let old_id: String = row.try_get("old_binding_id")?;
            let new_id: String = row.try_get("new_binding_id")?;
            sqlx::query(
                "INSERT INTO evidence_supersessions(\
                   id,revision_id,old_binding_id,new_binding_id,reason,created_at\
                 ) VALUES(?,?,?,?,?,?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(new_revision_id)
            .bind(
                binding_ids
                    .get(&old_id)
                    .ok_or_else(|| anyhow::anyhow!("Superseded binding is missing"))?,
            )
            .bind(
                binding_ids
                    .get(&new_id)
                    .ok_or_else(|| anyhow::anyhow!("Replacement binding is missing"))?,
            )
            .bind(row.try_get::<String, _>("reason")?)
            .bind(row.try_get::<i64, _>("created_at")?)
            .execute(&mut *tx)
            .await?;
        }

        let source_waivers = sqlx::query(
            "SELECT finding_code,author,reason,created_at \
             FROM publication_waivers WHERE revision_id=? ORDER BY created_at,id",
        )
        .bind(source_revision_id)
        .fetch_all(&mut *tx)
        .await?;
        for row in source_waivers {
            sqlx::query(
                "INSERT INTO publication_waivers(\
                   id,revision_id,finding_code,author,reason,created_at\
                 ) VALUES(?,?,?,?,?,?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(new_revision_id)
            .bind(row.try_get::<String, _>("finding_code")?)
            .bind(row.try_get::<String, _>("author")?)
            .bind(row.try_get::<String, _>("reason")?)
            .bind(row.try_get::<i64, _>("created_at")?)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        for binding_id in binding_ids.values() {
            self.sync_stored_evidence_projection(binding_id).await?;
        }
        self.get_publication_revision(new_revision_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Cloned revision was not persisted"))
    }

    async fn sync_stored_evidence_projection(&self, binding_id: &str) -> Result<()> {
        let binding = self
            .get_evidence_binding(binding_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Evidence binding not found"))?;
        let project_id: String = sqlx::query_scalar(
            "SELECT publication.project_id \
             FROM publication_revisions revision \
             JOIN publications publication ON publication.id=revision.publication_id \
             WHERE revision.id=?",
        )
        .bind(&binding.revision_id)
        .fetch_one(&self.pool)
        .await?;
        let (target_node_id, target_kind, target_title) = match binding.source_kind {
            EvidenceSourceKind::ArtifactVersion => {
                let row = sqlx::query(
                    "SELECT artifact.id,artifact.filename \
                     FROM artifact_versions version \
                     JOIN artifacts artifact ON artifact.id=version.artifact_id \
                     WHERE version.id=?",
                )
                .bind(&binding.source_id)
                .fetch_one(&self.pool)
                .await?;
                let artifact_id: String = row.try_get("id")?;
                (
                    artifact_node_id(&artifact_id),
                    ResearchNodeKind::Artifact,
                    row.try_get::<String, _>("filename")?,
                )
            }
            EvidenceSourceKind::Run => (
                run_node_id(&binding.source_id),
                ResearchNodeKind::Run,
                sqlx::query_scalar("SELECT title FROM runs WHERE id=?")
                    .bind(&binding.source_id)
                    .fetch_one(&self.pool)
                    .await?,
            ),
            EvidenceSourceKind::ExecutionLog | EvidenceSourceKind::CodeCell => {
                let snapshot: serde_json::Value =
                    serde_json::from_str(&binding.source_snapshot_json)?;
                let language = snapshot
                    .pointer("/anchor/language")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Execution");
                let cell = snapshot
                    .pointer("/anchor/cell_index")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default();
                (
                    source_node_id(binding.source_kind, &binding.source_id),
                    ResearchNodeKind::Run,
                    format!("{language} cell {cell}"),
                )
            }
            EvidenceSourceKind::MessageSpan => {
                let snapshot: serde_json::Value =
                    serde_json::from_str(&binding.source_snapshot_json)?;
                let seq = snapshot
                    .pointer("/anchor/message_seq")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default();
                (
                    source_node_id(binding.source_kind, &binding.source_id),
                    ResearchNodeKind::Artifact,
                    format!("Message {seq} excerpt"),
                )
            }
            EvidenceSourceKind::ToolCall => {
                let snapshot: serde_json::Value =
                    serde_json::from_str(&binding.source_snapshot_json)?;
                let name = snapshot
                    .pointer("/anchor/name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("tool");
                (
                    source_node_id(binding.source_kind, &binding.source_id),
                    ResearchNodeKind::Artifact,
                    format!("Tool result: {name}"),
                )
            }
            EvidenceSourceKind::ExternalResource => {
                let snapshot: serde_json::Value =
                    serde_json::from_str(&binding.source_snapshot_json)?;
                let kind = snapshot
                    .pointer("/anchor/kind")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("resource");
                let uri = snapshot
                    .pointer("/anchor/uri")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&binding.source_id);
                (
                    source_node_id(binding.source_kind, &binding.source_id),
                    ResearchNodeKind::DataAsset,
                    format!("{kind}: {uri}"),
                )
            }
        };
        self.sync_evidence_projection(
            &binding.id,
            &project_id,
            &binding.revision_id,
            binding.item_id.as_deref(),
            binding.source_kind,
            &binding.source_id,
            &target_node_id,
            target_kind,
            &target_title,
        )
        .await
    }
}
