use crate::{
    config::Config,
    output::{OutcomeEvent, PersistedMessage, ServerOutput, Session},
};
use anyhow::Result;
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
};
use wisp_core::{Agent, MemoryManager};
use wisp_dto::native_conversations::{Item, SendRequest};
use wisp_llm::{Message, Role};
use wisp_runs::{CancelRunTool, GetRunTool, HarvestRunTool, MonitorRunTool, RunInContextTool};
use wisp_skills::SkillIndex;
use wisp_store::{OpenContextEpoch, Store};

pub async fn build(
    config: &Config,
    store: &Store,
    runs: wisp_runs::RunManager,
    project: &str,
    root: &str,
    frame: &str,
    session: &Session,
    models: &crate::ModelFactory,
) -> Result<Agent> {
    let root = PathBuf::from(root);
    let mut paths = config.skills.clone();
    paths.push(root.join(".wisp/skills"));
    paths.push(root.join("skills"));
    let skills = Arc::new(SkillIndex::load(&paths));
    let cfg = config.provider.clone().with_session_id(frame);
    let mut tools =
        wisp_core::build_registry(skills.clone(), Arc::new(MemoryManager::new(&root)), true);
    tools.add(Box::new(wisp_core::ExploreTool::from_config(
        cfg.clone(),
        config.max_context,
    )));
    let mut agent = Agent::from_parts(
        models(cfg),
        None,
        tools,
        wisp_core::ContextManager::new(config.max_context),
        root.clone(),
        config.max_iterations,
        root.join(".wisp/server-sessions")
            .join(format!("{frame}.json")),
    );
    agent.ctx.messages = store.load_messages(frame).await?;
    agent.ctx.supports_vision = config.supports_vision;
    let repaired = wisp_core::repair_unpaired_tool_calls(&mut agent.ctx.messages);
    agent.seed_system_prompt(&skills, Some(wisp_runs::cli_compute_section(store).await));
    if repaired > 0 {
        store
            .open_context_epoch(
                frame,
                OpenContextEpoch {
                    messages: &agent.ctx.messages,
                    strategy: "recovery",
                    kind: "prune_only",
                    before_tokens: 0,
                    after_tokens: 0,
                    checkpoint_index: None,
                    first_kept_seq: None,
                    archive_ref: None,
                    ui_event_seq: None,
                },
            )
            .await?;
    } else if store.max_message_seq(frame).await? == 0 {
        for (i, message) in agent.ctx.messages.iter().enumerate() {
            store.append_message(frame, i as i64 + 1, message).await?;
        }
    } else if let Some(system) = agent
        .ctx
        .messages
        .first()
        .filter(|m| m.role == Role::System)
    {
        store.replace_system_message(frame, system).await?;
    }
    let scope = wisp_store::StateScope::mainline(project.to_string());
    agent.add_tool(Box::new(RunInContextTool::new(
        store.clone(),
        runs.clone(),
        project.into(),
        Some(frame.into()),
    )));
    agent.add_tool(Box::new(MonitorRunTool::new_in_scope(
        store.clone(),
        scope.clone(),
    )));
    agent.add_tool(Box::new(GetRunTool::new_in_scope(
        store.clone(),
        scope.clone(),
    )));
    agent.add_tool(Box::new(CancelRunTool::new_in_scope(
        store.clone(),
        runs.clone(),
        scope.clone(),
    )));
    agent.add_tool(Box::new(HarvestRunTool::new_in_scope(
        store.clone(),
        runs,
        scope,
    )));
    let worker = std::env::var_os("WISP_KERNEL_WORKER")
        .map(PathBuf::from)
        .or_else(wisp_runtime::bundled_worker_path)
        .unwrap_or_default();
    let r_worker = std::env::var_os("WISP_R_KERNEL_WORKER")
        .map(PathBuf::from)
        .or_else(wisp_runtime::bundled_r_worker_path)
        .unwrap_or_default();
    let runtime = session
        .runtime
        .lock()
        .unwrap()
        .get_or_insert_with(|| {
            wisp_runtime::RuntimeManager::local(
                config.data.join("runtimes").join(frame),
                worker.clone(),
                Some(r_worker.clone()),
                vec![],
            )
        })
        .clone();
    if worker.is_file() {
        agent.add_tool(Box::new(wisp_runtime::ReplTool::new(
            runtime.clone(),
            root.to_string_lossy(),
        )));
    }
    if r_worker.is_file() {
        agent.add_tool(Box::new(wisp_runtime::RTool::new(
            runtime,
            root.to_string_lossy(),
        )));
    }
    Ok(agent)
}

