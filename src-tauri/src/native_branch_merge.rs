//! Adapt native requests to the existing guarded branch-summary commands.
use crate::native_settings::{invoke_command, Broker};
use serde_json::{json, Value};
use wisp_dto::native_branch_merge as dto;

pub(crate) async fn dispatch(
    broker: &Broker,
    project: &str,
    command: &str,
    args: &Value,
) -> Result<Value, String> {
    // The conversation dispatcher already verified session/project ownership.
    let (command, args) = match command {
        "native_conversation_branch_preview" => {
            let input: dto::PreviewRequest =
                serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
            (
                "preview_session_branch_merge",
                json!({"id":input.session_id}),
            )
        }
        "native_conversation_branch_generate" => {
            let input: dto::GenerateRequest =
                serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
            input.validate()?;
            (
                "summarize_session_branch_merge",
                json!({"id":input.session_id,"expectedGuardHash":input.expected_guard_hash,"currentVersion":input.current_version,"userGuidance":input.user_guidance}),
            )
        }
        "native_conversation_branch_merge" => {
            let input: dto::MergeRequest =
                serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
            if input.expected_guard_hash.trim().is_empty() || input.summary.trim().is_empty() {
                return Err("A preview guard and approved summary are required".into());
            }
            (
                "merge_session_branch_summary",
                json!({"id":input.session_id,"expectedGuardHash":input.expected_guard_hash,"summary":input.summary}),
            )
        }
        _ => return Err("Unknown branch action".into()),
    };
    let result = invoke_command(broker, Some(project.into()), command, args).await?;
    Ok(json!({"project_id":project,"result":result}))
}
