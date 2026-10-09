//! Research mandates on the desktop: the Automation page's commands and the
//! scheduler hook that runs a due round.
//!
//! A round is a normal agent turn in the mandate's one conversation, sent
//! through `send_message_inner` like a scheduled task. What makes it a
//! mandate turn is the conversation, not the trigger: `turn_context` gives
//! every turn there — a due round or the researcher typing — the same brief.

use crate::{create_session_frame, send_message_inner, AppState};
use tauri::{AppHandle, Manager, State};
use wisp_dto::{MandateDraft, MandateOverview, MandateRecord};
use wisp_store::Store;

fn err(error: anyhow::Error) -> String {
    error.to_string()
}

async fn load(store: &Store, id: &str) -> Result<MandateRecord, String> {
    store
        .get_mandate(id.trim())
        .await
        .map_err(err)?
        .ok_or_else(|| "The mandate no longer exists.".to_string())
}

/// What a turn in `frame_id` is given when that conversation is a mandate's.
pub(crate) struct TurnContext {
    pub(crate) mandate_id: String,
    /// Injected before the turn's message; never part of the saved history.
    pub(crate) brief: String,
    /// The mandate asks for approval before anything that changes state.
    pub(crate) review_mutations: bool,
}

pub(crate) async fn turn_context(
    store: &Store,
    project_id: &str,
    frame_id: &str,
) -> Option<TurnContext> {
    let mandate = match store.mandate_for_frame(project_id, frame_id).await {
        Ok(mandate) => mandate?,
        Err(error) => {
            tracing::warn!(target: "wisp", %error, frame_id, "failed to load the conversation's mandate");
            return None;
        }
    };
    // A closed mandate's conversation is an ordinary one again.
    if mandate.status == "done" {
        return None;
    }
    // The brief is still worth giving without its ledger.
    let rounds = store
        .mandate_rounds(&mandate.id, wisp_app::mandates::BRIEF_ROUNDS)
        .await
        .unwrap_or_default();
    Some(TurnContext {
        brief: wisp_app::mandates::brief(&mandate, &rounds),
        review_mutations: mandate.constraints.review_mutations,
        mandate_id: mandate.id,
    })
}

/// Mandates of the projects the researcher can see, oldest first.
pub(crate) async fn visible_mandates(store: &Store) -> Result<Vec<MandateOverview>, String> {
    let mut all = Vec::new();
    for project in crate::research_assistant::visible_projects(store).await? {
        // One offline project folder must not hide every other mandate.
        let mandates = match store.list_mandates(&project.0).await {
            Ok(mandates) => mandates,
            Err(error) => {
                tracing::warn!(target: "wisp", %error, project_id = %project.0, "failed to list mandates");
                continue;
            }
        };
        for mandate in mandates {
            let last_round = store
                .mandate_rounds(&mandate.id, 1)
                .await
                .map_err(err)?
                .pop();
            all.push(MandateOverview {
                mandate,
                last_round,
            });
        }
    }
    all.sort_by(|a, b| {
        (a.mandate.created_at, &a.mandate.id).cmp(&(b.mandate.created_at, &b.mandate.id))
    });
    Ok(all)
}

async fn require_visible_project(store: &Store, project_id: &str) -> Result<(), String> {
    if crate::research_assistant::visible_projects(store)
        .await?
        .iter()
        .any(|project| project.0 == project_id)
    {
        Ok(())
    } else {
        Err("The project no longer exists.".into())
    }
}

#[tauri::command]
pub(crate) async fn list_all_mandates(
    state: State<'_, AppState>,
) -> Result<Vec<MandateOverview>, String> {
    visible_mandates(&state.store).await
}

#[tauri::command]
pub(crate) async fn create_mandate(
    state: State<'_, AppState>,
    draft: MandateDraft,
) -> Result<MandateRecord, String> {
    let mandate = wisp_app::mandates::new_mandate(draft, chrono::Utc::now().timestamp())?;
    require_visible_project(&state.store, &mandate.project_id).await?;
    state.store.create_mandate(&mandate).await.map_err(err)?;
    Ok(mandate)
}

/// Edit the goal, KPIs, constraints, period and cadence. The next round keeps
/// its time; a round already running finishes under the brief it started with.
#[tauri::command]
pub(crate) async fn update_mandate(
    state: State<'_, AppState>,
    id: String,
    draft: MandateDraft,
) -> Result<MandateRecord, String> {
    let mut mandate = load(&state.store, &id).await?;
    wisp_app::mandates::apply_draft(&mut mandate, draft)?;
    mandate.updated_at = chrono::Utc::now().timestamp();
    if !state.store.update_mandate(&mandate).await.map_err(err)? {
        return Err("The mandate no longer exists.".into());
    }
    load(&state.store, &id).await
}

