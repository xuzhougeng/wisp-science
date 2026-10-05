//! Native publication workspace. The desktop host reads and creates the paper
//! for one explicit project. No WebView window is selected.

use wisp_dto::native_publication::NativePublicationWorkspace;
use wisp_dto::native_settings::Request;

pub(crate) async fn execute(
    store: &wisp_store::Store,
    request: &Request,
) -> Result<serde_json::Value, String> {
    let Some(project_id) = request
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Err("A project id is required".into());
    };
    if store
        .get_project(project_id)
        .await
        .map_err(|e| e.to_string())?
        .is_none()
    {
        return Err("Project not found".into());
    }
    if matches!(
        request.command.as_str(),
        "native_publication_create" | "native_publication_mutate"
    ) {
        crate::exploration_commands::require_writable_scope(
            store,
            &wisp_store::StateScope::mainline(project_id.to_owned()),
        )
        .await?;
    }
    if request.command == "native_publication_sources" {
        let input: wisp_dto::native_publication::SourcesRequest =
            serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
        let page = store
            .publication_source_page(project_id, &input.kind, &input.query, input.offset)
            .await
            .map_err(|e| e.to_string())?;
        return serde_json::to_value(page).map_err(|e| e.to_string());
    }
    if request.command == "native_publication_mutate" {
        let input = serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
        return serde_json::to_value(mutate(store, project_id, input).await?)
            .map_err(|e| e.to_string());
    }
    let workspace = match request.command.as_str() {
        "native_publication_workspace" => {
            let input: wisp_dto::native_publication::WorkspaceRequest =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            crate::publication_commands::publication_workspace(
                store,
                project_id,
                input.publication_id.as_deref(),
                input.revision_id.as_deref(),
            )
            .await
            .map_err(|error| error.to_string())?
        }
        "native_publication_create" => {
            let input: wisp_dto::native_publication::CreateRequest =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            if input.title.trim().is_empty() || input.revision_label.trim().is_empty() {
                return Err("A paper title and revision label are required".into());
            }
            let revision = crate::publication_commands::create_publication(
                store,
                project_id,
                &crate::publication_commands::CreatePublicationInput::new(
                    input.title,
                    input.description,
                    input.revision_label,
                ),
            )
            .await
            .map_err(|error| error.to_string())?;
            crate::publication_commands::publication_workspace(
                store,
                project_id,
                None,
                Some(&revision.id),
            )
            .await
            .map_err(|error| error.to_string())?
        }
        _ => return Err("Unsupported native publication command".into()),
    };
    serde_json::to_value(summarize(workspace)?).map_err(|error| error.to_string())
}

