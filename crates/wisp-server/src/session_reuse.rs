//! Reusing a range of a conversation's turns, as the WebView's Quote /
//! reuse does: the turns are rendered as a quote the user puts in front of
//! a message in another conversation, or the conversation's model distills
//! them into an experience draft the user reads, changes and may keep as a
//! memory. The source conversation is only read; nothing is saved here.
//!
//! The excerpt is `wisp_core::ContextManager::render_excerpt`, and the
//! experience prompt and evidence are `wisp_core::session_experience`,
//! both shared with the WebView's commands.
use std::time::Duration;

use anyhow::{bail, ensure, Result};
use wisp_dto::{SessionExcerpt, SessionExperience};
use wisp_llm::{Message, Role};

use crate::{desktop::TurnRef, Server};

/// As long as the WebView waits for the experience draft.
const WAIT: Duration = Duration::from_secs(180);

/// The longest experience draft asked of the model, in tokens, as the
/// WebView caps it.
const EXPERIENCE_TOKENS: u64 = 4096;

/// The chosen turns: the 0-based numbers of the first and the last among
/// the conversation's user messages, and every saved message from the
/// first one up to the user message after the last.
struct Range {
    first: usize,
    last: usize,
    messages: Vec<Message>,
}

impl Server {
    /// Renders the turns from `first` to `last` (in either order) of
    /// `session` as a quote for another conversation. Refused for a
    /// conversation an ACP agent runs, for a turn that is still running and
    /// for a turn the saved conversation does not have.
    pub async fn render_session_excerpt(
        &self,
        project: &str,
        session: &str,
        first: &TurnRef,
        last: &TurnRef,
    ) -> Result<SessionExcerpt> {
        let range = self.reused_range(project, session, first, last).await?;
        let excerpt = wisp_core::ContextManager::render_excerpt(
            &range.messages,
            wisp_core::context::EXCERPT_MAX_BYTES,
        )
        .map_err(anyhow::Error::msg)?;
        let title = self
            .0
            .store
            .session_title(session)
            .await?
            .unwrap_or_default();
        Ok(SessionExcerpt {
            session_id: session.to_owned(),
            title,
            start_user_index: range.first,
            end_user_index: range.last,
            text: excerpt.text,
            truncated: excerpt.truncated,
        })
    }

    /// Has the conversation's model distill the turns from `first` to
    /// `last` into an editable experience draft, steered by the user's
    /// optional `focus`. Nothing is saved: the draft is the user's to
    /// change, quote or keep as a memory.
    pub async fn draft_session_experience(
        &self,
        project: &str,
        session: &str,
        first: &TurnRef,
        last: &TurnRef,
        focus: Option<&str>,
    ) -> Result<SessionExperience> {
        let range = self.reused_range(project, session, first, last).await?;
        let (evidence, truncated) =
            wisp_core::session_experience::evidence(&range.messages).map_err(anyhow::Error::msg)?;
        let prompt = wisp_core::session_experience::request(
            focus,
            range.first + 1,
            range.last + 1,
            &evidence,
            truncated,
        )
        .map_err(anyhow::Error::msg)?;
        // The conversation's own model, as the WebView asks it, kept short.
        let store = &self.0.store;
        let options = self.stored_options(session).await?;
        let mut model = self.0.model.read().unwrap().pick(&options.model)?.provider;
        let routes = crate::network::load(store).await;
        crate::network::route_model(&mut model.proxy, &routes);
        model.max_tokens = match model.max_tokens {
            0 => EXPERIENCE_TOKENS,
            cap => cap.min(EXPERIENCE_TOKENS),
        };
        let provider = (self.0.models)(model);
        let messages = [
            Message::system(wisp_core::session_experience::SYSTEM_PROMPT),
            Message::user(prompt),
        ];
        let reply = match tokio::time::timeout(WAIT, provider.complete(&messages, &[])).await {
            Err(_) => bail!("Experience generation timed out. Please try again."),
            Ok(Err(error)) => bail!(
                "{}",
                crate::network::explain(store, &format!("Experience generation failed: {error}"))
                    .await
            ),
            Ok(Ok(reply)) => reply,
        };
        let content = reply.content.trim();
        ensure!(
            !content.is_empty(),
            "The model returned an empty experience draft."
        );
        Ok(SessionExperience {
            content: content.to_owned(),
            truncated,
        })
    }

    /// Finds the turns from `first` to `last` the way the transcript names
    /// them, refusing what the WebView refuses.
    async fn reused_range(
        &self,
        project: &str,
        session: &str,
        first: &TurnRef,
        last: &TurnRef,
    ) -> Result<Range> {
        let session = session.trim();
        ensure!(!session.is_empty(), "No session to quote from.");
        self.own_conversation(project, session).await?;
        ensure!(
            crate::acp_turn::agent_of(&self.0.store, session)
                .await?
                .is_none(),
            "ACP conversations cannot be used as a quote source."
        );
        // The earlier turn has more user messages after it.
        let (first, last) = if first.from_end >= last.from_end {
            (first, last)
        } else {
            (last, first)
        };
        // Only finished turns are quoted: the running one is the latest.
        ensure!(
            !(last.from_end == 0 && self.session(session).live.lock().unwrap().running),
            "Wait for the selected turns to finish."
        );
        let (start, _) = self.locate_turn(session, first).await?;
        let (end, next) = self.locate_turn(session, last).await?;
        let transcript = self.numbered_transcript(session).await?;
        let users_before = |seq: i64| {
            transcript
                .iter()
                .filter(|(at, message)| *at < seq && message.role == Role::User)
                .count()
        };
        let messages: Vec<Message> = transcript
            .iter()
            .filter(|(seq, _)| *seq >= start && next.is_none_or(|next| *seq < next))
            .map(|(_, message)| message.clone())
            .collect();
        ensure!(
            !messages.is_empty(),
            "The selected turns are not available."
        );
        Ok(Range {
            first: users_before(start),
            last: users_before(end),
            messages,
        })
    }
}
