//! Compacting a conversation on request: its whole history is archived
//! under the project's `.wisp/history/`, and what the model is sent from
//! then on is shorter — noise pruned, or older turns folded into a
//! summary with the recent ones kept. A turn does the same by itself
//! when the context fills up; this is the same step asked for.

use std::sync::atomic::Ordering;

use anyhow::{anyhow, ensure, Context, Result};
use wisp_store::OpenContextEpoch;

use crate::{runtime, Server};

/// What a compaction did, in estimated tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Compacted {
    pub before: usize,
    pub after: usize,
    /// Older turns were folded into a summary; otherwise only noise went.
    pub summarised: bool,
}

impl Server {
    /// Compacts the conversation now. Refused while a turn runs in it and
    /// once its research is archived. The transcript on screen is not
    /// shortened; only what the model is sent is.
    pub async fn compact_conversation(&self, project: &str, session: &str) -> Result<Compacted> {
        self.own_conversation(project, session).await?;
        let (_, root) = self
            .0
            .store
            .get_project(project)
            .await?
            .context("Project not found")?;
        let live = self.session(session);
        // Held to the end: no turn is accepted while the history is folded.
        let _accept = live.accept.lock().await;
        let _admission = self.0.admission.read().await;
        ensure!(
            !self.0.shutting_down.load(Ordering::Relaxed),
            "Server is shutting down"
        );
        ensure!(
            !live.live.lock().unwrap().running,
            "A turn is running in this conversation; compact it when the turn has ended"
        );
        ensure!(
            !self.archived(session).await?,
            "This conversation's research is archived and it is read-only"
        );
        let options = self.stored_options(session).await?;
        let model = self.0.model.read().unwrap().pick(&options.model)?;
        let subagent = self.subagent_model_for(session, &model).await;
        let mut agent = runtime::build(
            &self.0.config,
            &model,
            &self.0.store,
            self.0.runs.clone(),
            project,
            &root,
            session,
            &live,
            &self.0.models,
            (!options.reasoning_effort.is_empty()).then_some(options.reasoning_effort),
            &[],
            options.plan_first,
            &[],
            vec![],
            &subagent,
            None,
            None,
            runtime::LabTurn::default(),
        )
        .await?;
        ensure!(
            agent
                .ctx
                .messages
                .iter()
                .any(|message| message.role == wisp_llm::Role::User),
            "There is nothing to compact yet"
        );
        let (before, after, _) = agent.compact().await.map_err(|error| anyhow!(error))?;
        let done = agent
            .ctx
            .last_compaction()
            .context("The compaction left nothing to keep")?;
        let summarised = matches!(done.kind, wisp_core::CompactionKind::Semantic);
        self.0
            .store
            .open_context_epoch(
                session,
                OpenContextEpoch {
                    messages: &agent.ctx.messages,
                    strategy: "manual",
                    kind: if summarised { "semantic" } else { "prune_only" },
                    before_tokens: done.before,
                    after_tokens: done.after,
                    checkpoint_index: done.checkpoint_index,
                    first_kept_seq: None,
                    archive_ref: Some(&done.archive_reference),
                    ui_event_seq: None,
                },
            )
            .await?;
        Ok(Compacted {
            before,
            after,
            summarised,
        })
    }
}
