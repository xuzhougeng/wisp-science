//! Native conversation commands reuse the desktop turn and transcript pipeline.
//! Snapshot reads serialize per session, so clients can discard late responses.
use crate::native_settings::{invoke_command, Broker};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use tauri::Manager;
use tokio::sync::Mutex;
use wisp_dto::{native_conversations as dto, native_settings::Request};

pub(crate) struct Conversations {
    pub(crate) queue_events: crate::native_queue::QueueEvents,
    epoch: String,
    sessions: Mutex<HashMap<String, Arc<Mutex<Record>>>>,
    suggestions: std::sync::Mutex<HashMap<String, Vec<String>>>,
}
impl Default for Conversations {
    fn default() -> Self {
        Self {
            queue_events: Default::default(),
            epoch: uuid::Uuid::new_v4().to_string(),
            sessions: Mutex::new(HashMap::new()),
            suggestions: Default::default(),
        }
    }
}
#[derive(Default)]
struct Record {
    queued_requests: HashMap<String, [u8; 32]>,
    sequence: u64,
    running: bool,
    stopping: bool,
    request_id: Option<String>,
    error: Option<String>,
    accepted: HashMap<String, [u8; 32]>,
}
impl Record {
    fn accept(
        &mut self,
        request: &dto::SendRequest,
        external_running: bool,
    ) -> Result<bool, String> {
        uuid::Uuid::parse_str(&request.request_id).map_err(|_| "Invalid send request ID")?;
        if request.message.trim().is_empty() || request.message.len() > 128 * 1024 {
            return Err("Message must contain 1–131072 bytes".into());
        }
        if request.references.len() > 64 {
            return Err("Too many composer references".into());
        }
        let payload =
            serde_json::to_vec(&(&request.message, &request.attachments, &request.references))
                .map_err(|error| error.to_string())?;
        let digest: [u8; 32] = Sha256::digest(payload).into();
        if let Some(previous) = self.accepted.get(&request.request_id) {
            return if previous == &digest {
                Ok(false)
            } else {
                Err("Request ID was already used for another message".into())
            };
        }
        if self.running || external_running {
            return Err("This conversation is already running".into());
        }
        if self.accepted.len() >= 1024 {
            return Err(
                "Native request ledger is full; restart the host after current tasks finish".into(),
            );
        }
        self.accepted.insert(request.request_id.clone(), digest);
        self.running = true;
        self.stopping = false;
        self.error = None;
        self.request_id = Some(request.request_id.clone());
        Ok(true)
    }
}
impl Conversations {
    // Same transient lifetime as WebView follow-ups. A new user turn invalidates
    // the previous suggestions, including turns started from another surface.
    pub(crate) fn observe(&self, event: &crate::AgentEvent) {
        if let crate::AgentEvent::User {
            frame_id,
            queue_id: Some(id),
            ..
        } = event
        {
            self.queue_events.record(frame_id, *id, "started");
        }
        let mut suggestions = self.suggestions.lock().unwrap();
        match event {
            crate::AgentEvent::User { frame_id, .. } => {
                suggestions.remove(frame_id);
            }
            crate::AgentEvent::FollowUps {
                frame_id,
                questions,
            } => {
                if suggestions.len() < 512 || suggestions.contains_key(frame_id) {
                    suggestions.insert(
                        frame_id.clone(),
                        questions
                            .iter()
                            .filter(|text| !text.trim().is_empty() && text.len() <= 8192)
                            .take(8)
                            .cloned()
                            .collect(),
                    );
                }
            }
            _ => {}
        }
    }

    async fn session(&self, id: &str) -> Result<Arc<Mutex<Record>>, String> {
        let mut sessions = self.sessions.lock().await;
        if let Some(record) = sessions.get(id) {
            return Ok(record.clone());
        }
        if sessions.len() >= 512 {
            return Err("Too many native conversation contexts; restart the host".into());
        }
        let record = Arc::new(Mutex::new(Record::default()));
        sessions.insert(id.to_owned(), record.clone());
        Ok(record)
    }
}
fn snapshot_item(item: crate::UiItem) -> dto::Item {
    let attachments = if item.role == "user" {
        dto::saved_attachment_names(&item.text)
    } else {
        Vec::new()
    };
    dto::Item {
        proposal: (item.role == "plan")
            .then(|| dto::PlanProposal::from_text(&item.text))
            .flatten(),
        plan_steps: (item.role == "tool"
            && item.tool_name.as_deref() == Some("update_plan")
            && item.ok == Some(true))
        .then(|| wisp_dto::execution_plan::parse_plan_steps(&item.text)),
        call_id: item.call_id,
        kind: item.kind,
        locations: item.locations,
        role: item.role,
        text: item.text,
        tool_name: item.tool_name,
        input: item.input,
        ok: item.ok,
        status: item.status,
        duration_ms: item.duration_ms,
        model_name: item.model_name,
        timestamp: None,
        attachments,
        run: None,
    }
}

fn submitted_run_id(item: &dto::Item) -> Option<String> {
    if item.role != "tool"
        || item.ok != Some(true)
        || !matches!(
            item.tool_name.as_deref(),
            Some("run_in_context" | "wisp_run_in_context" | "transfer_between_contexts")
        )
    {
        return None;
    }
    let value: Value = serde_json::from_str(&item.text).ok()?;
    value
        .get("run_id")
        .or_else(|| value.get("id"))?
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
}

fn transcript_run_id(item: &dto::Item) -> Option<String> {
    if item.role == "tool"
        && matches!(
            item.tool_name.as_deref(),
            Some("monitor_run" | "wisp_monitor_run")
        )
    {
        let input = item.input.as_deref()?.trim();
        if input.is_empty() {
            return None;
        }
        return Some(input.to_owned());
    }
    submitted_run_id(item)
}

fn annotate_runs(items: &mut [dto::Item], runs: &[wisp_store::RunSummary], session: &str) {
    let mut owners = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        if let Some(id) = submitted_run_id(item) {
            owners.entry(id).or_insert(index);
        }
    }
    for item in items {
        item.run = transcript_run_id(item).and_then(|id| {
            let run = runs
                .iter()
                .find(|run| run.id == id && run.frame_id.as_deref() == Some(session))?;
            Some(dto::TranscriptRun {
                owner_index: owners.get(&id).copied(),
                id,
                status: run.status.as_str().into(),
                needs_review: run.status.is_terminal()
                    && run.kind == "ssh_direct"
                    && run.cleaned_at.is_none(),
            })
        });
    }
}

