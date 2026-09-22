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
