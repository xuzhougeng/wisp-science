//! Runs the user moved to the background from their card in the transcript,
//! as the WebView desktop does (`src-tauri/src/run_background.rs`); the
//! request, the wait for the end and the notice are shared through
//! `wisp_runs::background`.
//!
//! Asking records the request on the Run. The `monitor_run` and
//! `run_in_context` waits read it (`wisp_runs`), so a turn waiting on that
//! Run gets `wait_backgrounded`, is told not to monitor it again and
//! finishes its reply; the Run itself is not touched. A watcher then posts
//! one completion message to the conversation that owns the Run once it
//! ends, after any turn running there. The agent is not resumed by it. The
//! request is stored with the Run, so one that ends while the app is closed
//! is reported after the next start.

use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;

use crate::Server;

/// Status reads only; the Run manager's own pollers advance the Run.
const POLL: Duration = if cfg!(test) {
    Duration::from_millis(10)
} else {
    Duration::from_secs(2)
};

/// How long a notice waits for a turn running in its conversation.
const IDLE_POLL: Duration = Duration::from_millis(200);

impl Server {
    /// Moves a Run of `project` that has not ended to the background: a
    /// turn waiting on it stops waiting and the conversation takes new
    /// messages, while the Run keeps going; its conversation gets a
    /// message, in Chinese when `chinese`, when it ends. Asking again for
    /// a Run already moved is not an error. The desktop's own command.
    pub async fn background_run(&self, project: &str, run: &str, chinese: bool) -> Result<()> {
        let record = self.own_run(project, run).await?;
        wisp_runs::background::request(&self.0.store, &record)
            .await
            .map_err(anyhow::Error::msg)?;
        self.watch_background_run(run.to_owned(), chinese);
        Ok(())
    }

    /// Resumes watching the Runs moved to the background before the app
    /// last closed, reporting those that ended meanwhile. The desktop
    /// starts this once.
    pub fn start_background_run_notices(&self) {
        let server = self.clone();
        tokio::spawn(async move {
            let pending = match server.0.store.list_pending_background_runs().await {
                Ok(pending) => pending,
                Err(error) => {
                    tracing::warn!(%error, "Could not list background Runs");
                    return;
                }
            };
            let chinese = server.notices_in_chinese().await;
            for run in pending {
                server.watch_background_run(run, chinese);
            }
        });
    }

    /// The language a notice from a watcher started at launch is in: the
    /// desktop's saved choice, else the WebView's, else English.
    async fn notices_in_chinese(&self) -> bool {
        let store = &self.0.store;
        let chosen = store
            .get_setting("desktop.ui.language")
            .await
            .ok()
            .flatten();
        let chosen = match chosen.as_deref().map(str::trim) {
            Some(choice @ ("zh" | "en")) => Some(choice.to_owned()),
            _ => store.get_setting("locale").await.ok().flatten(),
        };
        chosen.is_some_and(|locale| locale.trim().starts_with("zh"))
    }

    /// One watcher per Run in this process, so a repeated request or the
    /// launch scan never posts twice; the store's claim covers the rest.
    fn watch_background_run(&self, run: String, chinese: bool) {
        if !self.0.background_runs.lock().unwrap().insert(run.clone()) {
            return;
        }
        let server = self.clone();
        tokio::spawn(async move {
            if let Err(error) = server.report_background_run(&run, chinese).await {
                tracing::warn!(%error, run, "Could not report a background Run");
            }
            server.0.background_runs.lock().unwrap().remove(&run);
        });
    }

    async fn report_background_run(&self, run: &str, chinese: bool) -> Result<()> {
        let store = &self.0.store;
        // Until it ends, without claiming it: the app may close first.
        let frame = loop {
            if self.0.shutting_down.load(Ordering::Relaxed) {
                return Ok(());
            }
            match store.get_run(run).await {
                Ok(Some(record)) if record.status.is_terminal() => match record.frame_id {
                    Some(frame) => break frame,
                    None => return Ok(()),
                },
                Ok(Some(_)) => {}
                Ok(None) => return Ok(()),
                Err(error) => tracing::debug!(%error, run, "Background Run status read failed"),
            }
            tokio::time::sleep(POLL).await;
        };
        if store.frame_project_id(&frame).await?.is_none() {
            return Ok(());
        }
        // Added once whatever the conversation is doing has ended, as a
        // finished background batch's result is.
        let session = self.session(&frame);
        loop {
            if self.0.shutting_down.load(Ordering::Relaxed) {
                return Ok(());
            }
            {
                let _accept = session.accept.lock().await;
                if !session.live.lock().unwrap().running {
                    let Some(ended) =
                        wisp_runs::background::claim_when_ended(store, run, Duration::ZERO)
                            .await
                            .map_err(anyhow::Error::msg)?
                    else {
                        return Ok(());
                    };
                    let notice = wisp_runs::background::completion_notice(chinese, &ended);
                    let seq = store.max_message_seq(&frame).await? + 1;
                    store
                        .append_message(&frame, seq, &wisp_llm::Message::assistant(notice))
                        .await?;
                    let mut live = session.live.lock().unwrap();
                    live.generation = live.generation.wrapping_add(1);
                    return Ok(());
                }
            }
            tokio::time::sleep(IDLE_POLL).await;
        }
    }
}