fn timestamp_items(
    items: &mut [dto::Item],
    user_offset: usize,
    outline: &[(i64, String, i64, Option<i64>)],
) {
    let mut turn = None;
    for item in items {
        if matches!(item.role.as_str(), "user" | "queued_user") {
            turn = Some(turn.map_or(user_offset, |index| index + 1));
        }
        let Some((_, _, sent_at, response_at)) = turn.and_then(|index| outline.get(index)) else {
            continue;
        };
        item.timestamp = match item.role.as_str() {
            "user" | "queued_user" => Some(*sent_at),
            "assistant" => *response_at,
            _ => None,
        }
        .filter(|value| *value > 0);
    }
}

/// Copy one local file into the named project's uploads directory and bind it
/// with the same message-resource snapshot used for Markdown file links.
/// Does not select a WebView window or send a turn.
pub(crate) async fn attach_local_file(
    store: &wisp_store::Store,
    root: &std::path::Path,
    project_id: &str,
    frame_id: &str,
    source: &std::path::Path,
) -> Result<dto::ComposerAttachment, String> {
    if source.as_os_str().is_empty() {
        return Err("An attachment path is required".into());
    }
    let (dest, relative, name) =
        crate::artifact_commands::copy_local_file_into_uploads(root, source)?;
    let markdown = format!("[{name}]({relative})");
    let links = crate::resource_refs::bind_new_message_resources(
        store, root, project_id, frame_id, 0, &markdown,
    )
    .await;
    if links.first().is_none_or(|link| link.status != "ready") {
        let _ = std::fs::remove_file(&dest);
        let error = links
            .first()
            .and_then(|link| link.error.clone())
            .unwrap_or_else(|| "Attachment could not be bound".into());
        return Err(error);
    }
    Ok(dto::ComposerAttachment {
        path: relative,
        name,
    })
}

