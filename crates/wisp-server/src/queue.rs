//! Messages written while a turn runs, kept by the host for their
//! conversation and sent one at a time as turns end, as the WebView's
//! backend queue does: they go on while the conversation is not open, in
//! any window. They are kept in memory: quitting the app drops them.
//!
//! The next one waits while the last turn failed, or while it ended on a
//! question or a plan for the user, so it is never taken for the answer.

use std::collections::VecDeque;

use anyhow::{ensure, Result};
use wisp_dto::native_conversations::SendRequest;

use crate::Server;

/// How many messages may wait in one conversation.
pub const LIMIT: usize = 10;

/// A message waiting for its conversation's turn to end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedMessage {
    /// Names it while it waits.
    pub id: u64,
    pub message: String,
    /// Project files it carries, as a message's attachments.
    pub attachments: Vec<String>,
    /// What it refers to, as a message's references.
    pub references: Vec<wisp_dto::ComposerReferenceArg>,
}

impl QueuedMessage {
    /// The message as a turn of `session`'s, with a request id of its own.
    fn request(&self, session: &str) -> SendRequest {
        SendRequest {
            session_id: session.to_owned(),
            request_id: uuid::Uuid::new_v4().to_string(),
            message: self.message.clone(),
            resume: false,
            resume_after: None,
            attachments: self.attachments.clone(),
            references: self.references.clone(),
            acp_agent_id: None,
        }
    }
}

/// What waits in one conversation, first to go first.
#[derive(Default)]
pub(crate) struct Waiting {
    /// The project the conversation is in, from the first message queued.
    project: String,
    messages: VecDeque<QueuedMessage>,
    next_id: u64,
}

