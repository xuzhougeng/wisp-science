//! Native publication workspace. Reads and creates belong to one explicit project.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-publication.v1";

pub const COMMANDS: &[&str] = &[
    "native_publication_workspace",
    "native_publication_create",
    "native_publication_sources",
    "native_publication_mutate",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourcesRequest {
    pub kind: String,
    pub query: String,
    pub offset: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MutationRequest {
    pub revision_id: String,
    pub operation: Operation,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    SaveItem {
        id: Option<String>,
        parent_item_id: Option<String>,
        kind: String,
        title: String,
        content: String,
        ordinal: i64,
    },
    BindEvidence {
        item_id: Option<String>,
        source_kind: String,
        source_id: String,
        purpose: String,
        supported_claim_item_id: Option<String>,
        selection_state: String,
        visibility: String,
    },
    UpdateBinding {
        binding_id: String,
        selection_state: String,
        visibility: String,
    },
    CloneRevision {
        label: String,
    },
    SaveWaiver {
        finding_code: String,
        author: String,
        reason: String,
    },
    Check {
        policy: FreezePolicy,
    },
    Freeze {
        policy: FreezePolicy,
    },
    Verify {
        source_run_id: String,
        #[serde(default)]
        comparisons: Vec<Comparison>,
    },
    BuildCapsule {
        destination: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FreezePolicy {
    pub target_visibility: String,
    pub phi_pii_reviewed: bool,
    pub redistribution_reviewed: bool,
    pub snapshot_restricted_bytes: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Comparison {
    pub output_id: String,
    pub comparator: String,
    pub absolute_tolerance: Option<f64>,
    pub relative_tolerance: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MutationResult {
    pub workspace: NativePublicationWorkspace,
    pub readiness: Option<crate::PublicationReadinessInfo>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRequest {
    #[serde(default)]
    pub publication_id: Option<String>,
    #[serde(default)]
    pub revision_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub revision_label: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NativePublication {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NativePublicationRevision {
    pub id: String,
    pub publication_id: String,
    pub revision_number: i64,
    pub label: String,
    pub state: String,
    #[serde(default)]
    pub capability_level: String,
    pub parent_revision_id: Option<String>,
    pub manifest_sha256: Option<String>,
    pub frozen_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NativePublicationItem {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub ordinal: i64,
    #[serde(default)]
    pub revision_id: String,
    #[serde(default)]
    pub parent_item_id: Option<String>,
    #[serde(default)]
    pub content: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct NativePublicationWorkspace {
    pub publications: Vec<NativePublication>,
    pub publication: Option<NativePublication>,
    pub revision: Option<NativePublicationRevision>,
    pub items: Vec<NativePublicationItem>,
    #[serde(default)]
    pub revisions: Vec<NativePublicationRevision>,
    #[serde(default)]
    pub bindings: Vec<crate::PublicationEvidenceBinding>,
    #[serde(default)]
    pub item_links: Vec<crate::PublicationItemLinkInfo>,
    #[serde(default)]
    pub reviews: Vec<crate::PublicationEvidenceReview>,
    #[serde(default)]
    pub supersessions: Vec<crate::PublicationEvidenceSupersession>,
    #[serde(default)]
    pub waivers: Vec<crate::PublicationWaiverInfo>,
    pub readiness: Option<crate::PublicationReadinessInfo>,
    #[serde(default)]
    pub drift: Vec<crate::PublicationEvidenceDriftInfo>,
    #[serde(default)]
    pub lineage: Vec<crate::PublicationLineageInfo>,
    #[serde(default)]
    pub capsule_builds: Vec<crate::CapsuleBuildInfo>,
    pub effective_capability_level: Option<String>,
    #[serde(default)]
    pub reproduction_runs: Vec<crate::ReproductionRunInfo>,
    #[serde(default)]
    pub reproduction_results: Vec<crate::ReproductionResultInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_fixture_preserves_revision_item_and_source_identity() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-publication/v1/workspace-evidence.json"
        ))
        .unwrap();
        let page: NativePublicationWorkspace =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        let revision = page.revision.as_ref().unwrap();
        assert_eq!(page.revisions, vec![revision.clone()]);
        assert!(page
            .items
            .iter()
            .all(|item| item.revision_id == revision.id));
        let claim = &page.items[1];
        assert_eq!(
            claim.parent_item_id.as_deref(),
            Some(page.items[0].id.as_str())
        );
        assert!(!claim.content.is_empty());
        let binding = &page.bindings[0];
        assert_eq!(binding.revision_id, revision.id);
        assert_eq!(binding.item_id.as_deref(), Some(claim.id.as_str()));
        assert_eq!(binding.source_id, "artifact-version-17");
        let snapshot: serde_json::Value =
            serde_json::from_str(&binding.source_snapshot_json).unwrap();
        assert_eq!(snapshot["version"], 17);
        assert_eq!(serde_json::to_value(&page).unwrap(), fixture["result"]);
    }

    #[test]
    fn publication_fixture_requires_a_project_id() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-publication/v1/workspace.json"
        ))
        .unwrap();
        assert_eq!(fixture["schema"], SCHEMA);
        assert_eq!(fixture["project_id"], "research-1");
        assert!(COMMANDS.contains(&fixture["command"].as_str().unwrap()));
        let request: WorkspaceRequest = serde_json::from_value(fixture["args"].clone()).unwrap();
        assert!(request.publication_id.is_none());
        let page: NativePublicationWorkspace =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(page.publications[0].title, "RNA-seq paper");
        assert_eq!(page.publication.as_ref().unwrap().project_id, "research-1");
        assert_eq!(page.revision.as_ref().unwrap().label, "v1");
        assert_eq!(page.items[0].kind, "claim");
        let mut extra = fixture["args"].clone();
        extra["active_window"] = serde_json::json!("main");
        assert!(serde_json::from_value::<WorkspaceRequest>(extra).is_err());
    }
}
