//! Native search reads persisted project/session/artifact state without selecting
//! a WebView. Privacy is read before the search and rechecked before returning.
use wisp_dto::{native_search::*, native_settings::Request};

pub(crate) async fn execute(
    store: &wisp_store::Store,
    request: &Request,
) -> Result<serde_json::Value, String> {
    if request.command != COMMANDS[0] {
        return Err("Unsupported native search command".into());
    }
    let input: SearchRequest =
        serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
    if input.query.len() > 512
        || request
            .project_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.trim() != id)
    {
        return Err("Invalid search query or project preference".into());
    }
    let privacy = crate::privacy_mode::load(store).await?;
    let visible =
        |id: &str| !privacy.active || !privacy.project_ids.iter().any(|hidden| hidden == id);
    let query = input.query.trim();
    let q = query.to_lowercase();
    let preferred = request.project_id.as_deref();
    let mut projects = store.list_projects().await.map_err(|e| e.to_string())?;
    projects.retain(|p| visible(&p.0));
    projects.sort_by_key(|p| (Some(p.0.as_str()) != preferred, p.1.clone()));
    let mut items: Vec<_> = projects
        .iter()
        .filter(|p| {
            q.split_whitespace().all(|word| {
                format!("{} {} {}", p.1, p.2, p.6)
                    .to_lowercase()
                    .contains(word)
            })
        })
        .take(20)
        .map(|p| SearchItem {
            kind: SearchKind::Project,
            id: p.0.clone(),
            project_id: p.0.clone(),
            project_name: p.1.clone(),
            title: p.1.clone(),
            detail: p.2.clone(),
            session_id: None,
        })
        .collect();
    // Search each visible owner before limiting, so hidden projects cannot crowd
    // visible matches out of the result budget.
    let mut artifacts = Vec::new();
    for project in &projects {
        artifacts.extend(
            store
                .search_artifacts(Some(&project.0), query, 12, None)
                .await
                .map_err(|e| e.to_string())?,
        );
    }
    artifacts.sort_by_key(|a| {
        (
            Some(a.project_id.as_str()) != preferred,
            std::cmp::Reverse(a.ts),
            a.id.clone(),
        )
    });
    items.extend(artifacts.into_iter().take(12).map(|a| SearchItem {
        kind: SearchKind::Artifact,
        id: a.id,
        project_id: a.project_id,
        project_name: a.project_name.clone(),
        title: a.name,
        detail: format!("{} · {}", a.project_name, a.session_title),
        session_id: Some(a.session_id),
    }));
    let sessions = store
        .search_sessions_excluding_projects(
            None,
            query,
            12,
            None,
            preferred,
            if privacy.active {
                &privacy.project_ids
            } else {
                &[]
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    items.extend(sessions.into_iter().map(|s| SearchItem {
        kind: SearchKind::Session,
        id: s.id.clone(),
        project_id: s.project_id,
        project_name: s.project_name.clone(),
        title: s.title,
        detail: s.project_name,
        session_id: Some(s.id),
    }));
    let final_privacy = crate::privacy_mode::load(store).await?;
    items.retain(|item| {
        !final_privacy.active || !final_privacy.project_ids.contains(&item.project_id)
    });
    serde_json::to_value(SearchResponse {
        schema: SCHEMA.into(),
        query: input.query,
        preferred_project_id: request.project_id.clone(),
        items,
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wisp_llm::Message;

    #[tokio::test]
    async fn native_search_finds_body_hits_and_filters_hidden_owners() {
        let root = tempfile::tempdir().unwrap();
        let store = wisp_store::Store::open(&root.path().join("search.sqlite"))
            .await
            .unwrap();
        for project in ["current", "other", "hidden"] {
            store.create_project(project, project, "").await.unwrap();
            store
                .create_frame(project, project, "OPERON", "m")
                .await
                .unwrap();
            store
                .append_message(project, 1, &Message::user("ordinary title"))
                .await
                .unwrap();
            store
                .append_message(project, 2, &Message::assistant("needle in historical body"))
                .await
                .unwrap();
            store
                .save_artifact(
                    &format!("a-{project}"),
                    project,
                    project,
                    "needle.csv",
                    "text/csv",
                    "/synthetic/needle.csv",
                )
                .await
                .unwrap();
        }
        crate::privacy_mode::save(&store, true, &["hidden".into()])
            .await
            .unwrap();
        let mut request = Request {
            schema: wisp_dto::native_settings::SCHEMA.into(),
            id: "search".into(),
            command: COMMANDS[0].into(),
            project_id: Some("current".into()),
            args: json!({"query":"needle"}),
        };
        let value = execute(&store, &request).await.unwrap();
        let result: SearchResponse = serde_json::from_value(value).unwrap();
        assert_eq!(result.preferred_project_id.as_deref(), Some("current"));
        assert!(result.items.iter().all(|i| i.project_id != "hidden"));
        let sessions: Vec<_> = result
            .items
            .iter()
            .filter(|i| i.kind == SearchKind::Session)
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(sessions, ["current", "other"]);
        assert_eq!(
            result
                .items
                .iter()
                .filter(|i| i.kind == SearchKind::Artifact)
                .count(),
            2
        );
        request.args = json!({"query":"", "active_project":"hidden"});
        assert!(execute(&store, &request).await.is_err());
        request.args = json!({"query":"x".repeat(513)});
        assert!(execute(&store, &request).await.is_err());
        request.args = json!({"query":"hidden"});
        assert!(
            serde_json::from_value::<SearchResponse>(execute(&store, &request).await.unwrap())
                .unwrap()
                .items
                .is_empty()
        );
        drop(store);
    }
}
