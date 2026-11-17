//! What the publication page says about where a piece of evidence came
//! from: the run that made it, that run's inputs and code, whether its
//! environment was captured, and how exact the lineage is. Shared by the
//! WebView's publication workspace and the GPUI desktop's paper page.

use crate::{
    canonical_json_sha256, ArtifactCaptureTiming, EvidenceBinding, EvidenceSourceKind,
    LineageBasis, LineageConfidence, Store,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PublicationLineageSummary {
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

fn environment_captured(snapshot: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(snapshot) {
        Ok(serde_json::Value::Object(values)) => !values.is_empty(),
        Ok(serde_json::Value::Array(values)) => !values.is_empty(),
        Ok(value) => !value.is_null(),
        Err(_) => !snapshot.trim().is_empty(),
    }
}

pub async fn lineage_summary(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ArtifactMaterialization, ArtifactVersionDraft, EvidenceBindingDraft,
        EvidenceSelectionState, EvidenceVisibility, RunRecord,
    };

    #[test]
    fn an_environment_counts_as_captured_only_when_something_was_kept() {
        for (snapshot, captured) in [
            ("{}", false),
            ("[]", false),
            ("null", false),
            ("  ", false),
            (r#"{"python":"3.12"}"#, true),
            ("[1]", true),
            ("\"linux\"", true),
            ("not json", true),
        ] {
            assert_eq!(environment_captured(snapshot), captured, "{snapshot}");
        }
    }

    #[tokio::test]
    async fn a_result_and_a_run_are_traced_to_the_run_that_made_them() {
        let path = std::env::temp_dir().join(format!(
            "wisp_publication_lineage_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p", "p", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store
            .create_run(&RunRecord::new("run", "p", "local", "Fit", "command"))
            .await
            .unwrap();
        let version = store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: None,
                artifact_id: crate::logical_artifact_id("p", "figure:main"),
                project_id: "p".into(),
                root_frame_id: "f".into(),
                filename: "main.png".into(),
                content_type: "image/png".into(),
                storage_path: "snapshots/main.png".into(),
                logical_key: Some("figure:main".into()),
                size_bytes: Some(8),
                checksum: Some("a".repeat(64)),
                producing_run_id: Some("run".into()),
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        store
            .create_publication("paper", "p", "Paper", "")
            .await
            .unwrap();
        store
            .create_publication_revision("draft", "paper", None, "v1")
            .await
            .unwrap();
        let bind = |id: &str, source_kind, source_id: &str| EvidenceBindingDraft {
            id: id.into(),
            revision_id: "draft".into(),
            item_id: None,
            source_kind,
            source_id: source_id.into(),
            purpose: String::new(),
            supported_claim_item_id: None,
            selection_state: EvidenceSelectionState::Selected,
            visibility: EvidenceVisibility::Private,
        };
        let result = store
            .save_evidence_binding(&bind(
                "of-result",
                EvidenceSourceKind::ArtifactVersion,
                &version,
            ))
            .await
            .unwrap();
        let summary = lineage_summary(&store, &result).await.unwrap();
        assert_eq!(summary.source_label, "main.png");
        assert_eq!(summary.version_number, Some(1));
        assert_eq!(summary.checksum, Some("a".repeat(64)));
        assert_eq!(
            summary.capture_timing,
            Some(ArtifactCaptureTiming::AtCreation)
        );
        assert_eq!(summary.producing_run_id.as_deref(), Some("run"));
        assert_eq!(summary.producing_run_title.as_deref(), Some("Fit"));
        // No inputs or environment were recorded for the run.
        assert_eq!(summary.quality, LineageConfidence::Likely);
        assert!(!summary.environment_captured);
        assert_eq!(summary.run_input_count, 0);
        assert_eq!(summary.code_labels.len(), summary.code_snapshot_count);

        let run = store
            .save_evidence_binding(&bind("of-run", EvidenceSourceKind::Run, "run"))
            .await
            .unwrap();
        let summary = lineage_summary(&store, &run).await.unwrap();
        assert_eq!(summary.source_label, "Fit");
        assert_eq!(summary.exact_version_id, None);
        assert_eq!(summary.producing_run_id.as_deref(), Some("run"));
        let _ = std::fs::remove_file(&path);
    }
}
