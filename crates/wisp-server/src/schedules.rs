//! Scheduled tasks: a prompt sent into a new conversation of a project on
//! a fixed cadence, while the app is open.
//!
//! Nothing fires while the app is closed; a time that was missed fires
//! once the next time it is looked at, and the cadence keeps its place on
//! the clock. The rules follow `src-tauri/src/scheduler.rs`, which cannot
//! be shared from here because it lives in the Tauri shell, and the rows
//! are the store's own, so both desktops see the same tasks.
//!
//! A conversation's own timer (a row that replaces its previous turn) is
//! the WebView's to fire: it is listed nowhere here and never claimed.

use anyhow::{ensure, Context, Result};
use serde_json::json;
use wisp_dto::remote_access::RemoteRequest;
use wisp_store::{next_slot_after, ScheduleRecord, ScheduleRunRecord};

use crate::Server;

/// Shorter cadences would turn catching up into a busy loop.
pub const MIN_INTERVAL_SECS: i64 = 60;
const MAX_INTERVAL_SECS: i64 = 366 * 24 * 3600;
const NAME_LIMIT: usize = 80;
const PROMPT_LIMIT: usize = 20_000;
/// How often the app looks for due tasks.
pub const POLL_SECS: u64 = 30;

/// The form for a new task.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScheduleInput {
    pub project: String,
    /// Empty takes the prompt's first line.
    pub name: String,
    pub prompt: String,
    pub interval_secs: i64,
    /// When it first fires, in Unix seconds; `None` is one interval from
    /// now, and a time already past fires at the next look.
    pub start_at: Option<i64>,
    /// The conversation it is sent into each time: a conversation's
    /// timer. `None` starts a new conversation per firing.
    pub session: Option<String>,
}

/// How a task's latest firing went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleRun {
    pub fired_at: i64,
    /// The conversation it was sent into, when one was made.
    pub session: Option<String>,
    /// Why it could not be sent.
    pub error: Option<String>,
}

/// A task as the page lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleRow {
    pub id: String,
    pub project: String,
    pub project_name: String,
    pub name: String,
    pub prompt: String,
    pub interval_secs: i64,
    pub enabled: bool,
    pub next_run_at: i64,
    /// The conversation it is sent into, when it is a conversation's timer.
    pub session: Option<String>,
    /// Newest first, at most five.
    pub runs: Vec<ScheduleRun>,
}

/// Checks the form and makes the row it describes.
fn record(input: &ScheduleInput, now: i64) -> Result<ScheduleRecord> {
    let prompt = input.prompt.trim();
    ensure!(!prompt.is_empty(), "A task needs a prompt");
    ensure!(
        prompt.chars().count() <= PROMPT_LIMIT,
        "A prompt is at most {PROMPT_LIMIT} characters"
    );
    ensure!(
        (MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&input.interval_secs),
        "A task repeats at least every minute and at most once a year"
    );
    let name = match input.name.trim() {
        "" => prompt.lines().next().unwrap_or_default(),
        typed => typed,
    };
    Ok(ScheduleRecord {
        id: uuid::Uuid::new_v4().to_string(),
        project_id: input.project.clone(),
        frame_id: input.session.clone(),
        replace_previous_turn: false,
        name: name.chars().take(NAME_LIMIT).collect(),
        prompt: prompt.to_owned(),
        skill: None,
        interval_secs: input.interval_secs,
        enabled: true,
        next_run_at: input.start_at.unwrap_or(now + input.interval_secs),
        last_run_at: None,
        created_at: now,
        updated_at: now,
    })
}

/// What a fired task sends: where it came from, for the reader and the
/// model alike, then the prompt.
fn message(schedule: &ScheduleRecord) -> String {
    format!("[Scheduled task: {}]\n\n{}", schedule.name, schedule.prompt)
}