impl Waiting {
    /// Adds a message at the end. It needs words or files, and there is a
    /// limit to how many wait.
    pub(crate) fn push(
        &mut self,
        project: &str,
        message: &str,
        attachments: Vec<String>,
        references: Vec<wisp_dto::ComposerReferenceArg>,
    ) -> Result<u64> {
        let message = message.trim();
        ensure!(
            !message.is_empty() || !attachments.is_empty(),
            "Write a message first"
        );
        ensure!(
            self.messages.len() < LIMIT,
            "{LIMIT} messages are already waiting; remove one or wait for the turn to end"
        );
        self.project = project.to_owned();
        self.next_id += 1;
        self.messages.push_back(QueuedMessage {
            id: self.next_id,
            message: message.to_owned(),
            attachments,
            references,
        });
        Ok(self.next_id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub(crate) fn list(&self) -> Vec<QueuedMessage> {
        self.messages.iter().cloned().collect()
    }

    /// Takes one out, wherever it stands.
    pub(crate) fn take(&mut self, id: u64) -> Option<QueuedMessage> {
        let index = self.messages.iter().position(|queued| queued.id == id)?;
        self.messages.remove(index)
    }

    /// Moves one a place earlier; the first has nowhere to go.
    pub(crate) fn move_up(&mut self, id: u64) -> bool {
        match self.messages.iter().position(|queued| queued.id == id) {
            Some(index) if index > 0 => {
                self.messages.swap(index - 1, index);
                true
            }
            _ => false,
        }
    }

    /// The next to go, with its project.
    pub(crate) fn next(&mut self) -> Option<(String, QueuedMessage)> {
        let next = self.messages.pop_front()?;
        Some((self.project.clone(), next))
    }

    /// Puts one back first in line: it was taken to be sent and was not.
    pub(crate) fn put_back(&mut self, queued: QueuedMessage) {
        self.messages.push_front(queued);
    }
}

/// Whether a turn that ended with `turn` waits for the user: its last
/// message that says anything is a question it asked or a plan it proposed.
pub(crate) fn asks(turn: &[wisp_llm::Message]) -> bool {
    turn.iter()
        .rev()
        .find(|message| {
            message.role != wisp_llm::Role::Assistant
                || !message.content.as_text().trim().is_empty()
        })
        .is_some_and(|message| {
            message.role == wisp_llm::Role::Tool
                && matches!(
                    message.tool_name.as_deref(),
                    Some(wisp_tools::ask_user::ASK_USER | wisp_tools::plan::PROPOSE_PLAN)
                )
        })
}

impl Server {
    /// Keeps a message for when the conversation's turn ends; it is sent
    /// at once if none runs and nothing holds the queue.
    pub async fn queue_message(&self, project: &str, request: SendRequest) -> Result<u64> {
        self.own_conversation(project, &request.session_id).await?;
        // Nothing waits to go into a branch frozen as history: it would
        // only be refused when its turn came.
        self.ensure_not_frozen_branch(&request.session_id).await?;
        let id = self
            .session(&request.session_id)
            .queue
            .lock()
            .unwrap()
            .push(
                project,
                &request.message,
                request.attachments,
                request.references,
            )?;
        self.drain_later(&request.session_id);
        Ok(id)
    }

    /// What waits in a conversation, first to go first.
    pub async fn queued_messages(
        &self,
        project: &str,
        session: &str,
    ) -> Result<Vec<QueuedMessage>> {
        self.own_conversation(project, session).await?;
        Ok(self.session(session).queue.lock().unwrap().list())
    }

    /// Takes a waiting message back out; `None` when it has gone already.
    pub async fn take_queued(
        &self,
        project: &str,
        session: &str,
        id: u64,
    ) -> Result<Option<QueuedMessage>> {
        self.own_conversation(project, session).await?;
        Ok(self.session(session).queue.lock().unwrap().take(id))
    }

    /// Moves a waiting message a place earlier.
    pub async fn move_queued_up(&self, project: &str, session: &str, id: u64) -> Result<bool> {
        self.own_conversation(project, session).await?;
        Ok(self.session(session).queue.lock().unwrap().move_up(id))
    }

    /// Gives a waiting message of words alone to the running turn, which
    /// reads it at its next step. When the turn no longer reads, it stays
    /// in line and the reply is false.
    pub async fn guide_with_queued(&self, project: &str, session: &str, id: u64) -> Result<bool> {
        self.own_conversation(project, session).await?;
        let running = self.session(session);
        let Some(text) = running
            .queue
            .lock()
            .unwrap()
            .list()
            .into_iter()
            .find(|queued| queued.id == id && queued.attachments.is_empty())
            .map(|queued| queued.message)
        else {
            return Ok(false);
        };
        if !self.guide(project, session, &text).await? {
            return Ok(false);
        }
        running.queue.lock().unwrap().take(id);
        Ok(true)
    }

    /// Sends the next waiting message now, though the last turn failed or
    /// asked something: the user said so. False when a turn runs or none
    /// waits; a message the host refuses keeps its place.
    pub async fn send_queued_now(&self, project: &str, session: &str) -> Result<bool> {
        self.own_conversation(project, session).await?;
        let waiting = self.session(session);
        if waiting.live.lock().unwrap().running {
            return Ok(false);
        }
        let Some((_, next)) = waiting.queue.lock().unwrap().next() else {
            return Ok(false);
        };
        let root = self.project_root(project).await?;
        let request = next.request(session);
        match self
            .start_turn(
                project,
                &root,
                request,
                crate::channels::TurnOrigin::Desktop,
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(error) => {
                waiting.queue.lock().unwrap().put_back(next);
                Err(error)
            }
        }
    }

    /// Has the worker send the conversation's next waiting message.
    pub(crate) fn drain_later(&self, session: &str) {
        if !self
            .0
            .shutting_down
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            let _ = self
                .0
                .to_correct
                .send(crate::AfterTurn::Drain(session.to_owned()));
        }
    }

    /// Sends the conversation's next waiting message, unless a turn runs,
    /// the last one failed, or it waits for the user's answer.
    pub(crate) async fn drain(&self, session: &str) {
        let waiting = self.session(session);
        {
            let live = waiting.live.lock().unwrap();
            if live.running || live.error.is_some() {
                return;
            }
        }
        let turn = self
            .0
            .store
            .load_recent_turn_preview_messages(session, 1)
            .await
            .unwrap_or_default();
        if asks(&turn) {
            return;
        }
        let Some((project, next)) = waiting.queue.lock().unwrap().next() else {
            return;
        };
        let root = match self.project_root(&project).await {
            Ok(root) => root,
            Err(error) => {
                tracing::warn!(%error, "A queued message's project is gone");
                waiting.queue.lock().unwrap().put_back(next);
                return;
            }
        };
        let request = next.request(session);
        if let Err(error) = self
            .start_turn(
                &project,
                &root,
                request,
                crate::channels::TurnOrigin::Desktop,
            )
            .await
        {
            // Refused (a turn began meanwhile, or the conversation became
            // read-only): it keeps its place, first in line.
            tracing::warn!(%error, "Could not send a queued message");
            waiting.queue.lock().unwrap().put_back(next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_wait_in_order_and_can_be_moved_or_taken_back() {
        let mut waiting = Waiting::default();
        assert!(waiting.push("p", "  ", vec![], vec![]).is_err());
        let first = waiting.push("p", " one ", vec![], vec![]).unwrap();
        let second = waiting
            .push("p", "", vec!["data/a.csv".into()], vec![])
            .unwrap();
        let third = waiting.push("p", "three", vec![], vec![]).unwrap();
        assert_eq!(waiting.list()[0].message, "one");
        assert!(!waiting.move_up(first));
        assert!(waiting.move_up(third));
        let order: Vec<u64> = waiting.list().iter().map(|queued| queued.id).collect();
        assert_eq!(order, [first, third, second]);
        assert_eq!(waiting.take(third).unwrap().message, "three");
        assert!(waiting.take(third).is_none());
        let (project, next) = waiting.next().unwrap();
        assert_eq!((project.as_str(), next.id), ("p", first));
        waiting.put_back(next);
        assert_eq!(waiting.list()[0].id, first);
        for n in 0..LIMIT - 2 {
            waiting.push("p", &format!("m{n}"), vec![], vec![]).unwrap();
        }
        assert!(waiting.push("p", "over", vec![], vec![]).is_err());
    }

    #[test]
    fn a_turn_that_asked_the_user_holds_the_queue() {
        let asked = wisp_llm::Message::tool("c1", wisp_tools::ask_user::ASK_USER, "{}");
        let planned = wisp_llm::Message::tool("c2", wisp_tools::plan::PROPOSE_PLAN, "{}");
        let ran = wisp_llm::Message::tool("c3", "shell", "ok");
        let empty = wisp_llm::Message::assistant("");
        assert!(asks(&[ran.clone(), asked.clone()]));
        assert!(asks(&[planned]));
        // An empty answer after the question still leaves it waiting.
        assert!(asks(&[asked.clone(), empty.clone()]));
        assert!(!asks(&[asked, wisp_llm::Message::assistant("Done.")]));
        assert!(!asks(&[ran, empty]));
        assert!(!asks(&[]));
    }
}
