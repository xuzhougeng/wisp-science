//! Turn hooks: the one place that decides what runs when a user-visible turn
//! ends, for native and ACP sessions alike.
//!
//! - Stop (`run_stop`): before `Done`; may continue the turn once. Automatic
//!   review → one correction → one follow-up review.
//! - AfterTurn (`spawn_after_turn`): detached after `Done`; results arrive as
//!   events. Memory proposal and follow-up questions. Running here instead of
//!   in the webview's `Done` handler gives every origin (desktop, queue, IM,
//!   schedule, delegation) the same behavior.
//!
//! Side-model calls (reviewer, memory analyst, follow-ups) resolve their
//! backend in one place: `side_complete`.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HookId {
    AutoReview,
    MemoryProposal,
    FollowUps,
}

impl HookId {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AutoReview => "auto_review",
            Self::MemoryProposal => "memory_proposal",
            Self::FollowUps => "follow_ups",
        }
    }

    /// Every hook switch, read from its existing setting key.
    pub(crate) async fn enabled(self, store: &Store, frame_id: &str) -> bool {
        match self {
            Self::AutoReview => load_auto_review_enabled(store, frame_id).await,
            Self::MemoryProposal => load_memory_enabled(store).await,
            Self::FollowUps => store
                .get_setting("follow_up_questions")
                .await
                .ok()
                .flatten()
                .is_none_or(|value| value == "true"),
        }
    }
}

/// Facts about a finished turn that decide which hooks run.
pub(crate) struct TurnEnd<'a> {
    pub(crate) frame_id: &'a str,
    pub(crate) project_id: &'a str,
    /// Native `Completed` reports `None`; ACP reports its own reason.
    pub(crate) stop_reason: Option<&'a str>,
    pub(crate) resume: bool,
    /// A Reviewer specialist session is never reviewed by itself.
    pub(crate) reviewer_session: bool,
    /// Transcript index where this turn began (may be stale after compaction).
    pub(crate) turn_start: usize,
}

impl TurnEnd<'_> {
    /// The loop ended on its own: not cancelled, cut off, or refused.
    pub(crate) fn completed(&self) -> bool {
        self.stop_reason.is_none_or(|reason| reason == "end_turn")
    }

    fn runs_stop_hooks(&self) -> bool {
        self.completed() && !self.resume && !self.reviewer_session
    }
}

/// The only difference between native and ACP turns that a Stop hook sees.
pub(crate) enum TurnDriver<'a> {
    Native {
        agent: &'a mut Agent,
        output: &'a TauriOutput,
        model_label: &'a str,
    },
    Acp {
        state: &'a AppState,
        app: &'a AppHandle,
        project: &'a ActiveProject,
        frame_id: &'a str,
    },
}

impl TurnDriver<'_> {
    async fn transcript(&self) -> Result<Vec<Message>, String> {
        match self {
            Self::Native { agent, .. } => Ok(agent.ctx.messages.clone()),
            Self::Acp {
                state, frame_id, ..
            } => state
                .store
                .load_messages(frame_id)
                .await
                .map_err(|error| error.to_string()),
        }
    }

    fn emit(&self, event: AgentEvent) {
        match self {
            Self::Native { output, .. } => output.emit(event),
            Self::Acp { app, project, .. } => {
                emit_agent_event_in(app, event, Some(project.id.as_str()))
            }
        }
    }

    async fn model_label(&self) -> String {
        match self {
            Self::Native { model_label, .. } => model_label.to_string(),
            Self::Acp {
                state, frame_id, ..
            } => match state.store.get_acp_session(frame_id).await {
                Ok(Some(binding)) => acp::profile_label(&state.store, &binding.agent_profile_id)
                    .await
                    .unwrap_or_else(|| "ACP Agent".into()),
                _ => "ACP Agent".into(),
            },
        }
    }

    /// Run one more internal turn in the same session.
    async fn continue_turn(&mut self, prompt: &str, cancel: &AtomicBool) -> Result<(), String> {
        match self {
            Self::Native { agent, output, .. } => {
                agent.ctx.inject_user(prompt);
                let result = agent.run_resume(*output, Some(cancel), None).await;
                agent.ctx.clear_runtime_injections();
                result.map(|_| ()).map_err(|error| error.to_string())
            }
            Self::Acp {
                state,
                app,
                project,
                frame_id,
            } => acp::run_acp_internal_turn(state, app, project, frame_id, prompt)
                .await
                .map(|_| ()),
        }
    }
}

