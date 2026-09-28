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