/// Pause, resume or close a mandate. Resuming a mandate whose round came due
/// while it was paused runs that round at the next poll.
#[tauri::command]
pub(crate) async fn set_mandate_status(
    state: State<'_, AppState>,
    id: String,
    status: String,
) -> Result<MandateRecord, String> {
    if !matches!(status.as_str(), "active" | "paused" | "done") {
        return Err("A mandate can be set to active, paused or done.".into());
    }
    let now = chrono::Utc::now().timestamp();
    if !state
        .store
        .set_mandate_status(id.trim(), &status, now)
        .await
        .map_err(err)?
    {
        return Err("The mandate no longer exists.".into());
    }
    load(&state.store, &id).await
}

/// Delete the mandate. Its conversation stays in the project as history.
#[tauri::command]
pub(crate) async fn delete_mandate(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.store.delete_mandate(id.trim()).await.map_err(err)
}

/// Run a round now, out of cadence: the next scheduled round keeps its time.
#[tauri::command]
pub(crate) async fn run_mandate_now(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> Result<(), String> {
    let mandate = load(&state.store, &id).await?;
    if mandate.status == "done" {
        return Err("This mandate is closed. Reopen it to run another round.".into());
    }
    tauri::async_runtime::spawn(async move {
        run_round(app, mandate, chrono::Utc::now().timestamp(), false).await;
    });
    Ok(())
}

/// Called from the scheduler poll; returns once the due rounds are spawned.
pub(crate) async fn fire_due_mandates(app: &AppHandle) {
    let state = app.state::<AppState>();
    let now = chrono::Utc::now().timestamp();
    let due = match state.store.due_mandates(now).await {
        Ok(due) => due,
        Err(error) => {
            tracing::warn!(target: "wisp", %error, "failed to poll due mandates");
            return;
        }
    };
    for mandate in due {
        // A round can run for minutes; never let one delay the others.
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            run_round(app, mandate, now, true).await;
        });
    }
}

/// What a due mandate does at `now`.
#[derive(Debug, PartialEq, Eq)]
enum Due {
    /// Its period is over: close it instead of running.
    Close,
    /// Today's rounds are used up: come back when the local day ends.
    Defer { until: i64 },
    /// Run a round; the default next round is at this time.
    Round { next_run_at: i64 },
}

/// `rounds_today` and `day_end` describe the local day `now` falls in.
fn due(mandate: &MandateRecord, now: i64, rounds_today: usize, day_end: i64) -> Due {
    if mandate.ends_at.is_some_and(|end| end <= now) {
        return Due::Close;
    }
    if rounds_today >= mandate.constraints.max_rounds_per_day as usize {
        return Due::Defer { until: day_end };
    }
    Due::Round {
        // From now, not from the missed slot: a mandate that slept through a
        // week owes one round, and its cadence restarts from it.
        next_run_at: now
            + mandate
                .interval_secs
                .max(wisp_app::mandates::MIN_ROUND_INTERVAL_SECS),
    }
}

/// Ledger entries of the local day `now` falls in, and when that day ends.
async fn rounds_today(store: &Store, mandate_id: &str, now: i64) -> (usize, i64) {
    let today = chrono::DateTime::from_timestamp(now, 0)
        .map(|t| t.with_timezone(&chrono::Local).date_naive())
        .and_then(crate::research_recap::local_day);
    let Some((start, end)) = today else {
        return (0, now + 86_400);
    };
    let rounds = store
        .mandate_rounds_between(mandate_id, start, end)
        .await
        .map_or(0, |rounds| rounds.len());
    (rounds, end)
}