impl Server {
    /// Every project's scheduled tasks, the one due soonest first.
    pub async fn schedules(&self) -> Result<Vec<ScheduleRow>> {
        let store = &self.0.store;
        let mut rows = vec![];
        for (project, project_name, ..) in store.list_projects().await? {
            for schedule in store.list_schedules(&project).await? {
                if schedule.replace_previous_turn {
                    continue;
                }
                let runs = store
                    .list_schedule_runs(&schedule.id, 5)
                    .await?
                    .into_iter()
                    .map(|run| ScheduleRun {
                        fired_at: run.fired_at,
                        session: run.frame_id,
                        error: run.error,
                    })
                    .collect();
                rows.push(ScheduleRow {
                    id: schedule.id,
                    project: project.clone(),
                    project_name: project_name.clone(),
                    name: schedule.name,
                    prompt: schedule.prompt,
                    interval_secs: schedule.interval_secs,
                    enabled: schedule.enabled,
                    next_run_at: schedule.next_run_at,
                    session: schedule.frame_id,
                    runs,
                });
            }
        }
        rows.sort_by(|a, b| {
            (!a.enabled, a.next_run_at, &a.id).cmp(&(!b.enabled, b.next_run_at, &b.id))
        });
        Ok(rows)
    }

    pub async fn create_schedule(&self, input: ScheduleInput) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.0
            .store
            .get_project(&input.project)
            .await?
            .context("Choose a project for the task")?;
        if let Some(session) = &input.session {
            self.own_conversation(&input.project, session).await?;
        }
        let schedule = record(&input, chrono::Utc::now().timestamp())?;
        self.0.store.create_schedule(&schedule).await
    }

    /// Removes the timers of one conversation. Returns how many there were.
    pub async fn cancel_conversation_schedules(
        &self,
        project: &str,
        session: &str,
    ) -> Result<usize> {
        let _admission = self.0.admission.read().await;
        let mut removed = 0;
        for schedule in self.0.store.list_schedules(project).await? {
            if !schedule.replace_previous_turn && schedule.frame_id.as_deref() == Some(session) {
                self.0.store.delete_schedule(&schedule.id).await?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// A task of this page; a conversation's timer is not one.
    async fn schedule(&self, id: &str) -> Result<ScheduleRecord> {
        self.0
            .store
            .get_schedule(id)
            .await?
            .filter(|schedule| !schedule.replace_previous_turn)
            .context("That task no longer exists")
    }

    /// Turns a task on or off. Turned on again, a time that passed
    /// meanwhile fires once at the next look.
    pub async fn set_schedule_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.schedule(id).await?;
        self.0
            .store
            .set_schedule_enabled(id, enabled, chrono::Utc::now().timestamp())
            .await?;
        Ok(())
    }

    pub async fn delete_schedule(&self, id: &str) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.schedule(id).await?;
        self.0.store.delete_schedule(id).await
    }

    /// Fires a task at once, leaving its cadence where it was. Returns
    /// the conversation it was sent into.
    pub async fn run_schedule_now(&self, id: &str) -> Result<String> {
        let schedule = self.schedule(id).await?;
        self.fire(&schedule, chrono::Utc::now().timestamp())
            .await?
            .context("The task could not be sent")
    }

    /// Fires every task whose time has come by `now`, each at most once
    /// however long it was due. Returns how many were fired.
    pub async fn fire_due_schedules(&self, now: i64) -> Result<usize> {
        let mut fired = 0;
        for schedule in self.0.store.due_schedules(now).await? {
            if schedule.replace_previous_turn {
                continue;
            }
            // Claimed before it is sent, so a slow turn or a second look
            // never fires the same time twice.
            let next = next_slot_after(schedule.next_run_at, schedule.interval_secs, now);
            if !self
                .0
                .store
                .claim_schedule_fire(&schedule.id, schedule.next_run_at, next, now)
                .await?
            {
                continue;
            }
            // A task that cannot be sent is recorded as failed and stays
            // on its cadence.
            let _ = self.fire(&schedule, now).await;
            fired += 1;
        }
        Ok(fired)
    }

    /// Sends the task into a new conversation of its project and records
    /// how it went. `Ok(None)` is a failure that was recorded.
    async fn fire(&self, schedule: &ScheduleRecord, now: i64) -> Result<Option<String>> {
        let sent = self.send_scheduled(schedule).await;
        let (session, error) = match sent {
            Ok(session) => (Some(session), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        self.0
            .store
            .record_schedule_run(&ScheduleRunRecord {
                id: uuid::Uuid::new_v4().to_string(),
                schedule_id: schedule.id.clone(),
                frame_id: session.clone(),
                status: if error.is_none() { "fired" } else { "failed" }.into(),
                error,
                fired_at: now,
            })
            .await?;
        Ok(session)
    }

    async fn send_scheduled(&self, schedule: &ScheduleRecord) -> Result<String> {
        let fresh = schedule.frame_id.is_none();
        let session = match &schedule.frame_id {
            // A conversation's timer goes into that conversation. While a
            // turn runs there the send is refused and recorded as such.
            Some(session) => session.clone(),
            None => self.create_conversation(&schedule.project_id).await?,
        };
        self.execute(RemoteRequest {
            nonce: "schedule".into(),
            seq: 0,
            id: uuid::Uuid::new_v4().to_string(),
            command: "native_conversation_send".into(),
            project_id: Some(schedule.project_id.clone()),
            args: json!({
                "session_id": session,
                "request_id": uuid::Uuid::new_v4().to_string(),
                "message": message(schedule),
            }),
        })
        .await?;
        if fresh {
            // Named after the task, so it can be told from the others. The
            // send names a new conversation after its message, so this
            // comes after it.
            self.0
                .store
                .rename_session(&session, &schedule.project_id, &schedule.name)
                .await?;
        }
        Ok(session)
    }

    /// Looks for due tasks every half minute until the server shuts
    /// down. The desktop starts this once; a headless server does not.
    pub fn start_scheduler(&self) {
        use std::sync::atomic::Ordering;
        if self.0.scheduling.swap(true, Ordering::SeqCst) {
            return;
        }
        let server = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(POLL_SECS));
            loop {
                tick.tick().await;
                if server.0.shutting_down.load(Ordering::Relaxed) {
                    break;
                }
                let _ = server
                    .fire_due_schedules(chrono::Utc::now().timestamp())
                    .await;
                // The day's recaps are drafted once it is time.
                server.daily_recap_tick().await;
                // And the habits are looked over once a week.
                server.memory_tidy_tick().await;
                // Research mandates run their due rounds.
                server.fire_due_mandates().await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(prompt: &str, interval_secs: i64) -> ScheduleInput {
        ScheduleInput {
            project: "p".into(),
            name: String::new(),
            prompt: prompt.into(),
            interval_secs,
            start_at: None,
            session: None,
        }
    }

    #[test]
    fn a_task_is_checked_and_named_after_its_first_line() {
        let made = record(&input("  Summarise the week\nwith runs  ", 3_600), 1_000).unwrap();
        assert_eq!(made.name, "Summarise the week");
        assert_eq!(made.prompt, "Summarise the week\nwith runs");
        assert_eq!(made.next_run_at, 4_600);
        assert!(made.enabled && made.frame_id.is_none() && !made.replace_previous_turn);
        let timer = ScheduleInput {
            session: Some("c1".into()),
            ..input("check the queue", 600)
        };
        assert_eq!(record(&timer, 0).unwrap().frame_id.as_deref(), Some("c1"));
        assert_eq!(
            message(&made),
            "[Scheduled task: Summarise the week]\n\nSummarise the week\nwith runs"
        );

        let named = ScheduleInput {
            name: format!("  {}  ", "n".repeat(200)),
            start_at: Some(10),
            ..input("check the queue", 60)
        };
        let made = record(&named, 1_000).unwrap();
        assert_eq!(made.name.chars().count(), NAME_LIMIT);
        // A start already past is kept: it fires at the next look.
        assert_eq!(made.next_run_at, 10);

        assert!(record(&input("  ", 3_600), 0).is_err());
        assert!(record(&input("too often", 59), 0).is_err());
        assert!(record(&input("too rare", MAX_INTERVAL_SECS + 1), 0).is_err());
        assert!(record(&input(&"x".repeat(PROMPT_LIMIT + 1), 3_600), 0).is_err());
    }
}
