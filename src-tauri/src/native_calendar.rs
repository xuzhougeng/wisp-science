//! Native research calendar. Reads the named projects' mainline history and
//! does not select a WebView window.

use wisp_dto::native_settings::Request;

pub(crate) async fn execute(
    store: &wisp_store::Store,
    request: &Request,
) -> Result<serde_json::Value, String> {
    if request
        .project_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
    {
        return Err("The research calendar does not use a project id".into());
    }
    if request.command != "native_research_calendar" {
        return Err("Unsupported native calendar command".into());
    }
    let input: wisp_dto::native_calendar::CalendarRequest =
        serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
    let rows = store
        .research_calendar(&input.project_ids, input.from, input.until)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_value(rows).map_err(|error| error.to_string())
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
            id: "calendar-1".into(),
            project_id: project_id.map(str::to_owned),
            command: "native_research_calendar".into(),
            args,
        }
    }

    struct Fixture {
        path: std::path::PathBuf,
        store: Store,
    }

    impl Fixture {
        async fn open() -> Self {
            let path = std::env::temp_dir().join(format!(
                "wisp-native-calendar-{}.sqlite",
                uuid::Uuid::new_v4()
            ));
            let store = Store::open(&path).await.unwrap();
            Self { path, store }
        }

        async fn journal(&self, id: &str) {
            self.store.create_project(id, id, "").await.unwrap();
            self.store
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
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[tokio::test]
    async fn calendar_reads_only_the_requested_mainlines() {
        let fixture = Fixture::open().await;
        fixture.journal("a").await;
        fixture.journal("hidden").await;
        let value = execute(
            &fixture.store,
            &request(
                None,
                json!({"project_ids": ["a", "missing", "a"], "from": 0, "until": 86400}),
            ),
        )
        .await
        .unwrap();
        let rows: Vec<wisp_dto::ResearchCalendarProject> = serde_json::from_value(value).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].project_id, "a");
        assert_eq!(rows[0].history.entries[0].title, "a finding");
        assert!(rows[0].error.is_none());
        assert_eq!(rows[1].project_id, "missing");
        assert!(rows[1].error.is_some());
        assert!(rows[1].history.entries.is_empty());
        assert!(!rows.iter().any(|row| row.project_id == "hidden"));
        let outside = execute(
            &fixture.store,
            &request(
                None,
                json!({"project_ids": ["a"], "from": 101, "until": 86400}),
            ),
        )
        .await
        .unwrap();
        let outside: Vec<wisp_dto::ResearchCalendarProject> =
            serde_json::from_value(outside).unwrap();
        assert!(outside[0].history.entries.is_empty());
        assert!(outside[0].error.is_none());
    }

    #[tokio::test]
    async fn a_project_id_or_inverted_range_reads_nothing() {
        let fixture = Fixture::open().await;
        fixture.journal("a").await;
        let rejected = execute(
            &fixture.store,
            &request(
                Some("hidden"),
                json!({"project_ids": ["a"], "from": 0, "until": 86400}),
            ),
        )
        .await
        .unwrap_err();
        assert!(rejected.contains("does not use a project id"));
        let range = execute(
            &fixture.store,
            &request(
                None,
                json!({"project_ids": ["a"], "from": 100, "until": 100}),
            ),
        )
        .await
        .unwrap_err();
        assert!(range.contains("32 days") || range.contains("date range"));
        let still = execute(
            &fixture.store,
            &request(
                None,
                json!({"project_ids": ["a"], "from": 0, "until": 86400}),
            ),
        )
        .await
        .unwrap();
        let still: Vec<wisp_dto::ResearchCalendarProject> = serde_json::from_value(still).unwrap();
        assert_eq!(still[0].history.entries[0].title, "a finding");
    }
}