/// Run one round. With `advance`, the round is first claimed atomically so a
/// mandate can never run the same slot twice; a manual round skips the claim
/// and the daily cap, because the researcher asked for it.
async fn run_round(app: AppHandle, mandate: MandateRecord, now: i64, advance: bool) {
    let state = app.state::<AppState>();
    let (rounds, day_end) = if advance {
        rounds_today(&state.store, &mandate.id, now).await
    } else {
        (0, now)
    };
    let next_run_at = match due(&mandate, now, rounds, day_end) {
        Due::Close => {
            if let Err(error) = state
                .store
                .set_mandate_status(&mandate.id, "done", now)
                .await
            {
                tracing::warn!(target: "wisp", %error, mandate_id = %mandate.id, "failed to close an ended mandate");
            }
            return;
        }
        Due::Defer { until } => {
            if let Err(error) = state
                .store
                .schedule_mandate_round(&mandate.id, until, mandate.wait_run_id.as_deref(), now)
                .await
            {
                tracing::warn!(target: "wisp", %error, mandate_id = %mandate.id, "failed to defer a capped mandate");
            }
            return;
        }
        Due::Round { next_run_at } => next_run_at,
    };
    if advance {
        match state
            .store
            .claim_mandate_round(&mandate.id, mandate.next_run_at, next_run_at, now)
            .await
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                tracing::warn!(target: "wisp", %error, mandate_id = %mandate.id, "failed to claim a mandate round");
                return;
            }
        }
    }
    let started = chrono::Utc::now().timestamp();
    let result = send_round(&app, &state, &mandate).await;
    if let Err(error) = &result {
        tracing::warn!(target: "wisp", %error, mandate_id = %mandate.id, "mandate round failed");
    }
    // A round that never called end_round still leaves a ledger entry, so
    // the next one is not briefed as if nothing happened.
    let reported = state
        .store
        .mandate_rounds(&mandate.id, 1)
        .await
        .is_ok_and(|rounds| {
            rounds
                .first()
                .is_some_and(|round| round.created_at >= started)
        });
    if reported {
        return;
    }
    let answer = match &result {
        Ok(frame_id) => crate::channels::last_assistant_text(&state.store, frame_id)
            .await
            .unwrap_or_default(),
        Err(error) => format!("The round failed before finishing: {error}"),
    };
    if let Err(error) = wisp_app::mandates::record_unreported_round(
        &state.store,
        &mandate.id,
        &answer,
        chrono::Utc::now().timestamp(),
    )
    .await
    {
        tracing::warn!(target: "wisp", %error, mandate_id = %mandate.id, "failed to record an unreported round");
    }
}

/// The mandate's conversation, opening a fresh one when it has none yet or
/// the old one was deleted or archived.
async fn round_frame(store: &Store, mandate: &MandateRecord) -> Result<String, String> {
    if let Some(frame_id) = mandate.frame_id.as_deref() {
        let live = store
            .live_frame_project_id(frame_id)
            .await
            .map_err(err)?
            .is_some();
        if live && store.require_unarchived_session(frame_id).await.is_ok() {
            return Ok(frame_id.to_string());
        }
    }
    let frame_id = create_session_frame(store, &mandate.project_id).await?;
    store
        .rename_session(&frame_id, &mandate.project_id, &mandate.name)
        .await
        .map_err(err)?;
    store
        .set_mandate_frame(&mandate.id, Some(&frame_id))
        .await
        .map_err(err)?;
    Ok(frame_id)
}