async fn mutate(
    store: &wisp_store::Store,
    project: &str,
    input: wisp_dto::native_publication::MutationRequest,
) -> Result<wisp_dto::native_publication::MutationResult, String> {
    use serde_json::json;
    use wisp_dto::native_publication::{MutationResult, Operation};
    let selected = crate::publication_commands::publication_workspace(
        store,
        project,
        None,
        Some(&input.revision_id),
    )
    .await
    .map_err(|e| e.to_string())?;
    if selected.publication.as_ref().map(|p| p.project_id.as_str()) != Some(project)
        || selected.revision.as_ref().map(|r| r.id.as_str()) != Some(input.revision_id.as_str())
    {
        return Err("Publication revision does not belong to the selected project".into());
    }
    let mut revision_id = input.revision_id;
    let mut readiness = None;
    fn decode<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, String> {
        serde_json::from_value(value).map_err(|e| e.to_string())
    }
    match input.operation {
        Operation::SaveItem {
            id,
            parent_item_id,
            kind,
            title,
            content,
            ordinal,
        } => {
            store
                .save_publication_item(&wisp_store::PublicationItem {
                    id: id
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    revision_id: revision_id.clone(),
                    parent_item_id: parent_item_id.filter(|s| !s.is_empty()),
                    kind: decode(json!(kind))?,
                    title,
                    content,
                    ordinal,
                    metadata_json: "{}".into(),
                    created_at: 0,
                    updated_at: 0,
                })
                .await
                .map_err(|e| e.to_string())?;
        }
        Operation::BindEvidence {
            item_id,
            source_kind,
            source_id,
            purpose,
            supported_claim_item_id,
            selection_state,
            visibility,
        } => {
            let binding = decode(
                json!({ "revisionId": revision_id, "itemId": item_id, "sourceKind": source_kind,
                "sourceId": source_id, "purpose": purpose, "supportedClaimItemId": supported_claim_item_id,
                "selectionState": selection_state, "visibility": visibility }),
            )?;
            crate::publication_commands::bind_evidence(store, project, &binding)
                .await
                .map_err(|e| e.to_string())?;
        }
        Operation::UpdateBinding {
            binding_id,
            selection_state,
            visibility,
        } => {
            let binding = store
                .get_evidence_binding(&binding_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("Evidence binding not found")?;
            if binding.revision_id != revision_id {
                return Err("Evidence binding does not belong to the selected revision".into());
            }
            store
                .update_evidence_binding_selection(
                    &binding_id,
                    decode(json!(selection_state))?,
                    decode(json!(visibility))?,
                )
                .await
                .map_err(|e| e.to_string())?;
        }
        Operation::CloneRevision { label } => {
            revision_id = store
                .clone_publication_revision(&revision_id, &uuid::Uuid::new_v4().to_string(), &label)
                .await
                .map_err(|e| e.to_string())?
                .id;
        }
        Operation::SaveWaiver {
            finding_code,
            author,
            reason,
        } => {
            store
                .save_publication_waiver(&wisp_store::PublicationWaiver {
                    id: uuid::Uuid::new_v4().to_string(),
                    revision_id: revision_id.clone(),
                    finding_code,
                    author,
                    reason,
                    created_at: 0,
                })
                .await
                .map_err(|e| e.to_string())?;
        }
        operation @ (Operation::Check { .. } | Operation::Freeze { .. }) => {
            let check_only = matches!(operation, Operation::Check { .. });
            let policy = match operation {
                Operation::Check { policy } | Operation::Freeze { policy } => policy,
                _ => unreachable!(),
            };
            let result = crate::publication_freeze::prepare_or_freeze_publication(
                store,
                &revision_id,
                decode(serde_json::to_value(policy).map_err(|e| e.to_string())?)?,
                check_only,
            )
            .await?;
            readiness = Some(decode(
                serde_json::to_value(result.readiness).map_err(|e| e.to_string())?,
            )?);
        }
        Operation::Verify {
            source_run_id,
            comparisons,
        } => {
            let comparisons: Vec<crate::publication_reproduction::ReproductionComparisonRequest> =
                decode(serde_json::to_value(comparisons).map_err(|e| e.to_string())?)?;
            crate::publication_reproduction::verify_publication_revision(
                store,
                &revision_id,
                &source_run_id,
                &comparisons,
            )
            .await?;
        }
        Operation::BuildCapsule { destination } => {
            let path = std::path::Path::new(&destination);
            if !path.is_absolute()
                || path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("zip"))
                    != Some(true)
            {
                return Err("Choose an absolute ZIP destination".into());
            }
            crate::publication_capsule::build_publication_capsule_to(store, &revision_id, path)
                .await?;
        }
    }
    let workspace = crate::publication_commands::publication_workspace(
        store,
        project,
        None,
        Some(&revision_id),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(MutationResult {
        workspace: summarize(workspace)?,
        readiness,
    })
}

fn summarize(
    workspace: crate::publication_commands::PublicationWorkspace,
) -> Result<NativePublicationWorkspace, String> {
    // Deserialize through the shared wire contract rather than maintaining a
    // second field-by-field projection that silently drops workspace features.
    serde_json::from_value(serde_json::to_value(workspace).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wisp_dto::native_settings::{Request, SCHEMA};
    use wisp_store::Store;

    fn request(project_id: Option<&str>, command: &str, args: serde_json::Value) -> Request {
        Request {
            schema: SCHEMA.into(),
            id: "publication-1".into(),
            project_id: project_id.map(str::to_owned),
            command: command.into(),
            args,
        }
    }

    #[tokio::test]
    async fn editing_binding_cloning_and_checks_preserve_exact_ownership() {
        let store = Store::open(std::path::Path::new(":memory:")).await.unwrap();
        for project in ["a", "b"] {
            store.create_project(project, project, "").await.unwrap();
        }
        store
            .create_frame("frame", "a", "Wisp", "offline")
            .await
            .unwrap();
        store
            .save_artifact_version(&wisp_store::ArtifactVersionDraft {
                version_id: Some("version-a".into()),
                artifact_id: "artifact-a".into(),
                project_id: "a".into(),
                root_frame_id: "frame".into(),
                filename: "counts.csv".into(),
                content_type: "text/csv".into(),
                storage_path: "counts.csv".into(),
                logical_key: None,
                size_bytes: Some(3),
                checksum: Some("a".repeat(64)),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: wisp_store::ArtifactMaterialization::Snapshot,
                capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        let created = execute(
            &store,
            &request(
                Some("a"),
                "native_publication_create",
                json!({"title":"Paper", "revision_label":"v1"}),
            ),
        )
        .await
        .unwrap();
        let revision = created["revision"]["id"].as_str().unwrap();
        let save = json!({"revision_id":revision,"operation":{"action":"save_item","id":"claim-a","kind":"claim","title":"Original claim","content":"line one\n中文😀","ordinal":0}});
        assert!(execute(
            &store,
            &request(Some("b"), "native_publication_mutate", save.clone())
        )
        .await
        .is_err());
        let saved = execute(
            &store,
            &request(Some("a"), "native_publication_mutate", save.clone()),
        )
        .await
        .unwrap();
        assert_eq!(
            saved["workspace"]["items"][0]["content"],
            "line one\n中文😀"
        );
        let mut edit = save;
        edit["operation"]["title"] = json!("Edited claim");
        let edited = execute(
            &store,
            &request(Some("a"), "native_publication_mutate", edit),
        )
        .await
        .unwrap();
        assert_eq!(edited["workspace"]["items"].as_array().unwrap().len(), 1);
        let sources = execute(
            &store,
            &request(
                Some("a"),
                "native_publication_sources",
                json!({"kind":"files","query":"counts","offset":0}),
            ),
        )
        .await
        .unwrap();
        assert_eq!(sources["sources"][0]["id"], "version-a");
        let other_sources = execute(
            &store,
            &request(
                Some("b"),
                "native_publication_sources",
                json!({"kind":"files","query":"counts","offset":0}),
            ),
        )
        .await
        .unwrap();
        assert!(other_sources["sources"].as_array().unwrap().is_empty());
        let bound = execute(&store, &request(Some("a"), "native_publication_mutate", json!({"revision_id":revision,"operation":{
            "action":"bind_evidence","item_id":"claim-a","source_kind":"artifact_version","source_id":"version-a","purpose":"claim evidence","selection_state":"selected","visibility":"private"
        }}))).await.unwrap();
        assert_eq!(
            bound["workspace"]["lineage"][0]["exact_version_id"],
            "version-a"
        );
        let binding = bound["workspace"]["bindings"][0]["id"].as_str().unwrap();
        let cloned = execute(&store, &request(Some("a"), "native_publication_mutate", json!({"revision_id":revision,"operation":{"action":"clone_revision","label":"v2"}}))).await.unwrap();
        let next = cloned["workspace"]["revision"]["id"].as_str().unwrap();
        assert_ne!(revision, next);
        assert_eq!(
            cloned["workspace"]["revisions"].as_array().unwrap().len(),
            2
        );
        assert!(execute(&store, &request(Some("a"), "native_publication_mutate", json!({"revision_id":next,"operation":{"action":"update_binding","binding_id":binding,"selection_state":"rejected","visibility":"private"}}))).await.is_err());
        let updated = execute(&store, &request(Some("a"), "native_publication_mutate", json!({"revision_id":revision,"operation":{"action":"update_binding","binding_id":binding,"selection_state":"rejected","visibility":"private"}}))).await.unwrap();
        assert_eq!(
            updated["workspace"]["bindings"][0]["selection_state"],
            "rejected"
        );
        // Reject a foreign revision before any file snapshot, execution, or export.
        for operation in [
            json!({"action":"check","policy":{"target_visibility":"private","phi_pii_reviewed":false,"redistribution_reviewed":false,"snapshot_restricted_bytes":false}}),
            json!({"action":"freeze","policy":{"target_visibility":"private","phi_pii_reviewed":false,"redistribution_reviewed":false,"snapshot_restricted_bytes":false}}),
            json!({"action":"verify","source_run_id":"unavailable"}),
            json!({"action":"build_capsule","destination":"relative.zip"}),
            json!({"action":"save_waiver","finding_code":"anything","author":"reviewer","reason":"reason"}),
        ] {
            assert!(execute(
                &store,
                &request(
                    Some("b"),
                    "native_publication_mutate",
                    json!({"revision_id":revision,"operation":operation})
                )
            )
            .await
            .is_err());
        }
        let selected = execute(
            &store,
            &request(
                Some("a"),
                "native_publication_workspace",
                json!({"revision_id":revision}),
            ),
        )
        .await
        .unwrap();
        assert_eq!(selected["revision"]["id"], revision);
        assert_eq!(selected["items"][0]["title"], "Edited claim");
        assert_eq!(selected["bindings"][0]["source_id"], "version-a");
    }

    #[tokio::test]
    async fn create_and_read_stay_inside_the_named_project() {
        let path = std::env::temp_dir().join(format!(
            "wisp-native-publication-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store.create_project("a", "A", "").await.unwrap();
        store.create_project("b", "B", "").await.unwrap();
        let created = execute(
            &store,
            &request(
                Some("a"),
                "native_publication_create",
                json!({"title": "RNA-seq paper", "description": "counts", "revision_label": "v1"}),
            ),
        )
        .await
        .unwrap();
        let created: NativePublicationWorkspace = serde_json::from_value(created).unwrap();
        assert_eq!(created.publications.len(), 1);
        assert_eq!(created.publication.as_ref().unwrap().project_id, "a");
        assert_eq!(created.publication.as_ref().unwrap().title, "RNA-seq paper");
        assert_eq!(created.revision.as_ref().unwrap().label, "v1");
        assert_eq!(created.revision.as_ref().unwrap().state, "draft");
        let other = execute(
            &store,
            &request(Some("b"), "native_publication_workspace", json!({})),
        )
        .await
        .unwrap();
        let other: NativePublicationWorkspace = serde_json::from_value(other).unwrap();
        assert!(other.publications.is_empty());
        let reread = execute(
            &store,
            &request(Some("a"), "native_publication_workspace", json!({})),
        )
        .await
        .unwrap();
        let reread: NativePublicationWorkspace = serde_json::from_value(reread).unwrap();
        assert_eq!(
            reread.publication.as_ref().unwrap().id,
            created.publication.unwrap().id
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn missing_project_id_or_blank_title_creates_nothing() {
        let path = std::env::temp_dir().join(format!(
            "wisp-native-publication-empty-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store.create_project("a", "A", "").await.unwrap();
        let missing = execute(
            &store,
            &request(
                None,
                "native_publication_create",
                json!({"title": "Paper", "revision_label": "v1"}),
            ),
        )
        .await
        .unwrap_err();
        assert!(missing.contains("project id is required"));
        let blank = execute(
            &store,
            &request(
                Some("a"),
                "native_publication_create",
                json!({"title": "  ", "revision_label": "v1"}),
            ),
        )
        .await
        .unwrap_err();
        assert!(blank.contains("title and revision label"));
        let empty = execute(
            &store,
            &request(Some("a"), "native_publication_workspace", json!({})),
        )
        .await
        .unwrap();
        let empty: NativePublicationWorkspace = serde_json::from_value(empty).unwrap();
        assert!(empty.publications.is_empty());
        let _ = std::fs::remove_file(path);
    }
}
