//! Native assistant reads privacy-filtered records before naming any project.
//! Writes reuse the shared scheduler and its normal turn/approval pipeline.
use crate::native_settings::{invoke_command, Broker};
use serde_json::{json, Value};
use tauri::Manager;
use wisp_dto::{native_assistant as dto, native_settings::Request};
use wisp_store::Store;

pub(crate) async fn workspace(store: &Store, day: &str) -> Result<dto::Workspace, String> {
    let projects = crate::research_assistant::visible_projects(store).await?;
    let plan = crate::research_assistant::visible_plan(store, day).await?;
    let mut schedules = Vec::new();
    let mut runs = Vec::new();
    for (id, _, _, _) in &projects {
        let rows = store.list_schedules(id).await.map_err(|e| e.to_string())?;
        for row in &rows {
            runs.extend(
                store
                    .list_schedule_runs(&row.id, 10)
                    .await
                    .map_err(|e| e.to_string())?,
            );
        }
        schedules.extend(rows);
    }
    schedules.sort_by_key(|row| (row.next_run_at, row.id.clone()));
    runs.sort_by_key(|row| std::cmp::Reverse(row.fired_at));
    runs.truncate(100);
    Ok(dto::Workspace {
        day: day.into(),
        projects: projects
            .into_iter()
            .map(|(id, name, description, _)| dto::Project {
                id,
                name,
                description,
            })
            .collect(),
        plan,
        schedules,
        runs,
        daily_recap: crate::research_recap::load_daily(store).await,
    })
}

/// Return a checked project binding and the existing scheduler command.
async fn mutation(
    store: &Store,
    operation: dto::Operation,
) -> Result<(Option<String>, &'static str, Value), String> {
    use dto::Operation::*;
    let visible: std::collections::HashSet<_> = crate::research_assistant::visible_projects(store)
        .await?
        .into_iter()
        .map(|p| p.0)
        .collect();
    let (project, command, args) = match operation {
        CreateSchedule {
            project_id,
            name,
            prompt,
            interval_secs,
            session_id,
            skill,
            start_at,
        } => {
            if !visible.contains(&project_id) {
                return Err("The target project is not visible".into());
            }
            if !(60..=365 * 86400).contains(&interval_secs)
                || prompt.trim().is_empty()
                || prompt.len() > 128 * 1024
            {
                return Err("Invalid automation interval or prompt".into());
            }
            if let Some(session) = &session_id {
                check_session(store, &project_id, session).await?;
            }
            (
                Some(project_id.clone()),
                "create_schedule",
                json!({"projectId": project_id, "name": name, "prompt": prompt, "intervalSecs": interval_secs, "sessionId": session_id, "skill": skill, "startAt": start_at}),
            )
        }
        SetTimer {
            project_id,
            session_id,
            expression,
        } => {
            if !visible.contains(&project_id) {
                return Err("The target project is not visible".into());
            }
            wisp_dto::parse_timer_expression(&expression)?;
            check_session(store, &project_id, &session_id).await?;
            (
                Some(project_id),
                "set_session_timer",
                json!({"sessionId": session_id, "expression": expression}),
            )
        }
        SetEnabled { id, enabled } => (
            Some(check_schedule(store, &visible, &id).await?),
            "set_schedule_enabled",
            json!({"id": id, "enabled": enabled}),
        ),
        Delete { id } => (
            Some(check_schedule(store, &visible, &id).await?),
            "delete_schedule",
            json!({"id": id}),
        ),
        RunNow { id } => (
            Some(check_schedule(store, &visible, &id).await?),
            "run_schedule_now",
            json!({"id": id}),
        ),
        SetDailyRecap { enabled, time } => (
            None,
            "set_daily_recap_automation",
            json!({"enabled": enabled, "time": time}),
        ),
        RunDailyRecap => (None, "run_daily_recap_now", json!({})),
    };
    Ok((project, command, args))
}