pub async fn run(
    store: Store,
    mut agent: Agent,
    session: Arc<Session>,
    project: String,
    request: SendRequest,
) -> Result<()> {
    let start_seq = store.max_message_seq(&request.session_id).await?;
    let mut next_event = store.next_session_ui_event_seq(&request.session_id).await?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let writer = store.clone();
    let frame = request.session_id.clone();
    let persist_session = session.clone();
    let persist = tokio::spawn(async move {
        let result = wisp_store::persist_seq_loop(start_seq, rx, |seq, entry: PersistedMessage| {
            let writer = writer.clone();
            let frame = frame.clone();
            let event_seq = next_event;
            if entry.outcome.is_some() {
                next_event += 1;
            }
            async move {
                writer.append_message(&frame, seq, &entry.message).await?;
                if let Some(outcome) = entry.outcome {
                    let event = serde_json::to_string(&OutcomeEvent {
                        kind: "ServerToolResult".into(),
                        outcome,
                    })?;
                    writer
                        .append_session_ui_event(&frame, event_seq, &event)
                        .await?;
                }
                Ok::<(), anyhow::Error>(())
            }
        })
        .await;
        if result.is_err() {
            persist_session.stop();
        }
        result
    });
    let out = ServerOutput {
        session: session.clone(),
        frame: request.session_id.clone(),
        project,
        turn: request.request_id,
        persist: tx,
    };
    let result = agent
        .run(&request.message, &out, Some(&session.cancel))
        .await;
    drop(out);
    wisp_store::join_or_abort_persist(persist, std::time::Duration::from_secs(10)).await??;
    if let Some(compacted) = agent.ctx.last_compaction() {
        store
            .open_context_epoch(
                &request.session_id,
                OpenContextEpoch {
                    messages: &agent.ctx.messages,
                    strategy: "auto",
                    kind: match compacted.kind {
                        wisp_core::CompactionKind::PruneOnly => "prune_only",
                        wisp_core::CompactionKind::Semantic => "semantic",
                    },
                    before_tokens: compacted.before,
                    after_tokens: compacted.after,
                    checkpoint_index: compacted.checkpoint_index,
                    first_kept_seq: None,
                    archive_ref: Some(&compacted.archive_reference),
                    ui_event_seq: None,
                },
            )
            .await?;
    }
    result?;
    anyhow::ensure!(
        !session.cancel.load(Ordering::Relaxed),
        "Turn stopped by user"
    );
    Ok(())
}

fn bounded(text: &str) -> String {
    const LIMIT: usize = 128 * 1024;
    if text.len() <= LIMIT {
        return text.into();
    }
    let mut end = LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[Preview truncated; full output is saved on the server]",
        &text[..end]
    )
}

pub fn item(role: &str, text: &str) -> Item {
    Item {
        role: role.into(),
        text: bounded(text),
        resources: vec![],
        proposal: None,
        call_id: None,
        kind: None,
        locations: None,
        plan_steps: None,
        tool_name: None,
        input: None,
        ok: None,
        status: None,
        duration_ms: None,
        model_name: None,
        timestamp: None,
        attachments: vec![],
        run: None,
        // The server folds raw messages without persisted seqs; remote
        // clients keep their existing offset keys until that is wired.
        seq: None,
    }
}

pub fn items(messages: &[Message]) -> Vec<Item> {
    let mut items = Vec::new();
    for message in messages {
        match message.role {
            Role::System => {}
            Role::User => items.push(item("user", &message.content.as_text())),
            Role::Assistant => {
                let text = message.content.as_text();
                if !text.is_empty() {
                    items.push(item("assistant", &text));
                }
                for call in &message.tool_calls {
                    let mut row = item("tool", "");
                    row.call_id = Some(call.id.clone());
                    row.tool_name = Some(call.function.name.clone());
                    row.input = Some(bounded(&call.function.arguments));
                    row.status = Some("running".into());
                    items.push(row);
                }
            }
            Role::Tool => {
                let index = items
                    .iter()
                    .rposition(|item| item.role == "tool" && item.call_id == message.tool_call_id);
                let row = match index {
                    Some(index) => &mut items[index],
                    None => {
                        let mut row = item("tool", "");
                        row.tool_name = message.tool_name.clone();
                        row.call_id = message.tool_call_id.clone();
                        items.push(row);
                        items.last_mut().unwrap()
                    }
                };
                row.text = bounded(&message.content.as_text());
                row.status = Some("complete".into());
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transcript_hides_system_messages_and_preserves_unicode() {
        let rows = items(&[
            Message::system("private instructions"),
            Message::user("你好"),
            Message::assistant("完成"),
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text, "你好");
        assert!(bounded(&"中".repeat(50000)).ends_with("server]"));
    }
}
