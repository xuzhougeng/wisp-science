//! Private memos resolve one explicit project without activating a hidden window.
use serde_json::Value;
use wisp_dto::{native_memos::*, native_settings::Request};

pub(crate) async fn execute(store: &wisp_store::Store, request: &Request) -> Result<Value, String> {
    let project = request
        .project_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or("A project id is required")?;
    require_visible(store, project).await?;
    let encode = |value| serde_json::to_value(value).map_err(|e| e.to_string());
    match request.command.as_str() {
        "native_project_memos" => {
            let _: ListRequest =
                serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
            let memos = store
                .list_project_memos(project)
                .await
                .map_err(|e| e.to_string())?;
            require_visible(store, project).await?;
            serde_json::to_value(MemoList {
                schema: SCHEMA.into(),
                project_id: project.into(),
                memos,
            })
            .map_err(|e| e.to_string())
        }
        "native_project_memo_add" => {
            let input: AddRequest =
                serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
            let memo = store
                .add_project_memo(project, &input.input)
                .await
                .map_err(|e| e.to_string())?;
            encode(MemoResult {
                schema: SCHEMA.into(),
                project_id: project.into(),
                memo,
            })
        }
        "native_project_memo_update" => {
            let input: UpdateRequest =
                serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
            let memo = store
                .update_project_memo(project, &input.patch)
                .await
                .map_err(|e| e.to_string())?;
            encode(MemoResult {
                schema: SCHEMA.into(),
                project_id: project.into(),
                memo,
            })
        }
        "native_project_memo_delete" => {
            let input: DeleteRequest =
                serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
            store
                .delete_project_memo(project, &input.memo_id)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(DeletedMemo {
                schema: SCHEMA.into(),
                project_id: project.into(),
                memo_id: input.memo_id,
            })
            .map_err(|e| e.to_string())
        }
        _ => Err("Unsupported native memo command".into()),
    }
}

async fn require_visible(store: &wisp_store::Store, project: &str) -> Result<(), String> {
    let privacy = crate::privacy_mode::load(store).await?;
    if privacy.active && privacy.project_ids.iter().any(|id| id == project)
        || store
            .get_project(project)
            .await
            .map_err(|e| e.to_string())?
            .is_none()
    {
        return Err("Project is unavailable".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn private_memos_require_explicit_visible_owner_and_never_cross_projects() {
        let root = tempfile::tempdir().unwrap();
        let store = wisp_store::Store::open(&root.path().join("memos.sqlite"))
            .await
            .unwrap();
        for project in ["a", "b"] {
            store.create_project(project, project, "").await.unwrap();
        }
        let mut request = Request {
            schema: wisp_dto::native_settings::SCHEMA.into(),
            id: "memo".into(),
            project_id: Some("a".into()),
            command: "native_project_memo_add".into(),
            args: json!({"input":{"title":"Private","body":"Only a"}}),
        };
        let created: MemoResult =
            serde_json::from_value(execute(&store, &request).await.unwrap()).unwrap();
        assert_eq!(created.memo.project_id, "a");
        request.project_id = Some("b".into());
        request.command = "native_project_memo_update".into();
        request.args = json!({"patch":{"id":created.memo.id,"body":"wrong owner"}});
        assert!(execute(&store, &request).await.is_err());
        request.command = "native_project_memo_delete".into();
        request.args = json!({"memo_id":created.memo.id});
        let _ = execute(&store, &request).await;
        assert_eq!(
            store.list_project_memos("a").await.unwrap()[0].body,
            "Only a"
        );
        request.command = "native_project_memos".into();
        request.args = json!({});
        assert!(
            serde_json::from_value::<MemoList>(execute(&store, &request).await.unwrap())
                .unwrap()
                .memos
                .is_empty()
        );
        request.project_id = None;
        assert!(execute(&store, &request).await.is_err());
        request.project_id = Some("a".into());
        request.args = json!({"active_window":"other"});
        assert!(execute(&store, &request).await.is_err());
        request.args = json!({});
        crate::privacy_mode::save(&store, true, &["a".into()])
            .await
            .unwrap();
        assert!(execute(&store, &request).await.is_err());
        request.command = "native_project_memo_add".into();
        request.args = json!({"input":{"body":"hidden write"}});
        assert!(execute(&store, &request).await.is_err());
        assert_eq!(store.list_project_memos("a").await.unwrap().len(), 1);
    }
}
