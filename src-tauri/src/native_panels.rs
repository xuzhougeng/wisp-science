//! Native side-panel operations resolve the requested frame directly, never a shared
//! hidden window's active frame. Exploration directories remain isolated.
#![deny(irrefutable_let_patterns)]
use crate::native_settings::{invoke_command, Broker};
use serde_json::Value;
use tauri::Manager;
use wisp_dto::{native_conversations::PanelRequest, native_settings::Request};

pub(crate) async fn dispatch(
    broker: &Broker,
    request: &Request,
    project_id: &str,
    session: &str,
) -> Result<Value, String> {
    if request.command == "native_conversation_panel_run_review" {
        return crate::native_run_review::dispatch(broker, request, project_id, session).await;
    }
    let args: PanelRequest =
        serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
    let state = broker.app.state::<crate::AppState>();
    let (project, scope) =
        crate::exploration_commands::working_project_for_frame(&state, session).await?;
    if project.id != project_id {
        return Err("Project scope mismatch".into());
    }
    match request.command.as_str() {
        "native_conversation_panel_side_chat_options" => {
            let models = invoke_command(
                broker,
                Some(project_id.into()),
                "list_models",
                serde_json::json!({}),
            )
            .await?;
            let models: Vec<wisp_dto::ModelProfile> =
                serde_json::from_value(models).map_err(|e| e.to_string())?;
            let agents = invoke_command(
                broker,
                Some(project_id.into()),
                "list_acp_agents",
                serde_json::json!({}),
            )
            .await?;
            let agents: Vec<wisp_dto::AcpAgentProfile> =
                serde_json::from_value(agents).map_err(|e| e.to_string())?;
            let mut options: Vec<_> = models
                .into_iter()
                .filter(|model| model.is_chat_model())
                .map(
                    |model| wisp_dto::native_conversations::SideChatModelOption {
                        id: model.id,
                        label: model.label,
                        kind: "http".into(),
                        active: model.active,
                    },
                )
                .collect();
            options.extend(agents.into_iter().map(|agent| {
                wisp_dto::native_conversations::SideChatModelOption {
                    id: agent.id,
                    label: agent.label,
                    kind: "acp".into(),
                    active: false,
                }
            }));
            serde_json::to_value(options).map_err(|e| e.to_string())
        }
        "native_conversation_panel_side_chat" => {
            let value = invoke_command(broker, Some(project_id.into()), "side_chat", serde_json::json!({
                "sessionId": session, "question": args.question.ok_or("Question is required")?, "acpAgentId": args.acp_agent_id,
            })).await?;
            let reply: wisp_dto::SideChatResponse =
                serde_json::from_value(value).map_err(|e| e.to_string())?;
            if reply.session_id.as_deref() != Some(session) {
                return Err("Side-chat reply scope mismatch".into());
            }
            serde_json::to_value(reply).map_err(|e| e.to_string())
        }

        "native_conversation_panel_highlights" | "native_conversation_panel_notebook_stars" => {
            let kind = if request.command.ends_with("highlights") {
                "text"
            } else {
                "code"
            };
            let rows = state
                .library
                .list_for_session(session)
                .await
                .map_err(|e| e.to_string())?;
            let rows: Vec<_> = rows
                .into_iter()
                .filter(|row| library_item_in_scope(row, project_id, session, kind))
                .collect();
            contract::<Vec<wisp_dto::LibraryItem>>(
                serde_json::to_value(rows).map_err(|e| e.to_string())?,
            )
        }
        "native_conversation_panel_highlight_star" => {
            let value = invoke_command(
                broker,
                Some(project_id.into()),
                "star_library_text",
                serde_json::json!({
                    "sessionId": session, "text": args.text.ok_or("Selected text is required")?,
                }),
            )
            .await?;
            contract::<wisp_dto::LibraryItem>(value)
        }
        "native_conversation_panel_highlight_remove" => {
            // Personal library snapshots remain editable even when the source session is read-only.
            let id = args.library_item_id.ok_or("Library item ID is required")?;
            Ok(Value::Bool(
                remove_highlight(&state.library, project_id, session, &id).await?,
            ))
        }

        "native_conversation_panel_notebook_star" => {
            let value = invoke_command(
                broker,
                Some(project_id.into()),
                "star_library_code",
                serde_json::json!({
                    "sessionId": session,
                    "language": args.language.ok_or("Code language is required")?,
                    "code": args.code.ok_or("Code is required")?,
                }),
            )
            .await?;
            contract::<wisp_dto::LibraryItem>(value)
        }
        "native_conversation_panel_notebook_unstar" => {
            let id = args.library_item_id.ok_or("Library item ID is required")?;
            Ok(Value::Bool(
                remove_panel_library_item(&state.library, project_id, session, &id, "code").await?,
            ))
        }
        "native_conversation_panel_agent_delegation" => {
            if let Some(enabled) = args.enabled {
                crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
                state
                    .store
                    .require_unarchived_session(session)
                    .await
                    .map_err(|e| e.to_string())?;
                invoke_command(
                    broker,
                    Some(project_id.into()),
                    "set_session_delegation_enabled",
                    serde_json::json!({"sessionId": session, "enabled": enabled}),
                )
                .await
            } else {
                Ok(Value::Bool(
                    crate::delegation_runtime::session_delegation_enabled(&state.store, session)
                        .await,
                ))
            }
        }
        "native_conversation_panel_agent_action" => {
            use wisp_dto::native_conversations::AgentAction;
            let workflow_id = args.workflow_id.ok_or("Workflow ID is required")?;
            let action = args.action.ok_or("Workflow action is required")?;
            let rows = crate::delegation_runtime::load_agent_workflow_snapshots(
                &state.store,
                project_id,
                Some(session),
            )
            .await?;
            let workflow = rows
                .iter()
                .find(|row| row.workflow.id == workflow_id)
                .ok_or("Workflow is outside this conversation scope")?;
            if workflow.workflow.depth != 0 {
                return Err("Control nested workflows through their root workflow".into());
            }
            if action != AgentAction::Cancel {
                crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
                state
                    .store
                    .require_unarchived_session(session)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            let _activity = state.begin_project_activity(project_id)?;
            match action {
                AgentAction::Approve => {
                    crate::delegation_runtime::approve_agent_workflow_for_project(
                        &state,
                        project,
                        workflow_id,
                        args.expected_version
                            .ok_or("Workflow version is required")?,
                    )
                    .await?;
                }
                AgentAction::Run => {
                    crate::delegation_runtime::run_agent_workflow_for_project(
                        &state,
                        project,
                        workflow_id,
                    )
                    .await?;
                }
                AgentAction::Retry => {
                    let overrides = args
                        .budget_overrides
                        .map(|value| {
                            serde_json::from_value(
                                serde_json::to_value(value).map_err(|e| e.to_string())?,
                            )
                            .map_err(|e| e.to_string())
                        })
                        .transpose()?;
                    crate::delegation_runtime::retry_agent_workflow_for_project(
                        &state,
                        project,
                        Some(session.into()),
                        workflow_id,
                        overrides,
                    )
                    .await?;
                }
                AgentAction::Cancel | AgentAction::Discard => {
                    let command = if action == AgentAction::Cancel {
                        "cancel_agent_workflow"
                    } else {
                        "discard_agent_workflow"
                    };
                    invoke_command(
                        broker,
                        Some(project_id.into()),
                        command,
                        serde_json::json!({"workflowId": workflow_id}),
                    )
                    .await?;
                }
            }
            Ok(Value::Null)
        }
        "native_conversation_panel_agents" | "native_conversation_panel_agent_result" => {
            let workflows = crate::delegation_runtime::load_agent_workflow_snapshots(
                &state.store,
                project_id,
                Some(session),
            )
            .await?;
            if request.command == "native_conversation_panel_agents" {
                return contract::<Vec<wisp_dto::AgentWorkflowSnapshot>>(
                    serde_json::to_value(workflows).map_err(|e| e.to_string())?,
                );
            }
            let workflow_id = args.workflow_id.ok_or("Workflow ID is required")?;
            let step_id = args.step_id.ok_or("Step ID is required")?;
            if !workflows.iter().any(|row| row.workflow.id == workflow_id) {
                return Err("Workflow is outside this conversation scope".into());
            }
            let result = crate::delegation_runtime::load_agent_workflow_result(
                &state.store,
                project_id,
                &workflow_id,
                &step_id,
            )
            .await?;
            contract::<wisp_dto::AgentWorkflowResultDetail>(
                serde_json::to_value(result).map_err(|e| e.to_string())?,
            )
        }
        "native_conversation_panel_runtime_start" | "native_conversation_panel_runtime_execute" => {
            crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
            state
                .store
                .require_unarchived_session(session)
                .await
                .map_err(|e| e.to_string())?;
            let context = args.context_id.ok_or("Execution context is required")?;
            let language: wisp_runtime::RuntimeLanguage =
                serde_json::from_value(serde_json::json!(args
                    .language
                    .ok_or("Runtime language is required")?))
                .map_err(|e| e.to_string())?;
            let key = crate::runtime_commands::resolve_runtime_key(
                &state.runtime_manager,
                project_id.into(),
                scope.scope_key().into(),
                session,
                context.clone(),
                language,
            );
            let _activity = state.begin_project_activity(project_id)?;
            if request.command == "native_conversation_panel_runtime_start" {
                let info = state
                    .runtime_manager
                    .start(key, project.root)
                    .await
                    .map_err(|e| e.to_string())?;
                return serde_json::to_value(info).map_err(|e| e.to_string());
            }
            let code = args.code.ok_or("Code is required")?;
            if code.len() > wisp_runtime::MAX_CODE_BYTES {
                return Err("Code exceeds the runtime size limit".into());
            }
            if crate::exploration_isolation::is_host_local_context(&context) {
                if let Some(boundary) =
                    crate::exploration_isolation::boundary_for_scope(&state.store, &scope).await?
                {
                    boundary.check_local_source(&code)?;
                }
            }
            let execution = state
                .runtime_manager
                .execute(&key, &project.root, code)
                .await
                .map_err(|e| e.to_string())?;
            let result = crate::runtime_commands::finish_runtime_execution(
                &state,
                &scope,
                execution,
                |response, _| wisp_runtime::format_response(response),
            )
            .await?;
            contract::<wisp_dto::RuntimeExecutionSummary>(
                serde_json::to_value(result).map_err(|e| e.to_string())?,
            )
        }
        "native_conversation_panel_runtime_stop"
        | "native_conversation_panel_runtime_restart"
        | "native_conversation_panel_runtime_dismiss" => {
            let id = args.runtime_id.ok_or("Runtime ID is required")?;
            let runtime = state
                .runtime_manager
                .list()
                .into_iter()
                .find(|r| {
                    r.runtime_id == id
                        && crate::runtime_commands::runtime_visible(&r.key, &scope, session)
                })
                .ok_or("Runtime is outside this conversation scope")?;
            require_runtime_generation(runtime.generation, args.runtime_generation)?;
            if request.command == "native_conversation_panel_runtime_dismiss" {
                state
                    .runtime_manager
                    .dismiss_dead(&id)
                    .map_err(|e| e.to_string())?;
                return Ok(Value::Null);
            }
            let _activity = state.begin_project_activity(&runtime.key.project_id)?;
            if request.command == "native_conversation_panel_runtime_stop" {
                return serde_json::to_value(state.runtime_manager.stop(&runtime.key).await)
                    .map_err(|e| e.to_string());
            }
            state
                .store
                .require_unarchived_session(session)
                .await
                .map_err(|e| e.to_string())?;
            let root = if runtime.key.project_id == project_id {
                crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
                project.root
            } else {
                let (_, workspace) = state
                    .store
                    .get_project(&runtime.key.project_id)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or("Project not found")?;
                crate::ensure_writable(std::path::PathBuf::from(workspace), &state.app_data)
            };
            serde_json::to_value(
                state
                    .runtime_manager
                    .restart(runtime.key, root)
                    .await
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())
        }
        "native_conversation_panel_activity" => {
            let runtimes = state
                .runtime_manager
                .list()
                .into_iter()
                .filter(|runtime| {
                    crate::runtime_commands::runtime_visible(&runtime.key, &scope, session)
                })
                .collect::<Vec<_>>();
            let runs = state
                .store
                .list_run_summaries_in_scope(&scope)
                .await
                .map_err(|e| e.to_string())?;
            let read_only =
                crate::exploration_commands::require_writable_scope(&state.store, &scope)
                    .await
                    .is_err()
                    || state
                        .store
                        .require_unarchived_session(session)
                        .await
                        .is_err();
            contract::<wisp_dto::native_conversations::PanelActivity>(
                serde_json::json!({"runtimes": runtimes, "runs": runs, "read_only": read_only, "run_review_supported": true}),
            )
        }
        "native_conversation_panel_runtime_inspect" => {
            let id = args.runtime_id.ok_or("Runtime ID is required")?;
            let runtime = state
                .runtime_manager
                .list()
                .into_iter()
                .find(|runtime| {
                    runtime.runtime_id == id
                        && crate::runtime_commands::runtime_visible(&runtime.key, &scope, session)
                })
                .ok_or("Runtime is not visible in this conversation scope")?;
            let objects = state
                .runtime_manager
                .inspect(&runtime.key)
                .await
                .map_err(|e| e.to_string())?;
            contract::<wisp_dto::RuntimeObjectList>(
                serde_json::to_value(objects).map_err(|e| e.to_string())?,
            )
        }
        "native_conversation_panel_run_detail"
        | "native_conversation_panel_run_cancel"
        | "native_conversation_panel_run_harvest" => {
            let id = args.run_id.ok_or("Run ID is required")?;
            let mutation = request.command != "native_conversation_panel_run_detail";
            require_run_scope(&state.store, &scope, &id, mutation).await?;
            if request.command != "native_conversation_panel_run_detail" {
                crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
                state
                    .store
                    .require_unarchived_session(session)
                    .await
                    .map_err(|e| e.to_string())?;
                let _activity = state.begin_project_activity(project_id)?;
                if request.command == "native_conversation_panel_run_cancel" {
                    state.run_manager.cancel(&state.store, &id).await?;
                } else {
                    state.run_manager.harvest_run(&state.store, &id).await?;
                }
            }
            let run = state
                .store
                .get_run(&id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("Run not found")?;
            contract::<wisp_dto::RunRecord>(serde_json::to_value(run).map_err(|e| e.to_string())?)
        }
        "native_conversation_panel_contexts" => {
            let contexts = state
                .store
                .list_execution_contexts()
                .await
                .map_err(|e| e.to_string())?;
            let enabled_ids = state
                .store
                .list_session_execution_context_ids(session)
                .await
                .map_err(|e| e.to_string())?;
            let read_only =
                crate::exploration_commands::require_writable_scope(&state.store, &scope)
                    .await
                    .is_err()
                    || state
                        .store
                        .require_unarchived_session(session)
                        .await
                        .is_err();
            // Validate the existing store shape against the shared UI contract.
            let contexts =
                serde_json::from_value(serde_json::to_value(contexts).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            serde_json::to_value(wisp_dto::native_conversations::PanelContexts {
                contexts,
                enabled_ids,
                read_only,
                default_context: Some(wisp_dto::native_conversations::PanelDefaultContext {
                    context_id: state
                        .store
                        .session_default_execution_context(session)
                        .await
                        .map_err(|e| e.to_string())?,
                }),
            })
            .map_err(|e| e.to_string())
        }
        "native_conversation_panel_context_default" => {
            let context_id = args
                .context_id
                .ok_or("Execution context is required; use local for this machine")?;
            if context_id.trim().is_empty() {
                return Err("Execution context is required".into());
            }
            state
                .store
                .require_unarchived_session(session)
                .await
                .map_err(|e| e.to_string())?;
            let saved = crate::ssh_hosts::set_session_default_execution_context(
                state,
                session.into(),
                Some(context_id),
            )
            .await?;
            serde_json::to_value(wisp_dto::native_conversations::PanelDefaultContext {
                context_id: saved,
            })
            .map_err(|e| e.to_string())
        }
        "native_conversation_panel_context_enabled" => {
            let context_id = args.context_id.ok_or("Execution context is required")?;
            let enabled = args.enabled.ok_or("Enabled state is required")?;
            let context = state
                .store
                .get_execution_context(&context_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("Execution context not found")?;
            if context.kind == wisp_store::ExecutionContextKind::Local {
                return Err("The local context is always available".into());
            }
            state
                .store
                .require_unarchived_session(session)
                .await
                .map_err(|e| e.to_string())?;
            let ids = crate::ssh_hosts::set_session_execution_context_enabled(
                state,
                session.into(),
                context_id,
                enabled,
            )
            .await?;
            serde_json::to_value(ids).map_err(|e| e.to_string())
        }
        "native_conversation_panel_artifacts" => {
            invoke_command(
                broker,
                Some(project_id.into()),
                "list_artifacts",
                serde_json::json!({"sessionId": session}),
            )
            .await
        }
        "native_conversation_panel_files" => {
            let path = args.path.unwrap_or_else(|| ".".into());
            let directory = wisp_tools::safety::resolve_under_root(&project.root, &path)?;
            tokio::task::spawn_blocking(move || {
                let entries = crate::file_browser::list_dir_entries(&directory)?;
                serde_json::to_value(entries).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?
        }
        "native_conversation_panel_file_action" => {
            crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
            state
                .store
                .require_unarchived_session(session)
                .await
                .map_err(|e| e.to_string())?;
            let _activity = state.begin_project_activity(project_id)?;
            let action = args.file_action.ok_or("File action is required")?;
            let path = args.path.ok_or("File path is required")?;
            tokio::task::spawn_blocking(move || {
                apply_file_action(&project.root, action, &path, args.new_path.as_deref())
            })
            .await
            .map_err(|e| e.to_string())??;
            state
                .store
                .bump_state_generation(&scope)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Bool(true))
        }
        "native_conversation_panel_savefile" => {
            crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
            state
                .store
                .require_unarchived_session(session)
                .await
                .map_err(|e| e.to_string())?;
            let _activity = state.begin_project_activity(project_id)?;
            let path = args.path.ok_or("File path is required")?;
            let original = args.original_text.ok_or("Original file text is required")?;
            let text = args.text.ok_or("File text is required")?;
            tokio::task::spawn_blocking(move || {
                crate::file_browser::save_file_preview_at(&project.root, &path, &original, &text)
            })
            .await
            .map_err(|e| e.to_string())??;
            state
                .store
                .bump_state_generation(&scope)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Bool(true))
        }
        "native_conversation_panel_readfile" => {
            let path = args.path.ok_or("File path is required")?;
            read(project.root, path, args.render_pdf, args.render_office).await
        }
        "native_conversation_panel_readartifact" => {
            let id = args.artifact_id.ok_or("Artifact ID is required")?;
            if !state
                .store
                .artifact_visible_in_scope(&id, &scope)
                .await
                .map_err(|e| e.to_string())?
            {
                return Err("Artifact is not visible in this conversation scope".into());
            }
            let path = state
                .store
                .artifact_path_in_scope(&id, &scope)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("Artifact not found")?;
            read(project.root, path, args.render_pdf, args.render_office).await
        }
        _ => Err("Unknown native panel command".into()),
    }
}
fn library_item_in_scope(
    row: &wisp_store::LibraryItem,
    project: &str,
    session: &str,
    kind: &str,
) -> bool {
    row.kind == kind && row.source_project_id == project && row.source_session_id == session
}
async fn remove_highlight(
    library: &wisp_store::LibraryStore,
    project: &str,
    session: &str,
    id: &str,
) -> Result<bool, String> {
    remove_panel_library_item(library, project, session, id, "text").await
}
async fn remove_panel_library_item(
    library: &wisp_store::LibraryStore,
    project: &str,
    session: &str,
    id: &str,
    kind: &str,
) -> Result<bool, String> {
    let row = library
        .get(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Library item no longer exists; refresh the panel")?;
    if !library_item_in_scope(&row.item, project, session, kind) {
        return Err("Library item is outside this conversation scope or kind".into());
    }
    library.delete(id).await.map_err(|e| e.to_string())
}
fn require_runtime_generation(current: u64, expected: Option<u64>) -> Result<(), String> {
    if expected != Some(current) {
        return Err("Runtime changed; refresh before controlling it".into());
    }
    Ok(())
}
pub(crate) async fn require_run_scope(
    store: &wisp_store::Store,
    scope: &wisp_store::StateScope,
    id: &str,
    mutation: bool,
) -> Result<(), String> {
    if !store
        .run_visible_in_scope(id, scope)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("Run is not visible in this conversation scope".into());
    }
    if mutation
        && store
            .run_state_scope(id)
            .await
            .map_err(|e| e.to_string())?
            .as_ref()
            != Some(scope)
    {
        return Err("An inherited run cannot be modified from this scope".into());
    }
    Ok(())
}
fn contract<T: serde::de::DeserializeOwned + serde::Serialize>(
    value: Value,
) -> Result<Value, String> {
    let parsed: T = serde_json::from_value(value).map_err(|e| e.to_string())?;
    serde_json::to_value(parsed).map_err(|e| e.to_string())
}
async fn read(
    root: std::path::PathBuf,
    path: String,
    render_pdf: bool,
    render_office: bool,
) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || {
        let content =
            crate::file_browser::read_native_preview_at(&root, path, render_pdf, render_office)?;
        serde_json::to_value(content).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn apply_file_action(
    root: &std::path::Path,
    action: wisp_dto::native_conversations::PanelFileAction,
    path: &str,
    new_path: Option<&str>,
) -> Result<(), String> {
    use wisp_dto::native_conversations::PanelFileAction;
    match action {
        PanelFileAction::CreateFile => crate::file_browser::create_file_at(root, path),
        PanelFileAction::CreateDirectory => crate::file_browser::create_directory_at(root, path),
        PanelFileAction::Rename => crate::file_browser::rename_entry_at(
            root,
            path,
            new_path.ok_or("New path is required")?,
        ),
        PanelFileAction::Delete => crate::file_browser::delete_entry_at(root, path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_actions_preserve_collisions_and_workspace_boundary() {
        use wisp_dto::native_conversations::PanelFileAction::*;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        apply_file_action(&root, CreateDirectory, "data", None).unwrap();
        apply_file_action(&root, CreateFile, "data/a.txt", None).unwrap();
        std::fs::write(root.join("data/a.txt"), "keep").unwrap();
        assert!(apply_file_action(&root, CreateFile, "data/a.txt", None).is_err());
        apply_file_action(&root, CreateFile, "data/b.txt", None).unwrap();
        assert!(apply_file_action(&root, Rename, "data/a.txt", Some("data/b.txt")).is_err());
        assert!(apply_file_action(&root, Rename, "data/a.txt", None).is_err());
        assert!(apply_file_action(&root, Rename, "data/a.txt", Some("../escape.txt")).is_err());
        assert!(apply_file_action(&root, Delete, ".", None).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("data/a.txt")).unwrap(),
            "keep"
        );
        apply_file_action(&root, Rename, "data/a.txt", Some("data/c.txt")).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("data/c.txt")).unwrap(),
            "keep"
        );
        apply_file_action(&root, Delete, "data", None).unwrap();
        assert!(!root.join("data").exists());
        assert!(root.is_dir());
    }
    #[test]
    fn preview_save_checks_original_and_workspace_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("analysis.py"), "print(1)\n").unwrap();
        crate::file_browser::save_file_preview_at(&root, "analysis.py", "print(1)\n", "print(2)\n")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("analysis.py")).unwrap(),
            "print(2)\n"
        );
        assert!(crate::file_browser::save_file_preview_at(
            &root,
            "analysis.py",
            "print(1)\n",
            "stale"
        )
        .is_err());
        assert!(
            crate::file_browser::save_file_preview_at(&root, "missing.txt", "", "new").is_err()
        );
        std::fs::write(temp.path().join("outside.txt"), "outside").unwrap();
        assert!(crate::file_browser::save_file_preview_at(
            &root,
            "../outside.txt",
            "outside",
            "changed"
        )
        .is_err());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("outside.txt")).unwrap(),
            "outside"
        );
        std::fs::write(root.join("large.txt"), "x".repeat(9 * 1024 * 1024)).unwrap();
        assert!(
            crate::file_browser::save_file_preview_at(&root, "large.txt", "x", "short").is_err()
        );
    }
    #[tokio::test]
    async fn highlights_reject_foreign_sources_and_non_text_deletions() {
        let temp = tempfile::tempdir().unwrap();
        let library = wisp_store::LibraryStore::open(&temp.path().join("library.sqlite"))
            .await
            .unwrap();
        for kind in ["text", "code"] {
            let row = library
                .insert(wisp_store::NewLibraryItem {
                    kind: kind.into(),
                    title: "Saved".into(),
                    language: None,
                    code: kind.into(),
                    content_type: None,
                    content: None,
                    source_project_id: "p".into(),
                    source_project_name: "P".into(),
                    source_session_id: "s".into(),
                    source_session_title: "S".into(),
                    source_path: None,
                })
                .await
                .unwrap();
            assert!(remove_highlight(&library, "other", "s", &row.id)
                .await
                .is_err());
            assert!(remove_highlight(&library, "p", "other", &row.id)
                .await
                .is_err());
            assert!(library.get(&row.id).await.unwrap().is_some());
            if kind == "text" {
                assert!(
                    remove_panel_library_item(&library, "p", "s", &row.id, "code")
                        .await
                        .is_err()
                );
                assert!(remove_highlight(&library, "p", "s", &row.id).await.unwrap());
                assert!(remove_highlight(&library, "p", "s", &row.id).await.is_err());
            } else {
                assert!(remove_highlight(&library, "p", "s", &row.id).await.is_err());
                assert!(library.get(&row.id).await.unwrap().is_some());
                assert!(
                    remove_panel_library_item(&library, "p", "s", &row.id, "code")
                        .await
                        .unwrap()
                );
            }
        }
    }
    #[test]
    fn runtime_control_requires_the_displayed_generation() {
        assert!(require_runtime_generation(2, Some(2)).is_ok());
        assert!(require_runtime_generation(2, Some(1)).is_err());
        assert!(require_runtime_generation(2, None).is_err());
    }
    #[tokio::test]
    async fn native_run_reads_and_mutations_reject_foreign_and_missing_runs() {
        let temp = tempfile::tempdir().unwrap();
        let store = wisp_store::Store::open(&temp.path().join("store.sqlite"))
            .await
            .unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store.create_project("other", "Other", "").await.unwrap();
        store
            .create_run(&wisp_store::RunRecord::new(
                "r", "p", "local", "Run", "command",
            ))
            .await
            .unwrap();
        let own = wisp_store::StateScope::mainline("p");
        let other = wisp_store::StateScope::mainline("other");
        for mutation in [false, true] {
            assert!(require_run_scope(&store, &own, "r", mutation).await.is_ok());
            assert!(require_run_scope(&store, &other, "r", mutation)
                .await
                .is_err());
            assert!(require_run_scope(&store, &own, "missing", mutation)
                .await
                .is_err());
        }
        drop(store);
    }
    #[tokio::test]
    async fn preview_reuses_workspace_boundary_and_file_contract() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("report.txt"), "result").unwrap();
        std::fs::write(temp.path().join("outside.txt"), "outside").unwrap();
        let value = read(root.clone(), "report.txt".into(), true, false)
            .await
            .unwrap();
        let content: wisp_dto::FileContent = serde_json::from_value(value).unwrap();
        assert_eq!(content.text.as_deref(), Some("result"));
        assert!(!content.truncated);
        assert!(read(root, "../outside.txt".into(), true, false)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn native_pdf_preview_preserves_pages_instead_of_markdown_extraction() {
        use base64::Engine;
        let temp = tempfile::tempdir().unwrap();
        let bytes = include_bytes!("../tests/fixtures/native-selection.pdf");
        std::fs::write(temp.path().join("paper.pdf"), bytes).unwrap();
        let extracted = read(temp.path().into(), "paper.pdf".into(), false, false)
            .await
            .unwrap();
        assert!(extracted["text"]
            .as_str()
            .unwrap()
            .contains("Alpha selection"));
        let content: wisp_dto::FileContent = serde_json::from_value(
            read(temp.path().into(), "paper.pdf".into(), true, false)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(content.mime, "application/pdf");
        assert!(content.text.is_none());
        assert!(!content.truncated);
        assert_eq!(content.total_bytes, Some(bytes.len() as u64));
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(content.base64.unwrap())
                .unwrap(),
            bytes
        );
        let oversized = std::fs::File::create(temp.path().join("large.pdf")).unwrap();
        oversized.set_len(32 * 1024 * 1024 + 1).unwrap();
        assert!(read(temp.path().into(), "large.pdf".into(), true, false)
            .await
            .unwrap_err()
            .contains("byte limit"));
    }

    #[tokio::test]
    async fn native_office_preview_validates_bytes_and_preserves_legacy_extraction() {
        use base64::Engine;
        let temp = tempfile::tempdir().unwrap();
        for (name, bytes) in [
            (
                "paper.docx",
                include_bytes!("../../ui-tests/fixtures/office-preview.docx").as_slice(),
            ),
            (
                "table.xlsx",
                include_bytes!("../../ui-tests/fixtures/office-preview.xlsx").as_slice(),
            ),
            (
                "slides.pptx",
                include_bytes!("../../ui-tests/fixtures/office-preview.pptx").as_slice(),
            ),
        ] {
            std::fs::write(temp.path().join(name), bytes).unwrap();
            let content: wisp_dto::FileContent = serde_json::from_value(
                read(temp.path().into(), name.into(), false, true)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert!(content.text.is_none());
            assert!(!content.truncated);
            assert_eq!(
                base64::engine::general_purpose::STANDARD
                    .decode(content.base64.unwrap())
                    .unwrap(),
                bytes
            );
        }
        for pdf_only in [false, true] {
            let old = read(temp.path().into(), "paper.docx".into(), pdf_only, false)
                .await
                .unwrap();
            assert!(old["text"]
                .as_str()
                .unwrap()
                .contains("Native Office preview"));
        }
        std::fs::write(temp.path().join("broken.pptx"), b"not a zip").unwrap();
        assert!(read(temp.path().into(), "broken.pptx".into(), false, true)
            .await
            .is_err());
        let large = std::fs::File::create(temp.path().join("large.xlsx")).unwrap();
        large.set_len(32 * 1024 * 1024 + 1).unwrap();
        assert!(read(temp.path().into(), "large.xlsx".into(), false, true)
            .await
            .unwrap_err()
            .contains("byte limit"));
        assert!(
            read(temp.path().into(), "../outside.docx".into(), false, true)
                .await
                .is_err()
        );
    }
}
