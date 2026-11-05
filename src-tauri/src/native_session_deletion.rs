//! Explicit-project native previews reuse the WebView's family and file plans.
use crate::native_conversations::{call, require_owner};
use crate::native_settings::Broker;
use serde_json::{json, Value};
use tauri::Manager;
use wisp_dto::native_projects::{SessionDeletePreview, SessionDeletePreviewRequest};

fn validate_selection(ids: &[String]) -> Result<(), String> {
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    if ids.is_empty()
        || ids.len() > 1000
        || unique.len() != ids.len()
        || ids.iter().any(|id| id.trim().is_empty())
    {
        return Err("Choose between 1 and 1000 distinct conversations".into());
    }
    Ok(())
}

fn child_choice(children: &wisp_dto::SessionChildrenPreview, selected: bool) -> Option<bool> {
    if !children.is_empty() {
        Some(selected)
    } else if !children.nested.is_empty() {
        Some(true)
    } else {
        None
    }
}

pub(crate) async fn preview(
    broker: &Broker,
    request: &wisp_dto::native_settings::Request,
) -> Result<Value, String> {
    let project = request
        .project_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or("A project is required")?;
    let args: SessionDeletePreviewRequest =
        serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
    validate_selection(&args.session_ids)?;
    let state = broker.app.state::<crate::AppState>();
    let _activity = state.begin_project_activity(project)?;
    for id in &args.session_ids {
        require_owner(&state.store, project, id).await?;
    }
    let children: wisp_dto::SessionChildrenPreview = serde_json::from_value(
        call(
            broker,
            project,
            "preview_session_children",
            json!({"ids":args.session_ids}),
        )
        .await?,
    )
    .map_err(|e| e.to_string())?;
    let include_children = child_choice(&children, args.include_children);
    let artifacts = async {
        if !args.include_artifacts {
            return Ok(Vec::new());
        }
        if include_children == Some(true) {
            return serde_json::from_value(
                call(
                    broker,
                    project,
                    "preview_session_family_artifacts",
                    json!({"ids":args.session_ids}),
                )
                .await?,
            )
            .map_err(|e| e.to_string());
        }
        let mut families = Vec::new();
        for id in &args.session_ids {
            let preview = serde_json::from_value(
                call(
                    broker,
                    project,
                    "preview_session_artifacts",
                    json!({"id":id,"targetProjectId":null}),
                )
                .await?,
            )
            .map_err(|e| e.to_string())?;
            families.push(wisp_dto::SessionFamilyArtifactPreview {
                id: id.clone(),
                preview,
                children: Vec::new(),
            });
        }
        Ok::<_, String>(families)
    }
    .await;
    let (artifacts, artifact_error) = match artifacts {
        Ok(value) => (value, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    serde_json::to_value(SessionDeletePreview {
        project_id: project.to_owned(),
        session_ids: args.session_ids,
        children,
        include_children,
        artifacts,
        artifact_error,
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_requires_a_bounded_unique_selection() {
        assert!(validate_selection(&["s".into(), "other".into()]).is_ok());
        for invalid in [
            vec![],
            vec![" ".into()],
            vec!["s".into(), "s".into()],
            (0..1001).map(|i| i.to_string()).collect(),
        ] {
            assert!(validate_selection(&invalid).is_err());
        }
    }
    #[test]
    fn nested_selected_sessions_are_deleted_once_but_unseen_children_are_never_assumed() {
        let mut preview = wisp_dto::SessionChildrenPreview::default();
        assert_eq!(child_choice(&preview, true), None);
        preview.nested.push("child".into());
        assert_eq!(child_choice(&preview, false), Some(true));
        preview.branches = 2;
        assert_eq!(child_choice(&preview, false), Some(false));
        assert_eq!(child_choice(&preview, true), Some(true));
    }
}
