//! Handing a conversation off to a new one, as the WebView's handoff does:
//! the conversation's model writes a note from what it is sent now (the
//! summary standing in for compacted turns and the turns after it), the
//! user reads and changes the note, and a new conversation in the same
//! project starts with the note as its first message. The prompt, what it
//! includes and how it is bounded are the WebView's, shared from
//! `wisp_core::ContextManager::handoff_request`. The conversation handed
//! off is only read, and the new one keeps no link to it.
//!
//! The WebView's way on from a turn refused as too long, a new
//! conversation that reads this one, is started here too.
use std::time::Duration;

use anyhow::{bail, ensure, Result};
use wisp_dto::{
    native_conversations::{HandoffRequest, HandoffResponse, SendRequest},
    ComposerReferenceArg,
};

use crate::Server;

/// The longest instruction for the note, as the WebView allows.
const INSTRUCTION_LIMIT: usize = 8_000;

/// As long as the WebView waits for the note.
const WAIT: Duration = Duration::from_secs(180);

impl Server {
    /// Has the conversation's model write a handoff note from what the
    /// model is sent now, steered by the user's optional instruction.
    /// Nothing is saved. Refused while a turn runs, in a conversation an
    /// ACP agent runs and in one whose research is archived.
    pub async fn draft_handoff(
        &self,
        project: &str,
        request: HandoffRequest,
    ) -> Result<HandoffResponse> {
        let session = request.session_id.trim();
        ensure!(!session.is_empty(), "No conversation to hand off");
        ensure!(
            request
                .instruction
                .as_ref()
                .is_none_or(|text| text.chars().count() <= INSTRUCTION_LIMIT),
            "Handoff instructions must be at most 8000 characters"
        );
        self.own_conversation(project, session).await?;
        ensure!(
            !self.archived(session).await?,
            "This conversation's research is archived and it is read-only"
        );
        ensure!(
            crate::acp_turn::agent_of(&self.0.store, session)
                .await?
                .is_none(),
            "A conversation an ACP agent runs cannot write a handoff note"
        );
        ensure!(
            !self.session(session).live.lock().unwrap().running,
            "Wait for the current turn to finish before writing a handoff note"
        );
        let store = &self.0.store;
        let messages = store.load_messages(session).await?;
        let prompt =
            wisp_core::ContextManager::handoff_request(&messages, request.instruction.as_deref())
                .map_err(anyhow::Error::msg)?;
        // The conversation's own model, as the WebView asks it.
        let options = self.stored_options(session).await?;
        let mut model = self.0.model.read().unwrap().pick(&options.model)?.provider;
        let routes = crate::network::load(store).await;
        crate::network::route_model(&mut model.proxy, &routes);
        let provider = (self.0.models)(model);
        let reply = match tokio::time::timeout(WAIT, provider.complete(&prompt, &[])).await {
            Err(_) => bail!("The model did not finish the handoff note within three minutes"),
            Ok(Err(error)) => bail!(
                "{}",
                crate::network::explain(store, &format!("The handoff note failed: {error}")).await
            ),
            Ok(Ok(reply)) => reply,
        };
        let note = reply.content.trim();
        ensure!(!note.is_empty(), "The model returned an empty handoff note");
        Ok(HandoffResponse {
            project_id: project.to_owned(),
            session_id: session.to_owned(),
            note: note.to_owned(),
        })
    }

    /// Starts a new conversation in the project of `source` with `note` as
    /// its first message, the way the WebView's handoff opens one, and
    /// returns it. `source` is not touched.
    pub async fn start_from_handoff(
        &self,
        project: &str,
        source: &str,
        note: &str,
    ) -> Result<String> {
        let note = note.trim();
        ensure!(!note.is_empty(), "The handoff note is empty");
        self.own_conversation(project, source).await?;
        self.open_with_first_message(project, note, Vec::new())
            .await
    }

    /// The WebView's other way on from a turn the model refused as too
    /// long: a new conversation in the same project is sent `prompt` with
    /// `source` referenced, so the model reads that conversation as
    /// background (as a referenced conversation is read here) and goes on
    /// with its unfinished request. Returns the new conversation; `source`
    /// is not touched.
    pub async fn continue_in_new_conversation(
        &self,
        project: &str,
        source: &str,
        prompt: &str,
    ) -> Result<String> {
        let prompt = prompt.trim();
        ensure!(
            !prompt.is_empty(),
            "Say what the new conversation should do"
        );
        self.own_conversation(project, source).await?;
        self.open_with_first_message(
            project,
            prompt,
            vec![ComposerReferenceArg::Session {
                id: source.to_owned(),
            }],
        )
        .await
    }

    /// A new conversation in `project`, started the way Settings says new
    /// ones start, with `message` sent as its first. One whose first
    /// message could not be sent is removed again.
    async fn open_with_first_message(
        &self,
        project: &str,
        message: &str,
        references: Vec<ComposerReferenceArg>,
    ) -> Result<String> {
        let id = self.create_conversation(project).await?;
        let sent = self
            .send(
                project,
                SendRequest {
                    session_id: id.clone(),
                    request_id: uuid::Uuid::new_v4().to_string(),
                    message: message.to_owned(),
                    resume: false,
                    resume_after: None,
                    attachments: Vec::new(),
                    references,
                    acp_agent_id: None,
                },
            )
            .await;
        if let Err(error) = sent {
            if let Err(removing) = self.delete_conversation(project, &id).await {
                tracing::warn!(%removing, "Could not remove a new conversation left empty");
            }
            return Err(error);
        }
        Ok(id)
    }
}