/// Stop hooks. Review one completed analysis turn, request at most one
/// correction, then verify the corrected transcript once. Review failures
/// never fail the user's original turn.
pub(crate) async fn run_stop(
    state: &AppState,
    app: &AppHandle,
    end: &TurnEnd<'_>,
    driver: &mut TurnDriver<'_>,
    cancel: &AtomicBool,
) {
    if !end.runs_stop_hooks() || !HookId::AutoReview.enabled(&state.store, end.frame_id).await {
        return;
    }
    let frame_id = end.frame_id;
    let msgs = match driver.transcript().await {
        Ok(msgs) => msgs,
        Err(error) => {
            tracing::warn!("load transcript for review failed for {frame_id}: {error}");
            return;
        }
    };
    // Compaction may replace the pre-turn context and make `turn_start` stale.
    // In that case the whole transcript is the only safe review window.
    let turn = msgs.get(end.turn_start..).unwrap_or(&msgs);
    if !review::should_auto_review(turn) {
        return;
    }
    if !state.reviewing.lock().unwrap().insert(frame_id.to_string()) {
        return;
    }

    driver.emit(AgentEvent::ReviewStarted {
        frame_id: frame_id.to_string(),
    });
    match generate_review(state, frame_id, &msgs, Some(cancel)).await {
        Err(error) => {
            tracing::warn!("automatic review failed for {frame_id}: {error}");
            driver.emit(AgentEvent::ReviewFailed {
                frame_id: frame_id.to_string(),
                message: error,
            });
        }
        Ok(mut report) => {
            persist_review(&state.store, frame_id, msgs.len(), &report).await;
            emit_review(app, frame_id, report.clone(), Some(end.project_id));
            if report.has_findings() {
                driver.emit(AgentEvent::CorrectionStarted {
                    frame_id: frame_id.to_string(),
                    model: driver.model_label().await,
                });
                let follow_up = match driver
                    .continue_turn(&review::correction_prompt(&report), cancel)
                    .await
                {
                    Err(error) => Err(format!("correction turn failed: {error}")),
                    Ok(()) => match driver.transcript().await {
                        Err(error) => Err(format!("load corrected transcript failed: {error}")),
                        Ok(corrected) => generate_review(state, frame_id, &corrected, Some(cancel))
                            .await
                            .map_err(|error| format!("follow-up review failed: {error}")),
                    },
                };
                match follow_up {
                    Ok(follow_up) => report = review::reconcile_follow_up(report, follow_up),
                    Err(message) => {
                        tracing::warn!("automatic review for {frame_id}: {message}");
                        driver.emit(AgentEvent::ReviewFailed {
                            frame_id: frame_id.to_string(),
                            message,
                        });
                        report.set_status("unaddressed");
                    }
                }
                let message_count = driver
                    .transcript()
                    .await
                    .map_or(msgs.len(), |messages| messages.len());
                persist_review(&state.store, frame_id, message_count, &report).await;
                emit_review(app, frame_id, report, Some(end.project_id));
            }
        }
    }
    state.reviewing.lock().unwrap().remove(frame_id);
}

const AFTER_TURN_HOOKS: [HookId; 2] = [HookId::FollowUps, HookId::MemoryProposal];

/// AfterTurn hooks, detached from the turn. A newer turn in the same session
/// (started or finished) supersedes these results before they are emitted.
pub(crate) fn spawn_after_turn(app: &AppHandle, end: &TurnEnd<'_>) {
    if !end.completed() {
        return;
    }
    let app = app.clone();
    let frame_id = end.frame_id.to_string();
    let project_id = end.project_id.to_string();
    let generation = {
        let state = app.state::<AppState>();
        let mut generations = state.after_turn_generations.lock().unwrap();
        let generation = generations.entry(frame_id.clone()).or_default();
        *generation += 1;
        *generation
    };
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let state = state.inner();
        for hook in AFTER_TURN_HOOKS {
            if !after_turn_current(state, &frame_id, generation).await {
                return;
            }
            if !hook.enabled(&state.store, &frame_id).await {
                continue;
            }
            let event = match hook {
                HookId::FollowUps => follow_up_questions(state, &frame_id)
                    .await
                    .map(|questions| {
                        questions.map(|questions| AgentEvent::FollowUps {
                            frame_id: frame_id.clone(),
                            questions,
                        })
                    }),
                HookId::MemoryProposal => {
                    memory_commands::automatic_turn_memory_proposal(state, &frame_id)
                        .await
                        .map(|proposal| {
                            proposal.map(|proposal| AgentEvent::MemoryProposal {
                                frame_id: frame_id.clone(),
                                proposal,
                            })
                        })
                }
                HookId::AutoReview => unreachable!("auto review is a Stop hook"),
            };
            if !after_turn_current(state, &frame_id, generation).await {
                return;
            }
            let event = match event {
                Ok(Some(event)) => event,
                Ok(None) => continue,
                Err(message) => {
                    tracing::warn!("{} hook failed for {frame_id}: {message}", hook.as_str());
                    AgentEvent::HookFailed {
                        frame_id: frame_id.clone(),
                        hook: hook.as_str().into(),
                        message,
                    }
                }
            };
            emit_agent_event_in(&app, event, Some(project_id.as_str()));
        }
    });
}

