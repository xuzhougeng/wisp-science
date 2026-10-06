//! Native save panels select a destination; the host owns the reviewed ZIP write.
use crate::native_settings::Broker;
use serde_json::Value;
use tauri::Manager;
use wisp_dto::native_session_export as dto;

async fn require_ready(
    state: &crate::AppState,
    runtime: &crate::SessionRuntime,
    project: &str,
    session: &str,
    native_running: bool,
) -> Result<(), String> {
    crate::native_conversations::require_owner(&state.store, project, session).await?;
    if runtime.has_queued_turns() {
        return Err("Remove queued messages before exporting this conversation".into());
    }
    if native_running
        || state.running_turns.lock().await.contains(session)
        || state.awaiting_confirm.lock().unwrap().contains(session)
        || state.reviewing.lock().unwrap().contains(session)
    {
        return Err("Wait for the conversation to finish its turn, approval, or review".into());
    }
    crate::session_commands::flush_session_events(&runtime.ui_event_writer).await
}

async fn prepared(
    state: &crate::AppState,
    project: &str,
    session: &str,
    include_artifacts: bool,
) -> Result<(crate::session_export::PreparedSessionExport, dto::Preview), String> {
    let (working, _) =
        crate::exploration_commands::working_project_for_frame(state, session).await?;
    if working.id != project {
        return Err("Project scope mismatch".into());
    }
    let (title, _, source_revision) =
        crate::native_session_transfer::source(&state.store, project, session).await?;
    let head = state
        .store
        .frame_head_epoch(session)
        .await
        .map_err(|e| e.to_string())?;
    let export = crate::session_export::prepare_session_export(
        &state.store,
        &working.root,
        session,
        Vec::new(),
        include_artifacts,
    )
    .await?;
    let (export, revision) = export.reviewed(source_revision.clone()).await?;
    if crate::native_session_transfer::source(&state.store, project, session)
        .await?
        .2
        != source_revision
    {
        return Err("Conversation changed while preparing the export; preview it again".into());
    }
    let preview = export.preview(project, title, revision, head)?;
    Ok((export, preview))
}

pub(crate) async fn preview(
    broker: &Broker,
    project: &str,
    args: dto::PreviewRequest,
    native_running: bool,
) -> Result<Value, String> {
    args.validate(project)?;
    let state = broker.app.state::<crate::AppState>();
    let _activity = state.begin_project_activity(project)?;
    let runtime = state.session_runtime(&args.session_id).await;
    let _workflow = runtime
        .workflow
        .clone()
        .try_lock_owned()
        .map_err(|_| "Wait for this conversation to finish before exporting it")?;
    require_ready(&state, &runtime, project, &args.session_id, native_running).await?;
    let (_, preview) = prepared(&state, project, &args.session_id, args.include_artifacts).await?;
    serde_json::to_value(preview).map_err(|e| e.to_string())
}

pub(crate) async fn export(
    broker: &Broker,
    project: &str,
    args: dto::ExportRequest,
    native_running: bool,
) -> Result<Value, String> {
    args.validate(project)?;
    let destination = std::path::PathBuf::from(&args.destination_path);
    if !destination.is_absolute()
        || !destination
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        return Err("Choose an absolute ZIP archive destination".into());
    }
    let state = broker.app.state::<crate::AppState>();
    let _activity = state.begin_project_activity(project)?;
    let runtime = state.session_runtime(&args.session_id).await;
    let _workflow = runtime
        .workflow
        .clone()
        .try_lock_owned()
        .map_err(|_| "Wait for this conversation to finish before exporting it")?;
    require_ready(&state, &runtime, project, &args.session_id, native_running).await?;
    let (prepared, preview) =
        prepared(&state, project, &args.session_id, args.include_artifacts).await?;
    if preview.revision != args.revision {
        return Err(
            "Conversation or artifacts changed after the export preview; preview it again".into(),
        );
    }
    crate::native_conversations::require_owner(&state.store, project, &args.session_id).await?;
    let (bytes, checksum) =
        crate::session_export::write_session_export(prepared, destination).await?;
    serde_json::to_value(dto::ExportResult {
        schema: dto::SCHEMA.into(),
        project_id: project.into(),
        session_id: args.session_id,
        revision: args.revision,
        include_artifacts: args.include_artifacts,
        destination_path: args.destination_path,
        bytes,
        checksum,
    })
    .map_err(|e| e.to_string())
}
