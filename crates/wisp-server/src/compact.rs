//! Compacting a conversation on request: its whole history is archived
//! under the project's `.wisp/history/`, and what the model is sent from
//! then on is shorter — noise pruned, or older turns folded into a
//! summary with the recent ones kept. A turn does the same by itself
//! when the context fills up; this is the same step asked for.
//!
//! Every compaction is recorded as a context epoch; the transcript shows
//! each one as a `compaction` row after the last message saved before it,
//! with the fields the WebView's compaction card reads.

use std::sync::atomic::Ordering;

use anyhow::{anyhow, ensure, Context, Result};
use wisp_dto::{native_conversations::Item, ContextCompactionDto};
use wisp_store::{ContextEpochRecord, OpenContextEpoch};

use crate::{runtime, Server};

/// Which compaction is asked for, as the WebView's compact dialog offers
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactKind {
    /// Old tool results and pictures become short archived stubs; every
    /// question and answer stays as it was.
    Regular,
    /// Older turns are folded into a summary, with a short recent tail
    /// kept as it was.
    Semantic,
}

impl CompactKind {
    fn intent(self) -> wisp_core::CompactIntent {
        match self {
            Self::Regular => wisp_core::CompactIntent::PruneOnly,
            Self::Semantic => wisp_core::CompactIntent::Semantic,
        }
    }
}

/// What a compaction did, in estimated tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Compacted {
    pub before: usize,
    pub after: usize,
    /// Older turns were folded into a summary; otherwise only noise went.
    pub summarised: bool,
}

impl Server {
    /// Compacts the conversation now, the way `kind` says. Refused while a
    /// turn runs in it and once its research is archived. The transcript on
    /// screen is not shortened; only what the model is sent is, and a
    /// compaction row marks where that happened.
    pub async fn compact_conversation(
        &self,
        project: &str,
        session: &str,
        kind: CompactKind,
    ) -> Result<Compacted> {
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
        // The WebView compacts by sending `/compact`, which a frozen
        // branch refuses.
        self.ensure_not_frozen_branch(session).await?;
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
        let (before, after, _) = agent
            .compact_with_intent(None, kind.intent())
            .await
            .map_err(|error| anyhow!(error))?;
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
        // The context meter says how full the context is now.
        {
            let mut state = live.live.lock().unwrap();
            if let Some(usage) = state.usage.as_mut() {
                usage.context_tokens = after;
            }
            // The latest request's split no longer says what is sent.
            state.context = None;
        }
        Ok(Compacted {
            before,
            after,
            summarised,
        })
    }

    /// Puts back what the latest compaction replaced, as the WebView's undo
    /// does: only the compaction that was looked at (`expected`, the
    /// working set's epoch then), and only while nothing has been said
    /// after it. What the turns after it would have read is the model's
    /// again from the next turn.
    pub async fn undo_compaction(&self, project: &str, session: &str, expected: i64) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        ensure!(
            !self.archived(session).await?,
            "A frozen conversation cannot be changed"
        );
        ensure!(
            !self.0.store.session_branch_frozen(session).await?,
            "Frozen conversation branches cannot undo compaction."
        );
        ensure!(
            crate::acp_turn::agent_of(&self.0.store, session)
                .await?
                .is_none(),
            "A conversation an ACP agent runs cannot undo a compaction"
        );
        let live = self.session(session);
        // Acceptance takes the same lock, so no turn starts meanwhile.
        let _accept = live.accept.lock().await;
        ensure!(
            !live.live.lock().unwrap().running,
            "Stop the running turn before undoing the compaction"
        );
        let current = self.0.store.frame_head_epoch(session).await?;
        ensure!(
            expected > 0 && current == expected,
            "What the model is sent has changed; look at it again before undoing the compaction"
        );
        let undone = self.0.store.context_epoch(session, expected).await?;
        match self.0.store.undo_context_epoch(session).await {
            Ok(_) => {
                // Nothing was said since, so the context is as full as it
                // was when the compaction began.
                let mut state = live.live.lock().unwrap();
                if let (Some(usage), Some(record)) = (state.usage.as_mut(), undone) {
                    usage.context_tokens = usize::try_from(record.before_tokens).unwrap_or(0);
                }
                state.context = None;
                Ok(())
            }
            Err(error) if error.to_string().contains("context_epoch_has_new_turns") => {
                Err(anyhow!(
                    "The conversation went on after the compaction, so it can no longer be undone"
                ))
            }
            Err(error) if error.to_string().contains("context_epoch_none") => {
                Err(anyhow!("There is no compaction to undo"))
            }
            Err(error) => Err(error),
        }
    }
}