async fn after_turn_current(state: &AppState, frame_id: &str, generation: u64) -> bool {
    let latest = state
        .after_turn_generations
        .lock()
        .unwrap()
        .get(frame_id)
        .copied();
    latest == Some(generation) && !state.running_turns.lock().await.contains(frame_id)
}

/// The turn ended on an answer: an `attempt_completion` result or assistant
/// prose. A turn cut off after a tool call or parked on `ask_user` has none.
fn latest_turn_has_final_answer(msgs: &[Message]) -> bool {
    msgs.iter()
        .rev()
        .find(|message| message.role != wisp_llm::Role::System)
        .is_some_and(|message| {
            let answer = match message.role {
                wisp_llm::Role::Assistant => true,
                wisp_llm::Role::Tool => message.tool_name.as_deref() == Some("attempt_completion"),
                _ => false,
            };
            answer && !message.content.as_text().trim().is_empty()
        })
}

async fn follow_up_questions(
    state: &AppState,
    frame_id: &str,
) -> Result<Option<Vec<String>>, String> {
    let messages = state
        .store
        .load_recent_turn_preview_messages(frame_id, FOLLOW_UP_TRANSCRIPT_TURNS)
        .await
        .map_err(|error| error.to_string())?;
    if !latest_turn_has_final_answer(&messages) {
        return Ok(None);
    }
    let completion = side_complete(
        state,
        frame_id,
        "follow_ups",
        SideModel::Session { max_tokens: 512 },
        "Suggest exactly three concise, useful questions the user could ask next. Return only a JSON array of three strings. Do not answer them.",
        &review::serialize_transcript(&messages),
        None,
    )
    .await?;
    parse_follow_up_questions(&completion.text).map(Some)
}

/// Which configured model answers a side question about a session.
pub(crate) enum SideModel {
    /// The built-in Reviewer's backend: an HTTP model, or a throwaway
    /// read-only ACP session in the session's project (or `project_root`).
    Reviewer {
        reviewer: specialists::Specialist,
        backend: Option<review::ReviewBackendConfig>,
        project_root: Option<PathBuf>,
    },
    /// The session specialist's model when it has one, else the session model,
    /// with output capped at `max_tokens`.
    Session { max_tokens: u64 },
}

impl SideModel {
    /// The built-in Reviewer with the backend it resolves to for this session.
    pub(crate) async fn reviewer(state: &AppState, frame_id: &str) -> Result<Self, String> {
        let reviewer = specialists::get(&state.store, "reviewer")
            .await
            .ok_or_else(|| "Reviewer specialist missing.".to_string())?;
        let session_acp_profile_id = state
            .store
            .get_acp_session(frame_id)
            .await
            .map_err(|error| error.to_string())?
            .map(|binding| binding.agent_profile_id);
        let backend = resolve_review_backend(&reviewer, session_acp_profile_id.as_deref());
        Ok(Self::Reviewer {
            reviewer,
            backend,
            project_root: None,
        })
    }
}

pub(crate) struct SideCompletion {
    pub(crate) text: String,
    /// `http_model` or `acp_agent`.
    pub(crate) backend: &'static str,
    pub(crate) model: String,
    pub(crate) effort: String,
}

