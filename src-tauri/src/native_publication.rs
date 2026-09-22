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
    serde_json::to_value(summarize(workspace)).map_err(|error| error.to_string())
}

fn summarize(
    workspace: crate::publication_commands::PublicationWorkspace,
) -> NativePublicationWorkspace {
    NativePublicationWorkspace {
        publications: workspace.publications.iter().map(publication).collect(),
        publication: workspace.publication.as_ref().map(publication),
        revision: workspace.revision.as_ref().map(|revision| {
            wisp_dto::native_publication::NativePublicationRevision {
                id: revision.id.clone(),
                publication_id: revision.publication_id.clone(),
                revision_number: revision.revision_number,
                label: revision.label.clone(),
                state: revision.state.as_str().to_string(),
            }
        }),
        items: workspace
            .items
            .iter()
            .map(|item| wisp_dto::native_publication::NativePublicationItem {
                id: item.id.clone(),
                title: item.title.clone(),
                kind: item.kind.as_str().to_string(),
                ordinal: item.ordinal,
            })
            .collect(),
    }
}

fn publication(item: &wisp_store::Publication) -> wisp_dto::native_publication::NativePublication {
    wisp_dto::native_publication::NativePublication {
        id: item.id.clone(),
        project_id: item.project_id.clone(),
        title: item.title.clone(),
        description: item.description.clone(),
    }
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
