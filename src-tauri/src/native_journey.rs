//! Native research journey for one explicit project. No WebView window is selected.

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
    if request.command == "native_research_journey_graph" {
        if request.args.as_object().is_none_or(|args| !args.is_empty()) {
            return Err("Graph reads do not accept arguments".into());
        }
        return serde_json::to_value(
            store
                .research_graph_in_scope(&wisp_store::StateScope::mainline(project_id))
                .await
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string());
    }
    if matches!(
        request.command.as_str(),
        "native_research_journey_add" | "native_research_journey_recap"
    ) {
        if store
            .get_project(project_id)
            .await
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Err("Project is unavailable".into());
        }
        if request.command == "native_research_journey_add" {
            let input: wisp_dto::native_journey::AddEntryRequest =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            let id = store
                .add_research_journal_entry(
                    &wisp_store::StateScope::mainline(project_id),
                    &input.input,
                )
                .await
                .map_err(|error| error.to_string())?;
            return Ok(serde_json::Value::String(id));
        }
        let input: wisp_dto::native_journey::EditRecapRequest =
            serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
        return serde_json::to_value(
            store
                .update_research_recap(project_id, &input.edit)
                .await
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string());
    }
    if request.command == "native_research_journey_run" {
        let input: wisp_dto::native_journey::RunRequest =
            serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
        if !store
            .run_visible_in_scope(&input.run_id, &wisp_store::StateScope::mainline(project_id))
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("Run is unavailable in this project scope".into());
        }
        let run = store
            .get_run(&input.run_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("Run is unavailable")?;
        return serde_json::to_value(run).map_err(|error| error.to_string());
    }
    if request.command == "native_research_journey_artifact" {
        let input: wisp_dto::native_journey::ArtifactRequest =
            serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
        // This scope check precedes both metadata and filesystem reads, and
        // resolves the immutable version rather than the current artifact head.
        let source = store
            .research_journey_source(
                &wisp_store::StateScope::mainline(project_id),
                &input.version_id,
            )
            .await
            .map_err(|error| error.to_string())?;
        let context = store
            .get_artifact_version_context(&input.version_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("Artifact version is unavailable")?;
        let artifact = store
            .get_artifact_detail(&context.version.artifact_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("Artifact is unavailable")?;
        let root = std::path::PathBuf::from(artifact.project_root);
        let path = context.version.storage_path.clone();
        let content = tokio::task::spawn_blocking(move || {
            crate::file_browser::read_file_at(&root, path, Some(2 * 1024 * 1024))
        })
        .await
        .map_err(|error| error.to_string())?;
        let (value, content_error) = match content {
            Ok(content) => (
                serde_json::to_value(content).map_err(|error| error.to_string())?,
                None,
            ),
            Err(error) => (serde_json::Value::Null, Some(error)),
        };
        let page = wisp_dto::native_journey::JourneyArtifact {
            version_id: context.version.id,
            filename: context.filename,
            version_number: context.version.version_number,
            source,
            text: value["text"].as_str().map(str::to_owned),
            mime: value["mime"]
                .as_str()
                .unwrap_or(&context.version.content_type)
                .to_owned(),
            base64: value["base64"].as_str().map(str::to_owned),
            truncated: value["truncated"].as_bool().unwrap_or(false),
            content_error,
        };
        return serde_json::to_value(page).map_err(|error| error.to_string());
    }
    if request.command != "native_research_journey" {
        return Err("Unsupported native journey command".into());
    }
    let input: wisp_dto::native_journey::JourneyRequest =
        serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
    let page = store
        .research_journey(
            &wisp_store::StateScope::mainline(project_id),
            input.from,
            input.until,
        )
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_value(page).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wisp_dto::native_settings::{Request, SCHEMA};
    use wisp_dto::ResearchJournalInput;
    use wisp_store::{StateScope, Store};

    fn request(project_id: Option<&str>, args: serde_json::Value) -> Request {
        Request {
            schema: SCHEMA.into(),
            id: "journey-1".into(),
            project_id: project_id.map(str::to_owned),
            command: "native_research_journey".into(),
            args,
        }
    }

    async fn journal(store: &Store, id: &str) {
        store.create_project(id, id, "").await.unwrap();
        store
            .add_research_journal_entry(
                &StateScope::mainline(id),
                &ResearchJournalInput {
                    title: format!("{id} finding"),
                    body: "Evidence".into(),
                    category: "finding".into(),
                    occurred_at: 100,
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn journey_run_read_is_project_scoped_and_strictly_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("store.sqlite"))
            .await
            .unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store.create_project("other", "Other", "").await.unwrap();
        store
            .create_run(&wisp_store::RunRecord::new(
                "r", "p", "local", "Run", "command",
            ))
            .await
            .unwrap();
        let mut read = request(Some("p"), json!({"run_id": "r"}));
        read.command = "native_research_journey_run".into();
        assert_eq!(execute(&store, &read).await.unwrap()["id"], "r");
        read.project_id = Some("other".into());
        assert!(execute(&store, &read).await.is_err());
        read.project_id = None;
        assert!(execute(&store, &read).await.is_err());
        read.project_id = Some("p".into());
        read.args = json!({"run_id": "missing"});
        assert!(execute(&store, &read).await.is_err());
        read.args = json!({"run_id": "r", "action": "cancel"});
        assert!(execute(&store, &read).await.is_err());
    }

    #[tokio::test]
    async fn artifact_reads_the_requested_version_and_rejects_other_projects() {
        let root =
            std::env::temp_dir().join(format!("native-journey-version-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("v1.txt"), "original evidence").unwrap();
        std::fs::write(root.join("v2.txt"), "revised evidence").unwrap();
        let store = Store::open(&root.join("test.sqlite")).await.unwrap();
        store
            .create_project("p", "Project", root.to_str().unwrap())
            .await
            .unwrap();
        store.create_project("other", "Other", "").await.unwrap();
        store
            .create_frame("f", "p", "agent", "model")
            .await
            .unwrap();
        let mut ids = Vec::new();
        for path in ["v1.txt", "v2.txt"] {
            ids.push(
                store
                    .save_artifact_version(&wisp_store::ArtifactVersionDraft {
                        version_id: None,
                        artifact_id: "a".into(),
                        project_id: "p".into(),
                        root_frame_id: "f".into(),
                        filename: "result.txt".into(),
                        content_type: "text/plain".into(),
                        storage_path: path.into(),
                        logical_key: None,
                        size_bytes: None,
                        checksum: None,
                        producing_run_id: None,
                        env_snapshot_hash: None,
                        materialization: wisp_store::ArtifactMaterialization::Snapshot,
                        capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
                    })
                    .await
                    .unwrap(),
            );
        }
        let mut read = request(Some("p"), json!({"version_id": ids[0]}));
        read.command = "native_research_journey_artifact".into();
        let result = execute(&store, &read).await.unwrap();
        assert_eq!(result["version_id"], ids[0]);
        assert_eq!(result["version_number"], 1);
        assert_eq!(result["text"], "original evidence");
        read.project_id = Some("other".into());
        assert!(execute(&store, &read).await.unwrap_err().contains("scope"));
        read.project_id = Some("p".into());
        std::fs::remove_file(root.join("v1.txt")).unwrap();
        let missing = execute(&store, &read).await.unwrap();
        assert_eq!(missing["version_id"], ids[0]);
        assert!(missing["text"].is_null());
        assert!(missing["content_error"].is_string());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn native_journal_recap_and_graph_remain_in_the_named_project() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("store.sqlite"))
            .await
            .unwrap();
        journal(&store, "a").await;
        journal(&store, "b").await;
        let mut write = request(
            Some("a"),
            json!({"input": {"title": "新发现 🧬", "body": "Evidence", "category": "finding", "occurred_at": 200}}),
        );
        write.command = "native_research_journey_add".into();
        let id = execute(&store, &write).await.unwrap();
        assert!(id.as_str().is_some_and(|id| !id.is_empty()));
        assert_eq!(
            store
                .research_journey(&StateScope::mainline("a"), 0, 86400)
                .await
                .unwrap()
                .entries
                .len(),
            2
        );
        assert_eq!(
            store
                .research_journey(&StateScope::mainline("b"), 0, 86400)
                .await
                .unwrap()
                .entries
                .len(),
            1
        );
        write.project_id = Some("missing".into());
        assert!(execute(&store, &write)
            .await
            .unwrap_err()
            .contains("unavailable"));
        write.project_id = Some("a".into());
        write.args["unexpected"] = json!(true);
        assert!(execute(&store, &write).await.is_err());
        let recap = store
            .save_research_recap(
                "a",
                &wisp_dto::ResearchRecap {
                    day_start: 0,
                    status: "draft".into(),
                    headline: "Day".into(),
                    done: vec![wisp_dto::ResearchRecapItem {
                        text: "Original".into(),
                        refs: vec![0],
                    }],
                    sources: vec![wisp_dto::ResearchRecapSource {
                        kind: "record".into(),
                        id: id.as_str().unwrap().into(),
                        title: "Source".into(),
                    }],
                    ..Default::default()
                },
                false,
            )
            .await
            .unwrap()
            .unwrap();
        write.command = "native_research_journey_recap".into();
        write.args = json!({"edit": {"id": recap.id, "status": "confirmed", "headline": "Reviewed", "done": [{"text": "Rewritten", "refs": [0]}], "findings": [], "issues": [], "next": []}});
        write.project_id = Some("b".into());
        assert!(execute(&store, &write).await.is_err());
        write.project_id = Some("a".into());
        let saved: wisp_dto::ResearchRecap =
            serde_json::from_value(execute(&store, &write).await.unwrap()).unwrap();
        assert_eq!(saved.status, "confirmed");
        assert!(saved.done[0].refs.is_empty());
        assert_eq!(saved.sources, recap.sources);
        for project in ["a", "b"] {
            let node = wisp_store::ResearchNode::new(
                format!("node-{project}"),
                project,
                wisp_store::ResearchNodeKind::Decision,
                format!("Decision {project}"),
            )
            .unwrap();
            store
                .save_research_node_in_scope(&node, &StateScope::mainline(project))
                .await
                .unwrap();
        }
        write.command = "native_research_journey_graph".into();
        write.args = json!({});
        let graph = execute(&store, &write).await.unwrap();
        assert!(graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["id"] == "node-a"));
        assert!(!graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["id"] == "node-b"));
        write.args = json!({"project_id": "b"});
        assert!(execute(&store, &write).await.is_err());
    }

    #[tokio::test]
    async fn journey_reads_only_the_named_project() {
        let path = std::env::temp_dir().join(format!(
            "wisp-native-journey-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        journal(&store, "a").await;
        journal(&store, "b").await;
        let value = execute(
            &store,
            &request(Some("a"), json!({"from": 0, "until": 86400})),
        )
        .await
        .unwrap();
        let page: wisp_dto::ResearchJourney = serde_json::from_value(value).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].title, "a finding");
        let other = execute(
            &store,
            &request(Some("b"), json!({"from": 0, "until": 86400})),
        )
        .await
        .unwrap();
        let other: wisp_dto::ResearchJourney = serde_json::from_value(other).unwrap();
        assert_eq!(other.entries[0].title, "b finding");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn missing_project_id_or_bad_range_reads_nothing() {
        let path = std::env::temp_dir().join(format!(
            "wisp-native-journey-empty-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        journal(&store, "a").await;
        let missing = execute(&store, &request(None, json!({"from": 0, "until": 86400})))
            .await
            .unwrap_err();
        assert!(missing.contains("project id is required"));
        let range = execute(
            &store,
            &request(Some("a"), json!({"from": 100, "until": 100})),
        )
        .await
        .unwrap_err();
        assert!(range.contains("date range"));
        let still = execute(
            &store,
            &request(Some("a"), json!({"from": 0, "until": 86400})),
        )
        .await
        .unwrap();
        let still: wisp_dto::ResearchJourney = serde_json::from_value(still).unwrap();
        assert_eq!(still.entries[0].title, "a finding");
        let _ = std::fs::remove_file(path);
    }
}