/// One tool-free completion outside the session transcript.
pub(crate) async fn side_complete(
    state: &AppState,
    frame_id: &str,
    purpose: &str,
    model: SideModel,
    system: &str,
    user: &str,
    cancel: Option<&AtomicBool>,
) -> Result<SideCompletion, String> {
    let (settings, selected_profile, cap) = match model {
        SideModel::Reviewer {
            backend: Some(review::ReviewBackendConfig::AcpAgent { profile_id }),
            project_root,
            ..
        } => {
            if profile_id.trim().is_empty() {
                return Err("Reviewer ACP Agent is not configured.".into());
            }
            let project_root = match project_root {
                Some(root) => root,
                None => frame_project_root(state, frame_id).await?,
            };
            let label = acp::profile_label(&state.store, &profile_id)
                .await
                .ok_or_else(|| "The Reviewer ACP Agent profile no longer exists.".to_string())?;
            log_dev_llm_dispatch(
                frame_id,
                &format!("{purpose}_acp"),
                &profile_id,
                &label,
                &label,
                false,
            );
            let text = acp::acp_read_only_once(
                state,
                &project_root,
                &profile_id,
                &format!("{system}\n\n{user}"),
                cancel,
            )
            .await?;
            return Ok(SideCompletion {
                text,
                backend: "acp_agent",
                model: label,
                effort: String::new(),
            });
        }
        SideModel::Reviewer {
            mut reviewer,
            backend,
            ..
        } => {
            if let Some(review::ReviewBackendConfig::HttpModel { profile_id }) = backend {
                reviewer.model_id = profile_id;
            }
            let selected = if reviewer.model_id.trim().is_empty() {
                "active".to_string()
            } else {
                reviewer.model_id.clone()
            };
            (
                specialists::specialist_llm(&state.store, &reviewer).await,
                selected,
                None,
            )
        }
        SideModel::Session { max_tokens } => {
            match specialists::session_specialist(&state.store, frame_id).await {
                Some(specialist) if !specialist.model_id.trim().is_empty() => (
                    specialists::specialist_llm(&state.store, &specialist).await,
                    specialist.model_id,
                    Some(max_tokens),
                ),
                _ => (
                    load_session_settings(&state.store, frame_id).await,
                    "session".to_string(),
                    Some(max_tokens),
                ),
            }
        }
    };
    let (
        provider,
        api_url,
        configured_model,
        api_key,
        max_tokens,
        reasoning_effort,
        service_tier,
        user_agent,
        send_user_agent,
        send_session_id,
        session_header_name,
    ) = settings;
    let llm = wisp_llm::build(build_provider_config(
        &provider,
        &api_url,
        &api_key,
        &configured_model,
        cap.map_or(max_tokens, |cap| max_tokens.min(cap)),
        &reasoning_effort,
        &service_tier,
        &user_agent,
        send_user_agent,
        send_session_id,
        &session_header_name,
        Some(frame_id),
    )?);
    let actual_model = llm.model().to_string();
    log_dev_llm_dispatch(
        frame_id,
        &format!("{purpose}_http"),
        &selected_profile,
        &configured_model,
        &actual_model,
        false,
    );
    let completion = llm
        .complete(&[Message::system(system), Message::user(user)], &[])
        .await
        .map_err(|error| error.to_string())?;
    Ok(SideCompletion {
        text: completion.content,
        backend: "http_model",
        model: actual_model,
        effort: reasoning_effort.trim().to_string(),
    })
}

async fn frame_project_root(state: &AppState, frame_id: &str) -> Result<PathBuf, String> {
    let project_id = state
        .store
        .frame_project_id(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Session project was not found.".to_string())?;
    Ok(project_commands::load_active_project(state, &project_id)
        .await?
        .0
        .root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(stop_reason: Option<&str>, resume: bool, reviewer_session: bool) -> TurnEnd<'_> {
        TurnEnd {
            frame_id: "f",
            project_id: "p",
            stop_reason,
            resume,
            reviewer_session,
            turn_start: 0,
        }
    }

    #[test]
    fn native_and_acp_completion_share_one_gate() {
        assert!(end(None, false, false).completed());
        assert!(end(Some("end_turn"), false, false).completed());
        for cut in [
            "max_iterations",
            "max_tokens",
            "refusal",
            "cancelled",
            "compact",
        ] {
            let turn = end(Some(cut), false, false);
            assert!(!turn.completed(), "{cut}");
            assert!(!turn.runs_stop_hooks(), "{cut}");
        }
    }

    #[test]
    fn stop_hooks_skip_resumed_turns_and_the_reviewer_itself() {
        assert!(end(None, false, false).runs_stop_hooks());
        assert!(!end(None, true, false).runs_stop_hooks());
        assert!(!end(Some("end_turn"), false, true).runs_stop_hooks());
    }

    #[test]
    fn final_answer_is_prose_or_attempt_completion_not_a_trailing_tool() {
        let user = Message::user("go");
        assert!(latest_turn_has_final_answer(&[
            user.clone(),
            Message::assistant("Here is the answer."),
        ]));
        assert!(latest_turn_has_final_answer(&[
            user.clone(),
            Message::tool("c1", "attempt_completion", "Done: 3 samples."),
        ]));
        // Cut off after a tool call: the preceding prose was commentary.
        assert!(!latest_turn_has_final_answer(&[
            user.clone(),
            Message::assistant("I will record the decision first."),
            Message::tool("c2", "research_graph", "{\"node_id\":\"decision-1\"}"),
        ]));
        assert!(!latest_turn_has_final_answer(&[
            user.clone(),
            Message::tool("c3", "ask_user", "{}"),
        ]));
        assert!(!latest_turn_has_final_answer(&[
            user,
            Message::assistant("  "),
        ]));
        assert!(!latest_turn_has_final_answer(&[]));
    }
}