async fn check_session(store: &Store, project: &str, session: &str) -> Result<(), String> {
    if store
        .frame_project_id(session)
        .await
        .map_err(|e| e.to_string())?
        .as_deref()
        != Some(project)
    {
        return Err("The target conversation belongs to a different project".into());
    }
    store
        .require_unarchived_session(session)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn check_schedule(
    store: &Store,
    visible: &std::collections::HashSet<String>,
    id: &str,
) -> Result<String, String> {
    // Query only admitted projects rather than looking up a guessed hidden ID.
    for project in visible {
        if store
            .list_schedules(project)
            .await
            .map_err(|e| e.to_string())?
            .iter()
            .any(|row| row.id == id)
        {
            return Ok(project.clone());
        }
    }
    Err("The automation is unavailable or its project is hidden".into())
}

pub(crate) async fn execute(broker: &Broker, request: &Request) -> Result<Value, String> {
    if request.project_id.is_some() {
        return Err("Assistant commands do not use a project binding".into());
    }
    let state = broker.app.state::<crate::AppState>();
    match request.command.as_str() {
        "native_assistant_open" => {
            if !request.args.as_object().is_some_and(|a| a.is_empty()) {
                return Err("Assistant open takes no arguments".into());
            }
            crate::research_assistant::ensure(&state.store, &state.app_data).await?;
            serde_json::to_value(dto::OpenResult {
                project_id: wisp_dto::ASSISTANT_PROJECT_ID.into(),
                session_id: crate::research_assistant::ASSISTANT_FRAME_ID.into(),
            })
            .map_err(|e| e.to_string())
        }
        "native_assistant_workspace" => {
            let input: dto::WorkspaceRequest =
                serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
            serde_json::to_value(workspace(&state.store, &input.day).await?)
                .map_err(|e| e.to_string())
        }
        "native_assistant_mutate" => {
            let input: dto::MutationRequest =
                serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
            let (project, command, args) = mutation(&state.store, input.operation).await?;
            invoke_command(broker, project, command, args).await
        }
        _ => Err("Unsupported native assistant command".into()),
    }
}

pub(crate) async fn timer(
    broker: &Broker,
    project: &str,
    input: dto::TimerRequest,
) -> Result<Value, String> {
    let state = broker.app.state::<crate::AppState>();
    if !crate::research_assistant::visible_projects(&state.store)
        .await?
        .iter()
        .any(|p| p.0 == project)
    {
        return Err("The target project is not visible".into());
    }
    check_session(&state.store, project, &input.session_id).await?;
    let schedule = state
        .store
        .list_schedules(project)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.replace_previous_turn && s.frame_id.as_deref() == Some(&input.session_id));
    use dto::TimerOperation::*;
    match input.operation {
        Read => serde_json::to_value(schedule).map_err(|e| e.to_string()),
        Set { expression } => {
            wisp_dto::parse_timer_expression(&expression)?;
            invoke_command(
                broker,
                Some(project.into()),
                "set_session_timer",
                json!({"sessionId":input.session_id,"expression":expression}),
            )
            .await
        }
        Cancel => {
            let id = schedule.ok_or("This conversation has no timer")?.id;
            invoke_command(
                broker,
                Some(project.into()),
                "delete_schedule",
                json!({"id":id}),
            )
            .await?;
            Ok(Value::Null)
        }
        SetEnabled { enabled } => {
            let id = schedule.ok_or("This conversation has no timer")?.id;
            invoke_command(
                broker,
                Some(project.into()),
                "set_schedule_enabled",
                json!({"id":id,"enabled":enabled}),
            )
            .await?;
            Ok(Value::Null)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn privacy_filters_before_plan_schedule_and_target_reads() {
        let path = std::env::temp_dir().join(format!(
            "wisp-native-assistant-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        for id in ["visible", "hidden"] {
            store.create_project(id, id, "").await.unwrap();
            store
                .create_frame(&format!("{id}-session"), id, "OPERON", "m")
                .await
                .unwrap();
            store
                .create_schedule(&wisp_dto::ScheduleRecord {
                    id: format!("{id}-schedule"),
                    project_id: id.into(),
                    prompt: "Check records".into(),
                    interval_secs: 3600,
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        crate::privacy_mode::save(&store, true, &["hidden".into()])
            .await
            .unwrap();
        let value = workspace(&store, "2026-10-08").await.unwrap();
        assert_eq!(value.projects.len(), 1);
        assert_eq!(value.schedules[0].id, "visible-schedule");
        assert_eq!(value.schedules.len(), 1);
        assert!(workspace(&store, "2026-02-30").await.is_err());
        assert!(mutation(
            &store,
            dto::Operation::RunNow {
                id: "hidden-schedule".into()
            }
        )
        .await
        .is_err());
        assert!(mutation(
            &store,
            dto::Operation::SetTimer {
                project_id: "hidden".into(),
                session_id: "hidden-session".into(),
                expression: "1h records".into()
            }
        )
        .await
        .unwrap_err()
        .contains("not visible"));
        assert!(mutation(
            &store,
            dto::Operation::SetTimer {
                project_id: "visible".into(),
                session_id: "hidden-session".into(),
                expression: "1h records".into()
            }
        )
        .await
        .unwrap_err()
        .contains("different project"));
        let (binding, command, args) = mutation(
            &store,
            dto::Operation::SetTimer {
                project_id: "visible".into(),
                session_id: "visible-session".into(),
                expression: "1h records".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(binding.as_deref(), Some("visible"));
        assert_eq!(command, "set_session_timer");
        assert_eq!(args["sessionId"], "visible-session");
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