impl Server {
    /// The compaction rows a page of the transcript shows, each with the
    /// seq of the last message saved before it (where it goes). `seqs` are
    /// the page's message seqs, oldest first; `before` is the seq the page
    /// was asked to end before (`None` for the newest page); `saved_through`
    /// leaves out compactions recorded after it (a running turn shows the
    /// one it makes once it has ended); `latest` is the conversation's
    /// highest message seq. A record that only repaired a saved working set
    /// is not a compaction and is not shown.
    pub(crate) async fn compaction_rows(
        &self,
        session: &str,
        seqs: &[i64],
        before: Option<i64>,
        saved_through: Option<i64>,
        latest: i64,
        frozen: bool,
    ) -> Result<Vec<(i64, Item)>> {
        let records = self.0.store.context_epochs(session).await?;
        let shown: Vec<&ContextEpochRecord> = records
            .iter()
            .filter(|record| shown(record))
            .filter(|record| on_page(seqs, before, record.initial_head_seq))
            .filter(|record| saved_through.is_none_or(|limit| record.initial_head_seq <= limit))
            .collect();
        if shown.is_empty() {
            return Ok(Vec::new());
        }
        let head = self.0.store.frame_head_epoch(session).await?;
        let acp = crate::acp_turn::agent_of(&self.0.store, session)
            .await?
            .is_some();
        let mut rows = Vec::with_capacity(shown.len());
        for record in shown {
            let checkpoint = match record.checkpoint_seq {
                Some(seq) => self
                    .0
                    .store
                    .load_message_at_seq(session, seq)
                    .await?
                    .map(|message| message.content.as_text()),
                None => None,
            };
            let has_new_turns = record.epoch == head && latest > record.initial_head_seq;
            let (can_undo, reason) = undo_state(record.epoch, head, has_new_turns, frozen, acp);
            let card = ContextCompactionDto {
                epoch: u64::try_from(record.epoch).unwrap_or(0),
                before: usize::try_from(record.before_tokens).unwrap_or(0),
                after: usize::try_from(record.after_tokens).unwrap_or(0),
                strategy: record.strategy.clone(),
                checkpoint,
                // The WebView counts its visual turns for this; this host
                // keeps no such events.
                kept_from_user_index: None,
                can_undo,
                undo_reason: reason.map(str::to_owned),
            };
            let mut row = runtime::item("compaction", &serde_json::to_string(&card)?);
            row.seq = Some(record.initial_head_seq);
            rows.push((record.initial_head_seq, row));
        }
        Ok(rows)
    }
}

/// Whether a context epoch is a compaction the transcript shows: one asked
/// for or made by a turn, not a repair of a saved working set.
fn shown(record: &ContextEpochRecord) -> bool {
    record.strategy != "recovery"
}

/// Whether the page holds the last message saved before a compaction
/// (`through`, the highest seq the compaction read), so the compaction's
/// row goes on this page and on no other. Its messages run oldest first
/// and all come before `before`; the pages after it start at `before`.
fn on_page(seqs: &[i64], before: Option<i64>, through: i64) -> bool {
    seqs.first().is_some_and(|first| *first <= through)
        && before.is_none_or(|before| before > through)
}

/// Whether the compaction `epoch` can be undone now, by the rules
/// `undo_compaction` checks, and why not with the WebView's reasons where
/// it has one: `not_head` (a later compaction stands), `has_new_turns`
/// (the conversation went on), `read_only` (its research is archived) and
/// `acp` (an ACP agent runs it). A running turn is the client's to check;
/// it ends.
fn undo_state(
    epoch: i64,
    head: i64,
    has_new_turns: bool,
    frozen: bool,
    acp: bool,
) -> (bool, Option<&'static str>) {
    if epoch != head {
        (false, Some("not_head"))
    } else if has_new_turns {
        (false, Some("has_new_turns"))
    } else if frozen {
        (false, Some("read_only"))
    } else if acp {
        (false, Some("acp"))
    } else {
        (true, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compaction_row_goes_on_the_page_with_the_last_message_before_it() {
        // The newest page, 20..=30: a compaction after 25 is on it, one
        // after its last message too, one before its first message is not.
        let newest = [20, 22, 25, 30];
        assert!(on_page(&newest, None, 25));
        assert!(on_page(&newest, None, 40));
        assert!(!on_page(&newest, None, 19));
        // An older page, asked to end before 20: a compaction after its
        // last message (or later, before 20) is on it, as long as nothing
        // the newer page holds came before the compaction.
        let older = [10, 12, 15];
        assert!(on_page(&older, Some(20), 15));
        assert!(on_page(&older, Some(20), 19));
        assert!(!on_page(&older, Some(20), 25));
        assert!(!on_page(&older, Some(20), 9));
        // So each compaction is on exactly one of the two pages.
        for through in [10, 12, 15, 19, 20, 25, 30, 40] {
            let pages = [
                on_page(&older, Some(20), through),
                on_page(&newest, None, through),
            ];
            assert_eq!(pages.iter().filter(|on| **on).count(), 1, "{through}");
        }
        // An empty page holds none.
        assert!(!on_page(&[], None, 5));
    }

    #[test]
    fn only_the_latest_compaction_with_nothing_said_after_it_can_be_undone() {
        assert_eq!(undo_state(2, 2, false, false, false), (true, None));
        assert_eq!(
            undo_state(1, 2, false, false, false),
            (false, Some("not_head"))
        );
        assert_eq!(
            undo_state(2, 2, true, false, false),
            (false, Some("has_new_turns"))
        );
        assert_eq!(
            undo_state(2, 2, false, true, false),
            (false, Some("read_only"))
        );
        assert_eq!(undo_state(2, 2, false, false, true), (false, Some("acp")));
    }

    #[test]
    fn a_repaired_working_set_is_not_shown_as_a_compaction() {
        let record = |strategy: &str| ContextEpochRecord {
            frame_id: "f".into(),
            epoch: 1,
            parent_epoch: 0,
            strategy: strategy.into(),
            kind: "prune_only".into(),
            before_tokens: 0,
            after_tokens: 0,
            first_seq: 5,
            initial_head_seq: 7,
            checkpoint_seq: None,
            first_kept_seq: None,
            archive_ref: None,
            ui_event_seq: None,
            created_at: 0,
        };
        assert!(shown(&record("manual")));
        assert!(shown(&record("auto")));
        assert!(shown(&record("overflow")));
        assert!(!shown(&record("recovery")));
    }
}