fn follow_up_id(request_id: &str) -> Result<u64, String> {
    let id = uuid::Uuid::parse_str(request_id).map_err(|_| "Invalid follow-up request ID")?;
    let bytes = id.as_bytes();
    Ok(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
}

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

fn delete_session_arguments(value: &Value) -> Result<Value, String> {
    let args: dto::SessionRequest = decode(value)?;
    if args.before_seq.is_some() {
        return Err("Delete does not accept a history cursor".into());
    }
    Ok(json!({"id": args.session_id}))
}

fn require_acp_profile(profiles: &[crate::acp::AcpAgentProfile], id: &str) -> Result<(), String> {
    if id.trim().is_empty() || !profiles.iter().any(|profile| profile.id == id) {
        return Err("ACP Agent profile does not exist; refresh the model list".into());
    }
    Ok(())
}

fn turn_arguments(
    session: &str,
    message: &str,
    attachments: &[String],
    agent: Option<&str>,
    references: &[wisp_dto::ComposerReferenceArg],
) -> Value {
    json!({"sessionId":session,"message":message,"attachments":attachments,"acpAgentId":agent,"references":references})
}

pub(crate) async fn require_owner(
    store: &wisp_store::Store,
    project: &str,
    session: &str,
) -> Result<(), String> {
    if session.is_empty()
        || store
            .frame_project_id(session)
            .await
            .map_err(|e| e.to_string())?
            .as_deref()
            != Some(project)
    {
        return Err("Conversation does not belong to the selected project".into());
    }
    Ok(())
}

async fn owned_session_exists(
    store: &wisp_store::Store,
    project: &str,
    session: &str,
) -> Result<bool, String> {
    if session.is_empty() {
        return Err("A conversation is required".into());
    }
    match store
        .frame_project_id(session)
        .await
        .map_err(|error| error.to_string())?
    {
        None => Ok(false),
        Some(owner) if owner == project => Ok(true),
        Some(_) => Err("Conversation does not belong to the selected project".into()),
    }
}
async fn running(broker: &Broker, session: &str) -> bool {
    broker
        .app
        .state::<crate::AppState>()
        .running_turns
        .lock()
        .await
        .contains(session)
}
async fn call(broker: &Broker, project: &str, command: &str, args: Value) -> Result<Value, String> {
    invoke_command(broker, Some(project.to_owned()), command, args).await
}

pub(crate) async fn dispatch(broker: &Broker, request: &Request) -> Result<Value, String> {
    let project = request
        .project_id
        .as_deref()
        .filter(|v| !v.is_empty())
        .ok_or("A project is required")?;
    if request.command == "native_conversation_inbox" {
        if !request.args.as_object().is_some_and(|v| v.is_empty()) {
            return Err("Inbox takes no arguments".into());
        }
        let rows = call(
            broker,
            project,
            "search_sessions",
            json!({"query":"", "limit":50}),
        )
        .await?;
        let rows = rows
            .as_array()
            .ok_or("Invalid inbox response")?
            .iter()
            .filter(|row| row.get("status").and_then(Value::as_str) == Some("needs_you"))
            .cloned()
            .collect::<Vec<_>>();
        return Ok(Value::Array(rows));
    }
    if request.command == "native_conversation_create" {
        let args: dto::CreateRequest = decode(&request.args)?;
        if let Some(agent) = args.acp_agent_id.as_deref() {
            let state = broker.app.state::<crate::AppState>();
            require_acp_profile(&crate::acp::profiles(&state.store).await, agent)?;
        }
        let value = call(broker, project, "new_session", json!({})).await?;
        let id = value.as_str().ok_or("Invalid new conversation response")?;
        if let Some(agent) = args.acp_agent_id {
            broker
                .app
                .state::<crate::AppState>()
                .store
                .set_frame_acp_agent_selection(id, project, &agent)
                .await
                .map_err(|e| e.to_string())?;
        }
        return Ok(value);
    }
    let session = request
        .args
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or("A conversation is required")?;
    if request.command == "native_conversation_exists" {
        let args: dto::SessionRequest = decode(&request.args)?;
        if args.before_seq.is_some() {
            return Err("Existence check does not accept a history cursor".into());
        }
        return owned_session_exists(
            &broker.app.state::<crate::AppState>().store,
            project,
            session,
        )
        .await
        .map(Value::Bool);
    }
    require_owner(
        &broker.app.state::<crate::AppState>().store,
        project,
        session,
    )
    .await?;
    if request.command == "native_conversation_references" {
        return crate::native_composer::references(
            broker,
            project,
            session,
            decode(&request.args)?,
        )
        .await;
    }
    if request.command.starts_with("native_conversation_terminal_") {
        return crate::native_terminals::dispatch(broker, request, project, session).await;
    }
    if request.command.starts_with("native_conversation_panel_") {
        return crate::native_panels::dispatch(broker, request, project, session).await;
    }
    let acp_agent_id =
        crate::acp::session_agent_id(&broker.app.state::<crate::AppState>().store, session).await?;
    let record = broker.conversations.session(session).await?;
    match request.command.as_str() {
        "native_conversation_queue_action" => {
            crate::native_queue::dispatch(
                broker,
                project,
                decode(&request.args)?,
                acp_agent_id.is_some(),
            )
            .await
        }
        "native_conversation_history_action" => {
            let args: wisp_dto::native_history::HistoryRequest = decode(&request.args)?;
            let guard = record.lock().await;
            let native_running = guard.running;
            // Model calls may take minutes. Snapshot polling and the source
            // turn must remain live while a historical memory is proposed.
            if matches!(
                args.action,
                wisp_dto::native_history::HistoryAction::ProposeMemory
                    | wisp_dto::native_history::HistoryAction::Review
            ) {
                drop(guard);
                crate::native_history::dispatch(broker, project, args, native_running).await
            } else {
                let result =
                    crate::native_history::dispatch(broker, project, args, native_running).await;
                drop(guard);
                result
            }
        }
        "native_conversation_options" | "native_conversation_options_set" => {
            if request.command.ends_with("_set") {
                let args: dto::ComposerOptionRequest = decode(&request.args)?;
                let state = broker.app.state::<crate::AppState>();
                state
                    .store
                    .require_unarchived_session(session)
                    .await
                    .map_err(|e| e.to_string())?;
                let (_, scope) =
                    crate::exploration_commands::working_project_for_frame(&state, session).await?;
                crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
                let (command, payload) = args.change.command(session)?;
                call(broker, project, command, payload).await?;
            } else {
                let _: dto::SessionRequest = decode(&request.args)?;
            }
            let session_args = json!({"sessionId":session});
            let state = broker.app.state::<crate::AppState>();
            let options = dto::ComposerOptions {
                session_id: session.into(),
                full_permission: decode(
                    &call(
                        broker,
                        project,
                        "get_session_full_permission",
                        session_args.clone(),
                    )
                    .await?,
                )?,
                delegation: decode(
                    &call(
                        broker,
                        project,
                        "get_session_delegation_enabled",
                        session_args.clone(),
                    )
                    .await?,
                )?,
                completion: decode(
                    &call(
                        broker,
                        project,
                        "get_session_agent_completion",
                        session_args.clone(),
                    )
                    .await?,
                )?,
                auto_review: decode(
                    &call(broker, project, "get_auto_review_enabled", session_args).await?,
                )?,
                specialist: decode(
                    &call(
                        broker,
                        project,
                        "get_session_specialist",
                        json!({"frameId":session}),
                    )
                    .await?,
                )?,
                specialist_locked: state
                    .store
                    .load_messages(session)
                    .await
                    .map_err(|e| e.to_string())?
                    .iter()
                    .any(|m| m.role != wisp_llm::Role::System),
            };
            serde_json::to_value(options).map_err(|e| e.to_string())
        }
        "native_conversation_delete" => {
            call(
                broker,
                project,
                "delete_session",
                delete_session_arguments(&request.args)?,
            )
            .await
        }
        "native_conversation_pin" => {
            let args: dto::PinRequest = decode(&request.args)?;
            call(
                broker,
                project,
                "set_session_pinned",
                json!({"id":session,"pinned":args.pinned}),
            )
            .await
        }
        "native_conversation_rename" => {
            let args: dto::RenameRequest = decode(&request.args)?;
            call(
                broker,
                project,
                "rename_session",
                json!({"id":session,"title":args.title}),
            )
            .await
        }
        "native_conversation_share" => {
            let _: dto::SessionRequest = decode(&request.args)?;
            let state = broker.app.state::<crate::AppState>();
            let mut cursor = None;
            let mut pages = Vec::new();
            let mut bytes = 0usize;
            loop {
                let (items, next, _, _) =
                    crate::session_commands::native_transcript(&state, session, cursor).await?;
                let rows = items
                    .into_iter()
                    .filter(|item| {
                        matches!(item.role.as_str(), "user" | "assistant" | "reasoning")
                            && !item.text.trim().is_empty()
                    })
                    .map(|item| dto::ShareRow {
                        role: item.role,
                        text: item.text,
                    })
                    .collect::<Vec<_>>();
                bytes += rows.iter().map(|row| row.text.len()).sum::<usize>();
                if bytes > 16 * 1024 * 1024 {
                    return Err("Conversation exceeds the 16 MiB share limit".into());
                }
                pages.push(rows);
                if next.is_none() {
                    break;
                }
                if cursor.is_some_and(|old| next.unwrap() >= old) {
                    return Err("Transcript cursor did not advance".into());
                }
                cursor = next;
            }
            let rows = pages.into_iter().rev().flatten().collect::<Vec<_>>();
            serde_json::to_value(rows).map_err(|e| e.to_string())
        }
        "native_conversation_share_html" => {
            let args: dto::ShareExportRequest = decode(&request.args)?;
            crate::native_share::html(&args.rows, args.dark).map(Value::String)
        }

        "native_conversation_archive_get"
        | "native_conversation_archive_prepare"
        | "native_conversation_archive_retry"
        | "native_conversation_archive_continue" => {
            let _: dto::SessionRequest = decode(&request.args)?;
            let command = match request.command.as_str() {
                "native_conversation_archive_get" => "get_research_archive",
                "native_conversation_archive_prepare" => "prepare_research_archive",
                "native_conversation_archive_retry" => "retry_research_archive_cleanup",
                _ => "continue_research_archive",
            };
            call(broker, project, command, json!({"frameId": session})).await
        }
        "native_conversation_archive_confirm" => {
            let args: dto::ArchiveConfirmRequest = decode(&request.args)?;
            call(
                broker,
                project,
                "confirm_research_archive",
                json!({"frameId": session, "input": args.input}),
            )
            .await
        }

        "native_conversation_seen" => {
            let _: dto::SessionRequest = decode(&request.args)?;
            broker
                .app
                .state::<crate::AppState>()
                .store
                .mark_frame_seen(session)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "native_conversation_trajectory" | "native_conversation_trajectory_html" => {
            let _: dto::SessionRequest = decode(&request.args)?;
            let state = broker.app.state::<crate::AppState>();
            crate::session_commands::native_transcript(&state, session, None).await?;
            let snapshot =
                crate::session_commands::folded_session_trajectory(&state.store, session).await?;
            if request.command == "native_conversation_trajectory_html" {
                let exported_at = chrono::Utc::now().to_rfc3339();
                return Ok(Value::String(
                    crate::trajectory_export::render_trajectory_html(&snapshot, "zh", &exported_at),
                ));
            }
            let value = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
            let snapshot: wisp_dto::TrajectorySnapshotDto =
                serde_json::from_value(value).map_err(|e| e.to_string())?;
            serde_json::to_value(snapshot).map_err(|e| e.to_string())
        }
        "native_conversation_outline" => {
            let _: dto::SessionRequest = decode(&request.args)?;
            let state = broker.app.state::<crate::AppState>();
            // Flush pending persisted events before indexing, using the same
            // barrier as a transcript refresh.
            crate::session_commands::native_transcript(&state, session, None).await?;
            let rows = state
                .store
                .load_session_user_messages(session)
                .await
                .map_err(|e| e.to_string())?;
            let entries: Vec<dto::OutlineEntry> = rows
                .iter()
                .enumerate()
                .map(
                    |(index, (_, text, sent_at, response_at))| dto::OutlineEntry {
                        user_index: index,
                        text: text.clone(),
                        before_seq: rows.get(index + 1).map(|row| row.0),
                        sent_at: Some(*sent_at),
                        response_at: *response_at,
                    },
                )
                .collect();
            serde_json::to_value(entries).map_err(|e| e.to_string())
        }
        "native_conversation_snapshot" => {
            let args: dto::SessionRequest = decode(&request.args)?;
            let mut record = record.lock().await;
            let state = broker.app.state::<crate::AppState>();
            let (items, next_before_seq, frozen, user_offset) =
                crate::session_commands::native_transcript(&state, session, args.before_seq)
                    .await?;
            let mut model = call(
                broker,
                project,
                "get_session_model",
                json!({"sessionId":session}),
            )
            .await?;
            let binding = state
                .store
                .get_acp_session(session)
                .await
                .map_err(|e| e.to_string())?;
            if binding.is_none() {
                if let Some(agent) = &acp_agent_id {
                    model = json!(format!("acp:{agent}"));
                }
            }
            record.sequence += 1;
            let read_only = frozen;
            let mut items: Vec<_> = items.into_iter().map(snapshot_item).collect();
            let outline = state
                .store
                .load_session_user_messages(session)
                .await
                .map_err(|e| e.to_string())?;
            timestamp_items(&mut items, user_offset, &outline);
            let mut run_cards = Vec::new();
            if items.iter().any(|item| transcript_run_id(item).is_some()) {
                let (run_project, scope) =
                    crate::exploration_commands::working_project_for_frame(&state, session).await?;
                if run_project.id != project {
                    return Err("Project scope mismatch".into());
                }
                let runs = state
                    .store
                    .list_run_summaries_in_scope(&scope)
                    .await
                    .map_err(|e| e.to_string())?;
                annotate_runs(&mut items, &runs, session);
                let ids: std::collections::HashSet<_> = items
                    .iter()
                    .filter_map(|item| item.run.as_ref().map(|run| run.id.clone()))
                    .collect();
                for id in ids {
                    if let Some(run) = state.store.get_run(&id).await.map_err(|e| e.to_string())? {
                        if run.frame_id.as_deref() != Some(session) {
                            continue;
                        }
                        let mut card: wisp_dto::RunRecord = serde_json::from_value(
                            serde_json::to_value(&run).map_err(|e| e.to_string())?,
                        )
                        .map_err(|e| e.to_string())?;
                        for text in [&mut card.stdout_tail, &mut card.stderr_tail] {
                            if let Some(value) = text.as_mut() {
                                *value = value
                                    .chars()
                                    .rev()
                                    .take(16_384)
                                    .collect::<String>()
                                    .chars()
                                    .rev()
                                    .collect();
                            }
                        }
                        // Summary and output must describe one lifecycle, even
                        // when the run settles between the two database reads.
                        for item in &mut items {
                            if let Some(link) = item.run.as_mut().filter(|link| link.id == id) {
                                link.status = card.status.clone();
                                link.needs_review = run.status.is_terminal()
                                    && run.kind == "ssh_direct"
                                    && run.cleaned_at.is_none();
                            }
                        }
                        run_cards.push(card);
                    }
                }
                run_cards.sort_by(|a, b| a.id.cmp(&b.id));
            }
            let fast_mode = if frozen || binding.is_some() || acp_agent_id.is_some() {
                None
            } else if let Some(profile) =
                crate::models::profile_owned(&state.store, model.as_str().unwrap_or_default()).await
            {
                if crate::models::supports_fast_service_tier(&profile) {
                    let tier = state
                        .store
                        .frame_service_tier(session)
                        .await
                        .map_err(|e| e.to_string())?;
                    Some(dto::FastMode::from_tiers(
                        &profile.service_tier,
                        tier.as_deref(),
                    ))
                } else {
                    None
                }
            } else {
                None
            };
            let is_running = record.running || running(broker, session).await;
            let activity_status = crate::session_runtime_status(
                session,
                None,
                false,
                &if is_running {
                    [session.to_owned()].into()
                } else {
                    Default::default()
                },
                &state.awaiting_confirm.lock().unwrap(),
            )
            .to_owned();
            let snapshot = dto::Snapshot {
                run_cards,
                run_review_supported: Some(true),
                queue: Some(
                    crate::native_queue::snapshot(broker, session, acp_agent_id.is_some()).await,
                ),
                history_state: Some(
                    crate::native_history::snapshot(&state, project, session, &outline).await?,
                ),
                schema: dto::SCHEMA.into(),
                epoch: broker.conversations.epoch.clone(),
                sequence: record.sequence,
                project_id: project.into(),
                session_id: session.into(),
                items,
                next_before_seq,
                user_offset,
                running: is_running,
                activity_status: Some(activity_status),
                stopping: record.stopping,
                read_only,
                acp_state: if frozen || args.before_seq.is_some() {
                    None
                } else {
                    crate::acp::native_session_state(&state, session).await?
                },
                acp: if frozen {
                    None
                } else {
                    Some(crate::acp::native_interactions(&state, session).await)
                },
                model_id: model.as_str().unwrap_or_default().into(),
                composer_references: Some(true),
                follow_ups: if args.before_seq.is_some()
                    || record.running
                    || running(broker, session).await
                {
                    Vec::new()
                } else {
                    broker
                        .conversations
                        .suggestions
                        .lock()
                        .unwrap()
                        .get(session)
                        .cloned()
                        .unwrap_or_default()
                },
                fast_mode,
                plan_mode: if frozen || binding.is_some() || acp_agent_id.is_some() {
                    None
                } else {
                    Some(crate::plan_mode::session_plan_mode(&state.store, session).await)
                },
                acp_agent_id: binding.map(|binding| binding.agent_profile_id),
                request_id: record.request_id.clone(),
                error: record.error.clone(),
                approvals: state
                    .confirms
                    .lock()
                    .unwrap()
                    .get(session)
                    .map(|pending| wisp_dto::PendingToolApproval {
                        approval_id: pending.request.approval_id.clone(),
                        frame_id: session.into(),
                        message: pending.request.message.clone(),
                        tool: pending.request.tool.clone(),
                        preview: pending.request.preview.clone(),
                    })
                    .into_iter()
                    .collect(),
            };
            serde_json::to_value(snapshot).map_err(|e| e.to_string())
        }
        "native_conversation_attach" => {
            let args: dto::AttachRequest = decode(&request.args)?;
            let state = broker.app.state::<crate::AppState>();
            let (_, workspace) = state
                .store
                .get_project(project)
                .await
                .map_err(|error| error.to_string())?
                .ok_or("Project was not found")?;
            let attached = attach_local_file(
                &state.store,
                std::path::Path::new(&workspace),
                project,
                session,
                std::path::Path::new(args.path.trim()),
            )
            .await?;
            serde_json::to_value(attached).map_err(|error| error.to_string())
        }
        "native_conversation_enqueue" => {
            let args: dto::SendRequest = decode(&request.args)?;
            let id = follow_up_id(&args.request_id)?;
            let digest: [u8; 32] = Sha256::digest(
                serde_json::to_vec(&(&args.message, &args.attachments, &args.references))
                    .map_err(|e| e.to_string())?,
            )
            .into();
            let mut guard = record.lock().await;
            if let Some(previous) = guard.queued_requests.get(&args.request_id) {
                return if previous == &digest {
                    Ok(json!({"queued":true,"id":id.to_string()}))
                } else {
                    Err("Queued request ID was already used for another payload".into())
                };
            }
            if args.message.len() > 128 * 1024
                || args.attachments.len() > 64
                || guard.queued_requests.len() >= 1024
            {
                return Err("Queue payload or request ledger limit exceeded".into());
            }
            let state = broker.app.state::<crate::AppState>();
            state
                .store
                .require_unarchived_session(session)
                .await
                .map_err(|e| e.to_string())?;
            let scope = state
                .store
                .frame_state_scope(session)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("Session scope missing")?;
            crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
            if matches!(
                state
                    .store
                    .session_branch_state(session)
                    .await
                    .map_err(|e| e.to_string())?,
                Some("merged" | "orphaned")
            ) {
                return Err("Frozen conversation queues cannot be changed".into());
            }
            if acp_agent_id.is_some()
                && broker
                    .app
                    .state::<crate::AppState>()
                    .store
                    .get_acp_session(session)
                    .await
                    .map_err(|e| e.to_string())?
                    .is_none()
            {
                return Err(
                    "Wait for the ACP session to connect before queuing a follow-up".into(),
                );
            }
            let turn_running = guard.running || running(broker, session).await;
            let runtime = {
                let mut sessions = state.sessions.lock().await;
                sessions
                    .entry(session.to_owned())
                    .or_insert_with(|| std::sync::Arc::new(crate::SessionRuntime::new()))
                    .clone()
            };
            let text = crate::agent_turn::queue_follow_up_with_limit(
                turn_running,
                &runtime,
                id,
                &args.message,
                &args.attachments,
                &args.references,
                64,
            )?;
            guard
                .queued_requests
                .insert(args.request_id.clone(), digest);
            if !runtime
                .draining
                .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                // The hidden label is not the WebView's main window. The
                // shared driver drains the authoritative queue after each
                // current turn releases the workflow lock.
                crate::agent_turn::spawn_queue_driver(
                    broker.app.clone(),
                    runtime,
                    session.to_owned(),
                    "native-follow-up".into(),
                );
            }
            Ok(json!({"queued": true, "message": text, "id":id.to_string()}))
        }
        "native_conversation_send" => {
            let mut args: dto::SendRequest = decode(&request.args)?;
            args.attachments.retain(|path| !path.trim().is_empty());
            args.message = dto::message_with_attachments(&args.message, &args.attachments);
            let mut guard = record.lock().await;
            if guard.accept(&args, running(broker, session).await)? {
                let broker = broker.clone();
                let project = project.to_owned();
                let record = record.clone();
                let session = session.to_owned();
                let message = args.message.clone();
                let attachments = args.attachments.clone();
                let acp_agent_id = acp_agent_id.clone();
                let references = args.references.clone();
                tauri::async_runtime::spawn(async move {
                    let mut turn = Box::pin(call(
                        &broker,
                        &project,
                        "send_message",
                        turn_arguments(
                            &session,
                            &message,
                            &attachments,
                            acp_agent_id.as_deref(),
                            &references,
                        ),
                    ));
                    // The Stop request can precede creation of SessionRuntime.
                    // Keep cancelling until the turn settles, without dropping
                    // its persistence/cleanup future or affecting other sessions.
                    let result = loop {
                        tokio::select! {
                            result = &mut turn => break result,
                            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                                if record.lock().await.stopping {
                                    let _ = call(&broker, &project, "stop_agent", json!({"sessionId":session})).await;
                                }
                            }
                        }
                    };
                    let mut record = record.lock().await;
                    record.running = false;
                    record.stopping = false;
                    record.error = result.err();
                });
            }
            Ok(
                json!({"request_id":args.request_id,"session_id":session,"epoch":broker.conversations.epoch}),
            )
        }
        "native_conversation_stop" => {
            let _: dto::SessionRequest = decode(&request.args)?;
            record.lock().await.stopping = true;
            let result = call(broker, project, "stop_agent", json!({"sessionId":session})).await;
            let mut record = record.lock().await;
            if !record.running {
                record.stopping = false;
            }
            result
        }
        "native_conversation_approve" => {
            let args: dto::ApprovalRequest = decode(&request.args)?;
            crate::approval_commands::respond_native_confirmation(
                &broker.app.state::<crate::AppState>(),
                project,
                &args,
            )
            .await?;
            Ok(Value::Null)
        }
        "native_conversation_acp_setting" => {
            let args: dto::AcpSettingRequest = decode(&request.args)?;
            let record = record.lock().await;
            if record.running || running(broker, session).await {
                return Err("Wait for the current turn before changing ACP settings".into());
            }
            let state = broker.app.state::<crate::AppState>();
            state
                .store
                .require_unarchived_session(session)
                .await
                .map_err(|e| e.to_string())?;
            let (_, scope) =
                crate::exploration_commands::working_project_for_frame(&state, session).await?;
            crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
            let current = crate::acp::native_session_state(&state, session)
                .await?
                .ok_or("This conversation has no bound ACP session")?;
            let (command, payload) = args.change.command(&current)?;
            call(broker, project, command, payload).await
        }
        "native_conversation_acp_permission" => {
            let args: dto::AcpPermissionResponse = decode(&request.args)?;
            crate::acp::respond_native_permission(
                &broker.app.state::<crate::AppState>(),
                &broker.app,
                args,
            )
            .await?;
            Ok(Value::Null)
        }
        "native_conversation_acp_answer" => {
            let args: dto::AcpQuestionResponse = decode(&request.args)?;
            crate::acp::respond_native_question(
                &broker.app.state::<crate::AppState>(),
                &broker.app,
                args,
            )
            .await?;
            Ok(Value::Null)
        }
        "native_conversation_fast" => {
            let args: dto::FastRequest = decode(&request.args)?;
            let record = record.lock().await;
            if record.running || running(broker, session).await {
                return Err("Wait for the current turn before changing its service tier".into());
            }
            let state = broker.app.state::<crate::AppState>();
            if acp_agent_id.is_some()
                || state
                    .store
                    .get_acp_session(session)
                    .await
                    .map_err(|e| e.to_string())?
                    .is_some()
            {
                return Err("Fast is not available for this ACP Agent".into());
            }
            if crate::models::session_profile_id(&state.store, session).await != args.model_id {
                return Err("The conversation model changed; refresh before changing Fast".into());
            }
            let profile = crate::models::profile_owned(&state.store, &args.model_id)
                .await
                .ok_or("Model does not exist")?;
            if !crate::models::supports_fast_service_tier(&profile) {
                return Err("This model does not support Fast".into());
            }
            let tier = dto::FastMode::override_for(&profile.service_tier, args.enabled);
            call(
                broker,
                project,
                "set_session_service_tier",
                json!({"sessionId":session,"serviceTier":tier}),
            )
            .await?;
            let saved = state
                .store
                .frame_service_tier(session)
                .await
                .map_err(|e| e.to_string())?;
            if saved.as_deref() != tier {
                return Err(
                    "Service tier change was not confirmed; refresh before retrying".into(),
                );
            }
            serde_json::to_value(dto::FastMode::from_tiers(
                &profile.service_tier,
                saved.as_deref(),
            ))
            .map_err(|e| e.to_string())
        }
        "native_conversation_plan" => {
            let args: dto::PlanRequest = decode(&request.args)?;
            let record = record.lock().await;
            if record.running || running(broker, session).await {
                return Err("Wait for the current turn before changing its mode".into());
            }
            if acp_agent_id.is_some() {
                return Err("This ACP Agent owns its plan mode".into());
            }
            call(
                broker,
                project,
                "set_session_plan_mode",
                json!({"sessionId":session,"enabled":args.enabled}),
            )
            .await
        }
        "native_conversation_model" => {
            let args: dto::ModelRequest = decode(&request.args)?;
            let record = record.lock().await;
            if record.running || running(broker, session).await {
                return Err("Wait for the current turn before changing its model".into());
            }
            if acp_agent_id.is_some()
                || broker
                    .app
                    .state::<crate::AppState>()
                    .store
                    .get_acp_session(session)
                    .await
                    .map_err(|e| e.to_string())?
                    .is_some()
            {
                return Err("Start a new conversation to switch away from this ACP Agent".into());
            }
            let profiles = call(broker, project, "list_models", json!({})).await?;
            if !profiles.as_array().is_some_and(|rows| {
                rows.iter()
                    .any(|row| row["id"].as_str() == Some(&args.model_id))
            }) {
                return Err("Model does not exist".into());
            }
            call(
                broker,
                project,
                "set_active_model",
                json!({"sessionId":session,"id":args.model_id}),
            )
            .await
        }
        _ => Err("Unsupported native conversation command".into()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_plan_proposal_projection_requires_plan_role_and_keeps_raw_fallback() {
        let source = crate::UiItem {
            role: "plan".into(),
            text: r#"{"source":"native","entries":[{"content":"Inspect **samples**","priority":"high"}]}"#.into(),
            tool_name: None, ok: None, duration_ms: None, input: None,
            model_name: None, call_id: None, kind: None, status: None,
            locations: None, resources: vec![],
        };
        let item = super::snapshot_item(source.clone());
        let plan = item.proposal.unwrap();
        assert_eq!(plan.source, wisp_dto::PlanSource::Native);
        assert_eq!(plan.entries[0].content, "Inspect **samples**");
        assert_eq!(plan.entries[0].status, wisp_dto::PlanStatus::Pending);
        assert!(super::snapshot_item(crate::UiItem {
            role: "assistant".into(),
            ..source.clone()
        })
        .proposal
        .is_none());
        let invalid = super::snapshot_item(crate::UiItem {
            text: "unparsed plan".into(),
            ..source
        });
        assert!(invalid.proposal.is_none());
        assert_eq!(invalid.text, "unparsed plan");
    }

    #[test]
    fn native_structured_tool_projection_keeps_identity_and_only_accepted_plans() {
        let source = crate::UiItem {
            role: "tool".into(),
            text: "[x] Inspect\n[~] Analyze".into(),
            tool_name: Some("update_plan".into()),
            ok: Some(true),
            duration_ms: Some(42),
            input: Some("1/2 steps done".into()),
            model_name: None,
            call_id: Some("call-1".into()),
            kind: Some("execute".into()),
            status: Some("completed".into()),
            locations: Some("analysis.py:4".into()),
            resources: vec![],
        };
        let projected = super::snapshot_item(source.clone());
        assert_eq!(projected.call_id.as_deref(), Some("call-1"));
        assert_eq!(projected.locations.as_deref(), Some("analysis.py:4"));
        assert_eq!(
            projected.plan_steps.unwrap()[1].status,
            wisp_dto::execution_plan::PlanStatus::Running
        );
        for ok in [None, Some(false)] {
            assert!(super::snapshot_item(crate::UiItem {
                ok,
                ..source.clone()
            })
            .plan_steps
            .is_none());
        }
        let acp = super::snapshot_item(crate::UiItem {
            role: "acp_tool".into(),
            ..source
        });
        assert_eq!(acp.kind.as_deref(), Some("execute"));
        assert!(acp.plan_steps.is_none());
    }
    #[test]
    fn native_follow_ups_are_bounded_and_invalidated_by_the_owning_turn() {
        let conversations = super::Conversations::default();
        conversations.observe(&crate::AgentEvent::FollowUps {
            frame_id: "a".into(),
            questions: vec!["next".into(), "".into()],
        });
        conversations.observe(&crate::AgentEvent::FollowUps {
            frame_id: "b".into(),
            questions: vec!["other".into()],
        });
        assert_eq!(
            conversations.suggestions.lock().unwrap().get("a").unwrap(),
            &["next"]
        );
        conversations.observe(&crate::AgentEvent::User {
            frame_id: "a".into(),
            text: "new turn".into(),
            queue_id: None,
        });
        assert!(!conversations.suggestions.lock().unwrap().contains_key("a"));
        assert!(conversations.suggestions.lock().unwrap().contains_key("b"));
        conversations.observe(&crate::AgentEvent::FollowUps {
            frame_id: "a".into(),
            questions: vec!["next".into(); 20],
        });
        assert_eq!(
            conversations
                .suggestions
                .lock()
                .unwrap()
                .get("a")
                .unwrap()
                .len(),
            8
        );
    }
    #[test]
    fn transcript_run_projection_requires_exact_visible_session_owner() {
        use super::{annotate_runs, dto};
        let mut items: Vec<dto::Item> = serde_json::from_str(include_str!(
            "../../contracts/native-conversations/v1/transcript-runs.json"
        ))
        .unwrap();
        let activity: serde_json::Value = serde_json::from_str(include_str!(
            "../../contracts/native-conversations/v1/panel-activity.json"
        ))
        .unwrap();
        let mut run: wisp_store::RunSummary =
            serde_json::from_value(activity["runs"][0].clone()).unwrap();
        run.status = wisp_store::RunStatus::Succeeded;
        annotate_runs(&mut items, &[run.clone()], "session-a");
        assert_eq!(items[3].run.as_ref().unwrap().owner_index, Some(1));
        // Live stored state wins over successful tool calls and stale fixture metadata.
        run.status = wisp_store::RunStatus::Running;
        annotate_runs(&mut items, &[run.clone()], "session-a");
        assert_eq!(items[3].run.as_ref().unwrap().status, "running");
        run.status = wisp_store::RunStatus::Succeeded;
        run.kind = "ssh_direct".into();
        annotate_runs(&mut items, &[run.clone()], "session-a");
        assert!(items[3].run.as_ref().unwrap().needs_review);
        run.cleaned_at = Some(10);
        annotate_runs(&mut items, &[run.clone()], "session-a");
        assert!(!items[3].run.as_ref().unwrap().needs_review);
        // A paged-out submission is not replaced by a nearby tool or inferred title.
        items[1].tool_name = Some("shell".into());
        annotate_runs(&mut items, &[run.clone()], "session-a");
        assert!(items[1].run.is_none());
        assert_eq!(items[3].run.as_ref().unwrap().owner_index, None);
        items[1].tool_name = Some("run_in_context".into());
        items[1].ok = Some(false);
        annotate_runs(&mut items, &[run.clone()], "session-a");
        assert_eq!(items[3].run.as_ref().unwrap().owner_index, None);
        items[1].ok = Some(true);
        items[1].text = "not JSON".into();
        annotate_runs(&mut items, &[run.clone()], "session-a");
        assert_eq!(items[3].run.as_ref().unwrap().owner_index, None);
        run.frame_id = Some("other-session".into());
        annotate_runs(&mut items, &[run.clone()], "session-a");
        assert!(items.iter().all(|row| row.run.is_none()));
        run.frame_id = Some("session-a".into());
        run.id = "unrelated".into();
        annotate_runs(&mut items, &[run], "session-a");
        assert!(items.iter().all(|row| row.run.is_none()));
    }
    #[test]
    fn message_metadata_uses_owning_turn_and_page_offset() {
        let outline = vec![
            (1, "same".into(), 10, Some(12)),
            (5, "same".into(), 20, Some(25)),
            (9, "last".into(), 30, None),
        ];
        let mut items: Vec<crate::native_conversations::dto::Item> = [
            "assistant",
            "user",
            "tool",
            "assistant",
            "user",
            "assistant",
        ]
        .into_iter()
        .map(|role| serde_json::from_value(serde_json::json!({"role":role,"text":"same"})).unwrap())
        .collect();
        super::timestamp_items(&mut items, 1, &outline);
        assert_eq!(
            items.iter().map(|item| item.timestamp).collect::<Vec<_>>(),
            vec![None, Some(20), None, Some(25), Some(30), None]
        );
        assert!(items[0].duration_ms.is_none());
        assert!(items[0].model_name.is_none());
        let metadata: super::dto::Item = serde_json::from_value(serde_json::json!({
            "role":"tool", "text":"done", "duration_ms":1250, "model_name":"exact-model", "timestamp":20
        })).unwrap();
        assert_eq!(metadata.duration_ms, Some(1250));
        assert_eq!(
            serde_json::to_value(metadata).unwrap()["model_name"],
            "exact-model"
        );
    }
    #[test]
    fn deletion_uses_only_the_explicit_session_and_rejects_history_or_scope_overrides() {
        assert_eq!(
            super::delete_session_arguments(&serde_json::json!({"session_id":"s"})).unwrap(),
            serde_json::json!({"id":"s"})
        );
        for args in [
            serde_json::json!({}),
            serde_json::json!({"session_id":"s","before_seq":1}),
            serde_json::json!({"session_id":"s","project_id":"other"}),
            serde_json::json!({"session_id":"s","id":"other"}),
        ] {
            assert!(super::delete_session_arguments(&args).is_err());
        }
    }
    use super::*;
    #[test]
    fn acp_creation_requires_exact_profile_and_turn_routing_preserves_session() {
        let profiles = vec![crate::acp::AcpAgentProfile {
            id: "agent".into(),
            label: "Agent".into(),
            command: "fake-acp".into(),
            args: vec![],
        }];
        assert!(require_acp_profile(&profiles, "agent").is_ok());
        for id in ["", "agent-other", "missing", " agent "] {
            assert!(require_acp_profile(&profiles, id).is_err());
        }
        let new = turn_arguments(
            "new-session",
            "hello",
            &["uploads/a.csv".into()],
            Some("agent"),
            &[],
        );
        assert_eq!(
            new,
            json!({"sessionId":"new-session", "message":"hello", "attachments":["uploads/a.csv"], "acpAgentId":"agent", "references":[]})
        );
        let resume = turn_arguments("saved-session", "continue", &[], None, &[]);
        assert_eq!(resume["sessionId"], "saved-session");
        assert!(
            resume["acpAgentId"].is_null(),
            "Saved bindings are resolved by the shared send pipeline"
        );
    }
    #[test]
    fn sending_is_idempotent_and_rejects_ambiguous_id_reuse_and_busy_sessions() {
        let mut record = Record::default();
        let mut request = dto::SendRequest {
            session_id: "a".into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            message: "hello".into(),
            attachments: Vec::new(),
            references: Vec::new(),
        };
        assert!(record.accept(&request, false).unwrap());
        assert!(!record.accept(&request, false).unwrap());
        request
            .references
            .push(wisp_dto::ComposerReferenceArg::Artifact {
                id: "artifact-a".into(),
            });
        assert!(
            record.accept(&request, false).is_err(),
            "an accepted request ID cannot acquire another reference"
        );
        request.references.clear();
        request.message = "different".into();
        assert!(record.accept(&request, false).is_err());
        request.request_id = uuid::Uuid::new_v4().to_string();
        assert!(record.accept(&request, false).is_err());
        record.running = false;
        assert!(record.accept(&request, true).is_err());
        assert!(record.accept(&request, false).unwrap());
    }

    #[test]
    fn native_send_passes_references_to_the_shared_resolver() {
        let references = vec![wisp_dto::ComposerReferenceArg::Project {
            id: "source-project".into(),
        }];
        let args = turn_arguments("target-session", "inspect", &[], None, &references);
        assert_eq!(args["sessionId"], "target-session");
        assert_eq!(
            args["references"],
            json!([{"kind":"project", "id":"source-project"}])
        );
    }
    #[tokio::test]
    async fn session_ownership_is_required_even_for_read_and_stop() {
        let directory = std::env::temp_dir().join(format!("wisp_native_{}", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&directory.join("test.sqlite"))
            .await
            .unwrap();
        store
            .create_project("a", "A", &directory.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("s", "a", "OPERON", "model")
            .await
            .unwrap();
        assert!(require_owner(&store, "a", "s").await.is_ok());
        assert!(require_owner(&store, "b", "s").await.is_err());
        assert!(require_owner(&store, "a", "").await.is_err());
        assert!(require_owner(&store, "a", "missing").await.is_err());
        assert!(owned_session_exists(&store, "a", "s").await.unwrap());
        assert!(!owned_session_exists(&store, "a", "missing").await.unwrap());
        assert!(owned_session_exists(&store, "b", "s").await.is_err());
        assert!(owned_session_exists(&store, "a", "").await.is_err());
        drop(store);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn attach_binds_the_file_and_a_saved_message_shows_it() {
        let directory = std::env::temp_dir().join(format!("wisp_attach_{}", uuid::Uuid::new_v4()));
        let workspace = directory.join("project");
        let other = directory.join("other");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("keep.txt"), b"keep").unwrap();
        let source = directory.join("notes.csv");
        std::fs::write(&source, b"a,b\n1,2\n").unwrap();
        let store = wisp_store::Store::open(&directory.join("test.sqlite"))
            .await
            .unwrap();
        store
            .create_project("research-1", "Research", &workspace.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("session-a", "research-1", "OPERON", "model")
            .await
            .unwrap();
        let missing = attach_local_file(
            &store,
            &workspace,
            "research-1",
            "session-a",
            std::path::Path::new(""),
        )
        .await
        .unwrap_err();
        assert!(missing.contains("required"));
        let attached = attach_local_file(&store, &workspace, "research-1", "session-a", &source)
            .await
            .unwrap();
        assert_eq!(attached.path, "uploads/notes.csv");
        assert_eq!(attached.name, "notes.csv");
        assert_eq!(
            std::fs::read(workspace.join(&attached.path)).unwrap(),
            b"a,b\n1,2\n"
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"a,b\n1,2\n");
        assert!(other.join("keep.txt").is_file());
        let links = store
            .list_message_resource_links("session-a", 0, None)
            .await
            .unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].status, "ready");
        assert_eq!(links[0].display_name, "notes.csv");
        let message = dto::message_with_attachments("看看", &[attached.path.clone()]);
        store
            .append_message("session-a", 1, &wisp_llm::Message::user(&message))
            .await
            .unwrap();
        let saved = store.load_messages("session-a").await.unwrap();
        let item = snapshot_item(crate::UiItem {
            role: "user".into(),
            text: saved[0].content.as_text(),
            tool_name: None,
            ok: None,
            duration_ms: None,
            input: None,
            model_name: None,
            call_id: None,
            kind: None,
            status: None,
            locations: None,
            resources: Vec::new(),
        });
        assert_eq!(item.attachments, vec!["uploads/notes.csv".to_string()]);
        assert!(item.text.contains("uploads/notes.csv"));
        drop(store);
        let _ = std::fs::remove_dir_all(directory);
    }
}
