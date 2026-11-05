//! Explicit project/frame runtime scope; reads never create a frame or interpreter.
use crate::native_settings::Broker;
use serde_json::Value;
use tauri::Manager;
use wisp_dto::{native_runtimes as dto, native_settings::Request};

fn visible(key: &wisp_runtime::RuntimeKey, scope: &wisp_store::StateScope, session: &str) -> bool {
    key.project_id == scope.project_id()
        && key.scope_key == scope.scope_key()
        && (key.session_id.is_empty() || key.session_id == session)
}

pub(crate) async fn dispatch(broker: &Broker, request: &Request) -> Result<Value, String> {
    let project_id = request
        .project_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or("A project is required")?;
    let state = broker.app.state::<crate::AppState>();
    if request.command == "native_context_run_action" {
        let args: dto::ContextRunActionRequest =
            serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
        let (root, scope) = resolve_scope(&state, project_id, args.session_id.as_deref()).await?;
        let mutation = match &args.operation {
            dto::ContextRunAction::Review { operation } => operation.is_mutation(),
            dto::ContextRunAction::MethodSearch { operation } => operation.is_mutation(),
            _ => true,
        };
        crate::native_panels::require_run_scope(&state.store, &scope, &args.run_id, mutation)
            .await?;
        let run = state
            .store
            .get_run(&args.run_id)
            .await
            .map_err(|e| e.to_string())?
            .filter(|r| r.context_id == args.context_id && r.project_id == project_id)
            .ok_or("Run not found in this environment")?;
        let writable = writable_run_scope(&state, &scope, args.session_id.as_deref()).await
            && match run.frame_id.as_deref() {
                Some(frame) => state.store.require_unarchived_session(frame).await.is_ok(),
                None => true,
            }
            && state
                .store
                .run_state_scope(&run.id)
                .await
                .map_err(|e| e.to_string())?
                .as_ref()
                == Some(&scope);
        if mutation && !writable {
            return Err("This run is read-only".into());
        }
        let data = match args.operation {
            dto::ContextRunAction::MethodSearch { operation } => {
                if run.kind != "method_search" {
                    return Err("This run is not a method search".into());
                }
                // The candidate ledger and coordinator are mainline-only. Never
                // return a live ledger in place of an exploration's checkpoint.
                if matches!(scope, wisp_store::StateScope::Exploration { .. }) {
                    return Err(
                        "Method-search details are only available in the original project.".into(),
                    );
                }
                method_search(&state, root, scope, &run, operation).await?
            }
            dto::ContextRunAction::Review { operation } => {
                crate::native_run_review::review(&state, project_id, &run, operation, writable)
                    .await?
            }
            action => {
                let _activity = state.begin_project_activity(project_id)?;
                match action {
                    dto::ContextRunAction::Cancel { force } if run.kind != "method_search" => {
                        state
                            .run_manager
                            .cancel_with_intent(&state.store, &run.id, Some(force))
                            .await?
                    }
                    dto::ContextRunAction::Cleanup {}
                        if run.status.is_terminal()
                            && run.cleaned_at.is_none()
                            && run.remote_workdir.is_some()
                            && !matches!(run.kind.as_str(), "file_transfer" | "method_search") =>
                    {
                        state
                            .run_manager
                            .cleanup_run_workspace(&state.store, &run.id, false)
                            .await?;
                    }
                    _ => return Err("Run cannot be cleaned in its current state".into()),
                }
                // Acknowledge the exact run, not an inferred local status. The UI refreshes its summary.
                Value::String(run.id)
            }
        };
        return serde_json::to_value(dto::ContextRunsReply {
            project_id: project_id.into(),
            session_id: args.session_id,
            context_id: args.context_id,
            data,
        })
        .map_err(|e| e.to_string());
    }
    if matches!(
        request.command.as_str(),
        "native_context_runs_list" | "native_context_runs_detail" | "native_context_run_artifacts"
    ) {
        let args: dto::ContextRunsRequest =
            serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
        let (_, scope) = resolve_scope(&state, project_id, args.session_id.as_deref()).await?;
        if args.context_id.trim().is_empty() {
            return Err("An execution context is required".into());
        }
        let data = match (request.command.as_str(), args.run_id) {
            ("native_context_run_artifacts", None) => serde_json::to_value(
                state
                    .store
                    .context_run_artifacts(&scope, &args.context_id, args.session_id.as_deref())
                    .await
                    .map_err(|e| e.to_string())?,
            ),
            ("native_context_runs_list", None) => {
                let mut runs = state
                    .store
                    .list_run_summaries_in_scope(&scope)
                    .await
                    .map_err(|e| e.to_string())?;
                runs.retain(|r| r.context_id == args.context_id);
                let rows: Vec<wisp_dto::RunSummary> =
                    serde_json::from_value(serde_json::to_value(runs).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                let manageable = if !rows.is_empty()
                    && writable_run_scope(&state, &scope, args.session_id.as_deref()).await
                {
                    state
                        .store
                        .manageable_context_runs(&scope, &args.context_id)
                        .await
                        .map_err(|e| e.to_string())?
                } else {
                    Default::default()
                };
                serde_json::to_value(
                    rows.into_iter()
                        .map(|run| dto::ContextRunSummary {
                            can_manage: manageable.contains(&run.id),
                            run,
                        })
                        .collect::<Vec<_>>(),
                )
            }
            ("native_context_runs_detail", Some(id)) => {
                crate::native_panels::require_run_scope(&state.store, &scope, &id, false).await?;
                let record = state
                    .store
                    .get_run(&id)
                    .await
                    .map_err(|e| e.to_string())?
                    .filter(|r| r.context_id == args.context_id)
                    .ok_or("Run not found in this environment")?;
                let detail: wisp_dto::RunRecord = serde_json::from_value(
                    serde_json::to_value(record).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                serde_json::to_value(detail)
            }
            _ => return Err("Invalid environment runs request".into()),
        }
        .map_err(|e| e.to_string())?;
        return serde_json::to_value(dto::ContextRunsReply {
            project_id: project_id.into(),
            session_id: args.session_id,
            context_id: args.context_id,
            data,
        })
        .map_err(|e| e.to_string());
    }
    let args: dto::Request =
        serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
    let (root, scope) = resolve_scope(&state, project_id, args.session_id.as_deref()).await?;
    let session = args.session_id.as_deref().unwrap_or_default();
    let read_only = crate::exploration_commands::require_writable_scope(&state.store, &scope)
        .await
        .is_err()
        || if session.is_empty() {
            false
        } else {
            state
                .store
                .require_unarchived_session(session)
                .await
                .is_err()
        };
    match request.command.as_str() {
        "native_runtimes_read" => {
            if args.context_id.is_some()
                || args.language.is_some()
                || args.runtime_id.is_some()
                || args.runtime_generation.is_some()
            {
                return Err("Runtime read takes only a conversation scope".into());
            }
            let contexts = state
                .store
                .list_execution_contexts()
                .await
                .map_err(|e| e.to_string())?;
            let enabled_ids = if session.is_empty() {
                vec![]
            } else {
                state
                    .store
                    .list_session_execution_context_ids(session)
                    .await
                    .map_err(|e| e.to_string())?
            };
            let override_id = if session.is_empty() {
                None
            } else {
                state
                    .store
                    .session_default_execution_context(session)
                    .await
                    .map_err(|e| e.to_string())?
            };
            let default_context_id = match override_id.filter(|id| !id.trim().is_empty()) {
                Some(id) => Some(id),
                None => crate::ssh_hosts::stored_default_execution_context(&state.store).await,
            };
            let runtimes: Vec<_> = state
                .runtime_manager
                .list()
                .into_iter()
                .filter(|r| visible(&r.key, &scope, session))
                .collect();
            serde_json::to_value(dto::Snapshot {
                project_id: project_id.into(),
                session_id: args.session_id,
                contexts: serde_json::from_value(
                    serde_json::to_value(contexts).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                enabled_ids,
                default_context_id,
                runtimes: serde_json::from_value(
                    serde_json::to_value(runtimes).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                read_only,
            })
            .map_err(|e| e.to_string())
        }
        "native_runtimes_start" => {
            if read_only {
                return Err("Runtime scope is read-only".into());
            }
            if args.runtime_id.is_some() || args.runtime_generation.is_some() {
                return Err("Start takes a context and language".into());
            }
            let context = args.context_id.ok_or("Execution context is required")?;
            let language = serde_json::from_value(serde_json::json!(args
                .language
                .ok_or("Runtime language is required")?))
            .map_err(|e| e.to_string())?;
            let key = crate::runtime_commands::resolve_runtime_key(
                &state.runtime_manager,
                project_id.into(),
                scope.scope_key().into(),
                session,
                context,
                language,
            );
            let _activity = state.begin_project_activity(project_id)?;
            serde_json::to_value(
                state
                    .runtime_manager
                    .start(key, root)
                    .await
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())
        }
        "native_runtimes_stop"
        | "native_runtimes_restart"
        | "native_runtimes_dismiss"
        | "native_runtimes_inspect" => {
            if args.context_id.is_some() || args.language.is_some() {
                return Err("Runtime action takes an identity and generation".into());
            }
            let id = args.runtime_id.ok_or("Runtime ID is required")?;
            let runtime = state
                .runtime_manager
                .list()
                .into_iter()
                .find(|r| r.runtime_id == id && visible(&r.key, &scope, session))
                .ok_or("Runtime is outside this scope")?;
            if args.runtime_generation != Some(runtime.generation) {
                return Err("Runtime changed; refresh before trying again".into());
            }
            if request.command == "native_runtimes_inspect" {
                return serde_json::to_value(
                    state
                        .runtime_manager
                        .inspect(&runtime.key)
                        .await
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string());
            }
            if read_only {
                return Err("Runtime scope is read-only".into());
            }
            let _activity = state.begin_project_activity(project_id)?;
            match request.command.as_str() {
                "native_runtimes_stop" => {
                    serde_json::to_value(state.runtime_manager.stop(&runtime.key).await)
                        .map_err(|e| e.to_string())
                }
                "native_runtimes_restart" => serde_json::to_value(
                    state
                        .runtime_manager
                        .restart(runtime.key, root)
                        .await
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string()),
                _ => {
                    state
                        .runtime_manager
                        .dismiss_dead(&id)
                        .map_err(|e| e.to_string())?;
                    Ok(Value::Null)
                }
            }
        }
        _ => Err("Unknown runtime command".into()),
    }
}

async fn method_search(
    state: &crate::AppState,
    root: std::path::PathBuf,
    scope: wisp_store::StateScope,
    run: &wisp_store::RunRecord,
    operation: dto::MethodSearchOperation,
) -> Result<Value, String> {
    use crate::method_search as search;
    use dto::MethodSearchOperation as Op;
    let id = run.id.clone();
    let result = match operation {
        Op::Inspect {} => {
            search::method_search_details(&state.store, &root, &run.project_id, &id).await?
        }
        Op::ReadOutput { ref version_id } | Op::ExportOutput { ref version_id } => {
            let output = state
                .store
                .list_run_outputs(&id)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|o| &o.artifact_version_id == version_id)
                .ok_or("Version is not an output of this method search")?;
            let version = state
                .store
                .get_artifact_version(version_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("Output version not found")?;
            if !state
                .store
                .artifact_visible_in_scope(&version.artifact_id, &scope)
                .await
                .map_err(|e| e.to_string())?
            {
                return Err("Output not visible in this project".into());
            }
            let data = if matches!(operation, Op::ExportOutput { .. }) {
                let mut data = tokio::task::spawn_blocking(move || {
                    crate::native_panels::export_source(&root, &version.storage_path)
                })
                .await
                .map_err(|e| e.to_string())??;
                if let Some(name) = std::path::Path::new(&output.source_path)
                    .file_name()
                    .and_then(|s| s.to_str())
                {
                    data["name"] = Value::String(name.to_owned());
                }
                data
            } else {
                crate::native_panels::read(root, version.storage_path, true, true).await?
            };
            return Ok(serde_json::json!({"run_id":id, "version_id":version_id, "data":data}));
        }
        action => {
            let (project, _, _) =
                crate::project_commands::load_active_project(state, &run.project_id).await?;
            match action {
                Op::Start {} => search::start_in_project(state, project, scope, id).await?,
                Op::Pause {} => search::pause_in_project(state, project, id).await?,
                Op::Resume {} => search::resume_in_project(state, project, scope, id).await?,
                Op::Cancel {} => search::cancel_in_project(state, project, id).await?,
                _ => unreachable!(),
            }
        }
    };
    serde_json::to_value(result).map_err(|e| e.to_string())
}

async fn writable_run_scope(
    state: &crate::AppState,
    scope: &wisp_store::StateScope,
    session: Option<&str>,
) -> bool {
    crate::exploration_commands::require_writable_scope(&state.store, scope)
        .await
        .is_ok()
        && match session {
            Some(frame) => state.store.require_unarchived_session(frame).await.is_ok(),
            None => true,
        }
}

async fn resolve_scope(
    state: &crate::AppState,
    project_id: &str,
    session: Option<&str>,
) -> Result<(std::path::PathBuf, wisp_store::StateScope), String> {
    if session == Some("") {
        return Err("Invalid conversation ID".into());
    }
    if let Some(session) = session {
        let (project, scope) =
            crate::exploration_commands::working_project_for_frame(state, session).await?;
        if project.id != project_id {
            return Err("Project scope mismatch".into());
        }
        Ok((project.root, scope))
    } else {
        let (_, workspace) = state
            .store
            .get_project(project_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("Project not found")?;
        Ok((
            std::path::PathBuf::from(workspace),
            wisp_store::StateScope::mainline(project_id),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_scope_excludes_other_projects_sessions_and_explorations() {
        let mut key = wisp_runtime::RuntimeKey {
            project_id: "p".into(),
            scope_key: wisp_runtime::MAINLINE_RUNTIME_SCOPE.into(),
            session_id: "".into(),
            context_id: "local".into(),
            language: wisp_runtime::RuntimeLanguage::Python,
        };
        let scope = wisp_store::StateScope::mainline("p");
        assert!(visible(&key, &scope, ""));
        key.session_id = "s".into();
        assert!(!visible(&key, &scope, ""));
        assert!(visible(&key, &scope, "s"));
        assert!(!visible(&key, &scope, "other"));
        key.scope_key = "exploration".into();
        assert!(!visible(&key, &scope, "s"));
        key.scope_key = wisp_runtime::MAINLINE_RUNTIME_SCOPE.into();
        key.project_id = "other".into();
        assert!(!visible(&key, &scope, "s"));
    }
}