async fn send_round(
    app: &AppHandle,
    state: &State<'_, AppState>,
    mandate: &MandateRecord,
) -> Result<String, String> {
    let frame_id = round_frame(&state.store, mandate).await?;
    send_message_inner(
        state.inner(),
        app.clone(),
        "main",
        Some(frame_id),
        wisp_app::mandates::round_prompt(mandate),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        crate::TurnOrigin::Desktop,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("wisp.sqlite")).await.unwrap();
        store.create_project("p1", "RNA-seq", "").await.unwrap();
        (store, dir)
    }

    fn draft(project_id: &str) -> MandateDraft {
        MandateDraft {
            project_id: project_id.into(),
            goal: "Track new single-cell papers.".into(),
            interval_secs: 86_400,
            ..Default::default()
        }
    }

    #[test]
    fn a_due_mandate_runs_until_its_period_ends() {
        let mut mandate = wisp_app::mandates::new_mandate(draft("p1"), 1_000).unwrap();
        assert_eq!(
            due(&mandate, 500_000, 0, 600_000),
            Due::Round {
                next_run_at: 500_000 + 86_400
            },
            "a missed slot owes one round, counted from now"
        );
        mandate.constraints.max_rounds_per_day = 2;
        assert!(matches!(
            due(&mandate, 500_000, 1, 600_000),
            Due::Round { .. }
        ));
        assert_eq!(
            due(&mandate, 500_000, 2, 600_000),
            Due::Defer { until: 600_000 },
            "today's rounds are used up"
        );
        mandate.ends_at = Some(500_000);
        assert!(matches!(
            due(&mandate, 499_999, 0, 600_000),
            Due::Round { .. }
        ));
        assert_eq!(
            due(&mandate, 500_000, 2, 600_000),
            Due::Close,
            "the period ending wins over the cap"
        );
    }

    #[tokio::test]
    async fn todays_rounds_are_counted_within_the_local_day() {
        let (store, _dir) = store().await;
        let mandate = wisp_app::mandates::new_mandate(draft("p1"), 1_000).unwrap();
        store.create_mandate(&mandate).await.unwrap();
        let now = chrono::Utc::now().timestamp();
        assert_eq!(rounds_today(&store, &mandate.id, now).await.0, 0);
        wisp_app::mandates::record_unreported_round(&store, &mandate.id, "Checked", now)
            .await
            .unwrap();
        // Two days back is another local day in every timezone.
        wisp_app::mandates::record_unreported_round(&store, &mandate.id, "Old", now - 2 * 86_400)
            .await
            .unwrap();
        let (rounds, day_end) = rounds_today(&store, &mandate.id, now).await;
        assert_eq!(rounds, 1);
        assert!(day_end > now && day_end <= now + 25 * 3_600);
    }

    #[tokio::test]
    async fn a_mandate_conversation_gets_the_brief_until_the_mandate_closes() {
        let (store, _dir) = store().await;
        store.create_frame("f1", "p1", "OPERON", "m").await.unwrap();
        store.create_frame("f2", "p1", "OPERON", "m").await.unwrap();
        let mut mandate = wisp_app::mandates::new_mandate(draft("p1"), 1_000).unwrap();
        mandate.frame_id = Some("f1".into());
        store.create_mandate(&mandate).await.unwrap();

        let context = turn_context(&store, "p1", "f1").await.unwrap();
        assert_eq!(context.mandate_id, mandate.id);
        assert!(context
            .brief
            .contains("Goal: Track new single-cell papers."));
        assert!(context.brief.contains("no round has reported yet"));
        assert!(context.review_mutations, "review is the default");
        wisp_app::mandates::record_unreported_round(&store, &mandate.id, "Queued 3 runs", 1_500)
            .await
            .unwrap();
        let context = turn_context(&store, "p1", "f1").await.unwrap();
        assert!(context.brief.contains("Done: Queued 3 runs"));
        assert!(turn_context(&store, "p1", "f2").await.is_none());

        store
            .set_mandate_status(&mandate.id, "paused", 2_000)
            .await
            .unwrap();
        assert!(
            turn_context(&store, "p1", "f1").await.is_some(),
            "the researcher can still work with a paused mandate"
        );
        store
            .set_mandate_status(&mandate.id, "done", 2_000)
            .await
            .unwrap();
        assert!(turn_context(&store, "p1", "f1").await.is_none());
    }

    #[tokio::test]
    async fn a_round_reuses_the_conversation_and_replaces_a_deleted_one() {
        let (store, _dir) = store().await;
        let mut mandate = wisp_app::mandates::new_mandate(draft("p1"), 1_000).unwrap();
        store.create_mandate(&mandate).await.unwrap();

        let first = round_frame(&store, &mandate).await.unwrap();
        mandate = store.get_mandate(&mandate.id).await.unwrap().unwrap();
        assert_eq!(mandate.frame_id.as_deref(), Some(first.as_str()));
        assert_eq!(round_frame(&store, &mandate).await.unwrap(), first);
        let title: String = store
            .get_session_reference(&first)
            .await
            .unwrap()
            .unwrap()
            .title;
        assert_eq!(title, "Track new single-cell papers.");

        store.delete_session(&first, "p1").await.unwrap();
        let second = round_frame(&store, &mandate).await.unwrap();
        assert_ne!(second, first);
        assert_eq!(
            store
                .get_mandate(&mandate.id)
                .await
                .unwrap()
                .unwrap()
                .frame_id,
            Some(second)
        );
    }

    #[tokio::test]
    async fn listing_skips_projects_hidden_by_privacy_mode() {
        let (store, _dir) = store().await;
        store.create_project("p2", "Private", "").await.unwrap();
        for project in ["p1", "p2"] {
            let mandate = wisp_app::mandates::new_mandate(draft(project), 1_000).unwrap();
            store.create_mandate(&mandate).await.unwrap();
        }
        assert_eq!(visible_mandates(&store).await.unwrap().len(), 2);
        crate::privacy_mode::save(&store, true, &["p2".into()])
            .await
            .unwrap();
        let shown = visible_mandates(&store).await.unwrap()[0]
            .mandate
            .id
            .clone();
        wisp_app::mandates::record_unreported_round(&store, &shown, "Filed 2 papers", 1_500)
            .await
            .unwrap();
        let visible = visible_mandates(&store).await.unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].mandate.project_id, "p1");
        assert_eq!(
            visible[0]
                .last_round
                .as_ref()
                .map(|round| round.done.as_str()),
            Some("Filed 2 papers")
        );
        assert!(require_visible_project(&store, "p2").await.is_err());
        // The payload round-trips through the shared UI contract.
        let wire = serde_json::to_value(&visible).unwrap();
        assert_eq!(
            serde_json::from_value::<Vec<MandateOverview>>(wire).unwrap(),
            visible
        );
    }
}
