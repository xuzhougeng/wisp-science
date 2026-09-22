//! Native library search and delete. The desktop host uses the app-global
//! library store. No WebView window is selected, and a project id is rejected
//! so the call cannot retarget one.

use wisp_dto::native_settings::Request;

pub(crate) async fn execute(
    library: &wisp_store::LibraryStore,
    request: &Request,
) -> Result<serde_json::Value, String> {
    if request
        .project_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
    {
        return Err("The library is app-global and does not use a project id".into());
    }
    match request.command.as_str() {
        "native_library_search" => {
            let input: wisp_dto::native_library::SearchRequest =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            let kind = kind_filter(input.kind)?;
            let rows = library
                .search_summaries(input.query.trim(), kind.as_deref())
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_value(rows).map_err(|error| error.to_string())
        }
        "native_library_delete" => {
            let input: wisp_dto::native_library::DeleteRequest =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            let id = input.id.trim();
            if id.is_empty() {
                return Err("A library item id is required".into());
            }
            let deleted = library
                .delete(id)
                .await
                .map_err(|error| error.to_string())?;
            Ok(serde_json::Value::Bool(deleted))
        }
        _ => Err("Unsupported native library command".into()),
    }
}

fn kind_filter(kind: Option<String>) -> Result<Option<String>, String> {
    let Some(kind) = kind else {
        return Ok(None);
    };
    let kind = kind.trim();
    if kind.is_empty() {
        return Ok(None);
    }
    if matches!(kind, "code" | "figure" | "text") {
        return Ok(Some(kind.to_owned()));
    }
    Err("Library kind must be code, figure, or text".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wisp_dto::native_settings::{Request, SCHEMA};
    use wisp_store::{LibraryStore, NewLibraryItem};

    fn request(project_id: Option<&str>, command: &str, args: serde_json::Value) -> Request {
        Request {
            schema: SCHEMA.into(),
            id: "library-1".into(),
            project_id: project_id.map(str::to_owned),
            command: command.into(),
            args,
        }
    }

    fn code_item(title: &str, code: &str) -> NewLibraryItem {
        NewLibraryItem {
            kind: "code".into(),
            title: title.into(),
            language: Some("python".into()),
            code: code.into(),
            content_type: None,
            content: None,
            source_project_id: "research-1".into(),
            source_project_name: "RNA-seq 研究".into(),
            source_session_id: "session-a".into(),
            source_session_title: "探索".into(),
            source_path: None,
        }
    }

    struct Fixture {
        root: std::path::PathBuf,
        library: LibraryStore,
    }

    impl Fixture {
        async fn open() -> Self {
            let root =
                std::env::temp_dir().join(format!("wisp-native-library-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let library = LibraryStore::open(&root.join("library.sqlite"))
                .await
                .unwrap();
            Self { root, library }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[tokio::test]
    async fn search_filters_the_library_store_and_delete_removes_one_item() {
        let fixture = Fixture::open().await;
        let code = fixture
            .library
            .insert(code_item("counts", "import pandas as pd"))
            .await
            .unwrap();
        fixture
            .library
            .insert(NewLibraryItem {
                kind: "figure".into(),
                title: "Plot".into(),
                language: None,
                code: String::new(),
                content_type: Some("image/png".into()),
                content: Some(b"png".to_vec()),
                source_project_id: "research-1".into(),
                source_project_name: "RNA-seq 研究".into(),
                source_session_id: "session-b".into(),
                source_session_title: "作图".into(),
                source_path: Some("figures/plot.png".into()),
            })
            .await
            .unwrap();
        fixture
            .library
            .insert_version(&code.id, Some("python".into()), "print(2)".into())
            .await
            .unwrap();
        assert_eq!(
            fixture.library.list_versions(&code.id).await.unwrap().len(),
            2
        );

        let matched = execute(
            &fixture.library,
            &request(None, "native_library_search", json!({"query": "pandas"})),
        )
        .await
        .unwrap();
        let matched: Vec<wisp_dto::LibraryItemSummary> = serde_json::from_value(matched).unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, code.id);
        assert!(matched[0].code_preview.contains("pandas"));
        assert_eq!(matched[0].source_project_id, "research-1");

        let figures = execute(
            &fixture.library,
            &request(
                None,
                "native_library_search",
                json!({"query": "", "kind": "figure"}),
            ),
        )
        .await
        .unwrap();
        let figures: Vec<wisp_dto::LibraryItemSummary> = serde_json::from_value(figures).unwrap();
        assert_eq!(figures.len(), 1);
        assert_eq!(figures[0].kind, "figure");
        assert_eq!(figures[0].source_session_id, "session-b");

        let rejected = execute(
            &fixture.library,
            &request(
                None,
                "native_library_search",
                json!({"query": "", "kind": "secret"}),
            ),
        )
        .await
        .unwrap_err();
        assert!(rejected.contains("code, figure, or text"));
        assert_eq!(fixture.library.list_summaries().await.unwrap().len(), 2);

        let deleted = execute(
            &fixture.library,
            &request(None, "native_library_delete", json!({"id": code.id})),
        )
        .await
        .unwrap();
        assert_eq!(deleted, true);
        assert!(fixture.library.get(&code.id).await.unwrap().is_none());
        assert!(fixture
            .library
            .list_versions(&code.id)
            .await
            .unwrap()
            .is_empty());
        let again = execute(
            &fixture.library,
            &request(None, "native_library_delete", json!({"id": code.id})),
        )
        .await
        .unwrap();
        assert_eq!(again, false);
        let remaining = execute(
            &fixture.library,
            &request(None, "native_library_search", json!({"query": ""})),
        )
        .await
        .unwrap();
        let remaining: Vec<wisp_dto::LibraryItemSummary> =
            serde_json::from_value(remaining).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].kind, "figure");
    }

    #[tokio::test]
    async fn a_project_id_is_rejected_and_deletes_nothing() {
        let fixture = Fixture::open().await;
        let code = fixture
            .library
            .insert(code_item("counts", "import pandas"))
            .await
            .unwrap();
        let error = execute(
            &fixture.library,
            &request(
                Some("other-project"),
                "native_library_delete",
                json!({"id": code.id}),
            ),
        )
        .await
        .unwrap_err();
        assert!(error.contains("does not use a project id"));
        assert!(fixture.library.get(&code.id).await.unwrap().is_some());
        let empty = execute(
            &fixture.library,
            &request(None, "native_library_delete", json!({"id": "  "})),
        )
        .await
        .unwrap_err();
        assert!(empty.contains("id is required"));
        assert_eq!(fixture.library.list_summaries().await.unwrap().len(), 1);
    }
}
