//! Research mandates, a port of the WebView's `src-tauri/src/mandates.rs`:
//! a standing goal a project's agent works on in rounds, on a cadence,
//! in one conversation of the project.
//!
//! A round is an ordinary turn in that conversation, started like a
//! scheduled task. What makes it a mandate turn is the conversation, not the
//! trigger: every turn there (a due round or the researcher typing) is given
//! the mandate's brief and its two tools, `end_round` and
//! `request_assistance`, from `wisp-app`. When the mandate says so (the
//! default), its turns ask before anything that changes state. A request
//! for the researcher is added to the research assistant's conversation;
//! chat apps are not told.

use anyhow::{Context as _, Result};
use wisp_dto::{MandateDraft, MandateOverview, MandateRecord, MandateRequest};
use wisp_store::Store;

use crate::Server;

/// Reports shown on a mandate's card.
pub(crate) const CARD_REPORTS: usize = 5;

/// What a turn in a mandate's conversation is given.
pub(crate) struct TurnContext {
    pub mandate_id: String,
    /// Told before the turn's message; never part of the saved history.
    pub brief: String,
    /// The mandate asks for approval before anything that changes state.
    pub review_mutations: bool,
}

/// `human_message` is the turn's message when the researcher wrote it (not
/// a resume or a timer). While the mandate waits on a request, such a
/// message is the answer: it closes the request before the brief is built,
/// so the turn that carries it is the round that acts on it.
pub(crate) async fn turn_context(
    store: &Store,
    project_id: &str,
    frame_id: &str,
    human_message: Option<&str>,
) -> Option<TurnContext> {
    let mut mandate = match store.mandate_for_frame(project_id, frame_id).await {
        Ok(mandate) => mandate?,
        Err(error) => {
            tracing::warn!(%error, frame_id, "Could not read the conversation's mandate");
            return None;
        }
    };
    // A closed mandate's conversation is an ordinary one again.
    if mandate.status == "done" {
        return None;
    }
    if let Some(reply) = human_message
        .filter(|text| mandate.status == "waiting" && !wisp_app::mandates::is_round_prompt(text))
    {
        let now = chrono::Utc::now().timestamp();
        match wisp_app::mandates::answer_request(store, &mandate.id, reply, now).await {
            Ok(Some(_)) => mandate.status = "active".into(),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, mandate = %mandate.id, "Could not record the researcher's answer")
            }
        }
    }
    // The brief is still worth giving without its ledger or request.
    let rounds = store
        .mandate_rounds(&mandate.id, wisp_app::mandates::BRIEF_ROUNDS)
        .await
        .unwrap_or_default();
    let request = store
        .latest_mandate_request(&mandate.id)
        .await
        .unwrap_or_default();
    Some(TurnContext {
        brief: wisp_app::mandates::brief(&mandate, &rounds, request.as_ref()),
        review_mutations: mandate.constraints.review_mutations,
        mandate_id: mandate.id,
    })
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "login" => "登录 / Sign-in",
        "materials" => "补充资料 / Materials",
        "payment" => "付款授权 / Payment",
        "release" => "发布 / Release",
        _ => "需要你判断 / Your judgement",
    }
}

/// What the researcher reads; it has to be actionable without opening the
/// conversation.
fn request_notice(project: &str, mandate: &MandateRecord, request: &MandateRequest) -> String {
    let mut lines = vec![
        "职责需要你 / A mandate needs you".to_string(),
        format!("{project} · {}", mandate.name),
        format!("[{}] {}", kind_label(&request.kind), request.what),
    ];
    if !request.why.is_empty() {
        lines.push(format!("原因 / Why: {}", request.why));
    }
    if !request.then_what.is_empty() {
        lines.push(format!("之后 / Then: {}", request.then_what));
    }
    lines.push(
        "把答复告诉科研助理，或在「研究职责」页的卡片上、该项目的职责对话里回复，它就会继续。 / Tell the research assistant your answer, or reply on its card on the Research mandates page or in the mandate's conversation, and it continues."
            .into(),
    );
    lines.join("\n")
}

fn status_label(status: &str) -> &'static str {
    match status {
        "paused" => "已暂停 / paused",
        "waiting" => "等你处理 / needs you",
        "done" => "已结束 / closed",
        _ => "进行中 / active",
    }
}

fn local_minute(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn clip(text: &str, chars: usize) -> String {
    crate::recaps::clip(text, chars)
}

/// The mandates of every project, oldest first.
pub(crate) async fn all_mandates(store: &Store) -> Result<Vec<MandateOverview>> {
    let mut all = Vec::new();
    for (project, ..) in store.list_projects().await? {
        // One unreadable project must not hide every other mandate.
        let mandates = match store.list_mandates(&project).await {
            Ok(mandates) => mandates,
            Err(error) => {
                tracing::warn!(%error, project, "Could not list mandates");
                continue;
            }
        };
        for mandate in mandates {
            let last_round = store.mandate_rounds(&mandate.id, 1).await?.pop();
            let request = store
                .latest_mandate_request(&mandate.id)
                .await?
                .filter(|request| request.status == "open");
            let reports = store.mandate_reports(&mandate.id, CARD_REPORTS).await?;
            all.push(MandateOverview {
                mandate,
                last_round,
                request,
                reports,
            });
        }
    }
    all.sort_by(|a, b| {
        (a.mandate.created_at, &a.mandate.id).cmp(&(b.mandate.created_at, &b.mandate.id))
    });
    Ok(all)
}

/// Every mandate at a glance, for the research assistant: status, KPIs, the
/// latest round, the next one, and what it waits on, each with its id.
pub(crate) async fn summary_text(store: &Store) -> Result<String> {
    let mandates = all_mandates(store).await?;
    if mandates.is_empty() {
        return Ok("还没有研究职责。 / No research mandates yet.".into());
    }
    let names: std::collections::HashMap<_, _> = store
        .list_projects()
        .await?
        .into_iter()
        .map(|project| (project.0, project.1))
        .collect();
    let mut lines = vec!["研究职责 / Research mandates".to_string()];
    for (index, overview) in mandates.iter().enumerate() {
        let mandate = &overview.mandate;
        let project = names
            .get(&mandate.project_id)
            .map_or(mandate.project_id.as_str(), String::as_str);
        lines.push(format!(
            "{}. [{}] {project} · {} (id: {})",
            index + 1,
            status_label(&mandate.status),
            mandate.name,
            mandate.id
        ));
        for kpi in &mandate.kpis {
            let mut line = format!(
                "   KPI {}: {}",
                kpi.name,
                kpi.current.as_deref().unwrap_or("–")
            );
            if !kpi.target.is_empty() {
                line.push_str(&format!(" / {} {}", kpi.target, kpi.period));
            }
            lines.push(line.trim_end().to_string());
        }
        if let Some(round) = &overview.last_round {
            let mut line = format!(
                "   上一轮 / Last round #{} ({}): {}",
                round.seq,
                local_minute(round.created_at),
                clip(&round.done, 200)
            );
            if !round.blockers.is_empty() {
                line.push_str(&format!(
                    " | 阻塞 / Blocked: {}",
                    clip(&round.blockers, 200)
                ));
            }
            lines.push(line);
        }
        if mandate.status == "active" {
            lines.push(format!(
                "   下一轮 / Next round: {}",
                local_minute(mandate.next_run_at)
            ));
        }
        if let Some(request) = &overview.request {
            lines.push(format!(
                "   等你处理 / Needs you: [{}] {}",
                kind_label(&request.kind),
                clip(&request.what, 300)
            ));
        }
    }
    Ok(lines.join("\n"))
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
        .and_then(crate::recaps::local_day);
    let Some((start, end)) = today else {
        return (0, now + 86_400);
    };
    let rounds = store
        .mandate_rounds_between(mandate_id, start, end)
        .await
        .map_or(0, |rounds| rounds.len());
    (rounds, end)
}

impl Server {
    async fn mandate(&self, id: &str) -> Result<MandateRecord> {
        self.0
            .store
            .get_mandate(id.trim())
            .await?
            .context("The mandate no longer exists.")
    }

    /// The mandates of every project, with each one's latest round, open
    /// request and reports.
    pub async fn mandates(&self) -> Result<Vec<MandateOverview>> {
        all_mandates(&self.0.store).await
    }

    pub async fn create_mandate(&self, draft: MandateDraft) -> Result<MandateRecord> {
        let mandate = wisp_app::mandates::new_mandate(draft, chrono::Utc::now().timestamp())
            .map_err(anyhow::Error::msg)?;
        self.known_project(&mandate.project_id).await?;
        self.0.store.create_mandate(&mandate).await?;
        Ok(mandate)
    }

    /// Changes the goal, KPIs, constraints, period and cadence. The next
    /// round keeps its time; a round already running finishes under the
    /// brief it started with.
    pub async fn update_mandate(&self, id: &str, draft: MandateDraft) -> Result<MandateRecord> {
        let mut mandate = self.mandate(id).await?;
        wisp_app::mandates::apply_draft(&mut mandate, draft).map_err(anyhow::Error::msg)?;
        mandate.updated_at = chrono::Utc::now().timestamp();
        anyhow::ensure!(
            self.0.store.update_mandate(&mandate).await?,
            "The mandate no longer exists."
        );
        self.mandate(id).await
    }

    /// Pauses, resumes, closes or reopens a mandate. Closing withdraws an
    /// unanswered request and runs no round again.
    pub async fn set_mandate_status(&self, id: &str, status: &str) -> Result<MandateRecord> {
        anyhow::ensure!(
            matches!(status, "active" | "paused" | "done"),
            "A mandate can be set to active, paused or done."
        );
        let mandate = self.mandate(id).await?;
        let now = chrono::Utc::now().timestamp();
        if status == "done" {
            // Closing twice writes nothing twice.
            if mandate.status != "done" {
                self.close_mandate(&mandate.id, now).await?;
            }
        } else {
            anyhow::ensure!(
                self.0
                    .store
                    .set_mandate_status(&mandate.id, status, now)
                    .await?,
                "The mandate no longer exists."
            );
        }
        self.mandate(id).await
    }

    pub(crate) async fn close_mandate(&self, id: &str, now: i64) -> Result<()> {
        let store = &self.0.store;
        anyhow::ensure!(
            store.set_mandate_status(id, "done", now).await?,
            "The mandate no longer exists."
        );
        store.withdraw_mandate_requests(id).await?;
        Ok(())
    }

    /// Deletes the mandate; its conversation stays in the project.
    pub async fn delete_mandate(&self, id: &str) -> Result<()> {
        self.0.store.delete_mandate(id.trim()).await
    }

    /// Runs a round now, out of cadence: the next scheduled round keeps its
    /// time.
    pub async fn run_mandate_now(&self, id: &str) -> Result<()> {
        let mandate = self.mandate(id).await?;
        anyhow::ensure!(
            mandate.status != "done",
            "This mandate is closed. Reopen it to run another round."
        );
        let server = self.clone();
        tokio::spawn(async move {
            server
                .run_round(mandate, chrono::Utc::now().timestamp(), false)
                .await
        });
        Ok(())
    }

    /// Answers the mandate's open request, or simply tells it something,
    /// and starts the turn that acts on it.
    pub async fn reply_to_mandate(&self, id: &str, text: &str) -> Result<MandateRecord> {
        let text = text.trim();
        anyhow::ensure!(!text.is_empty(), "Write a reply first.");
        let mandate = self.mandate(id).await?;
        anyhow::ensure!(mandate.status != "done", "This mandate is closed.");
        // Answered before the turn starts, so the card stops waiting at once
        // and a second reply cannot land on the same request.
        wisp_app::mandates::answer_request(
            &self.0.store,
            &mandate.id,
            text,
            chrono::Utc::now().timestamp(),
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let frame = self.round_frame(&mandate).await?;
        self.send_to_mandate(&mandate.project_id, &frame, text.to_owned())
            .await?;
        self.mandate(id).await
    }

    /// Called with the scheduler's poll; starts the rounds that are due.
    pub(crate) async fn fire_due_mandates(&self) {
        let store = &self.0.store;
        let now = chrono::Utc::now().timestamp();
        // A finished run pulls its mandate's next round forward into this scan.
        if let Err(error) = wisp_app::mandates::wake_on_finished_runs(store, now).await {
            tracing::warn!(%error, "Could not check the runs mandates wait on");
        }
        let due = match store.due_mandates(now).await {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(%error, "Could not look for due mandates");
                return;
            }
        };
        for mandate in due {
            // A round can run for minutes; one never delays the others.
            let server = self.clone();
            tokio::spawn(async move { server.run_round(mandate, now, true).await });
        }
    }

    /// Runs one round. With `advance`, the round is first claimed so a
    /// mandate never runs the same slot twice; a round asked for now skips
    /// the claim and the daily cap.
    async fn run_round(&self, mandate: MandateRecord, now: i64, advance: bool) {
        let store = &self.0.store;
        let (rounds, day_end) = if advance {
            rounds_today(store, &mandate.id, now).await
        } else {
            (0, now)
        };
        let next_run_at = match due(&mandate, now, rounds, day_end) {
            Due::Close => {
                if let Err(error) = self.close_mandate(&mandate.id, now).await {
                    tracing::warn!(%error, mandate = %mandate.id, "Could not close an ended mandate");
                }
                return;
            }
            Due::Defer { until } => {
                if let Err(error) = store
                    .schedule_mandate_round(&mandate.id, until, mandate.wait_run_id.as_deref(), now)
                    .await
                {
                    tracing::warn!(%error, mandate = %mandate.id, "Could not defer a capped mandate");
                }
                return;
            }
            Due::Round { next_run_at } => next_run_at,
        };
        if advance {
            match store
                .claim_mandate_round(&mandate.id, mandate.next_run_at, next_run_at, now)
                .await
            {
                Ok(true) => {}
                Ok(false) => return,
                Err(error) => {
                    tracing::warn!(%error, mandate = %mandate.id, "Could not claim a mandate round");
                    return;
                }
            }
        }
        let started = chrono::Utc::now().timestamp();
        let result = async {
            let frame = self.round_frame(&mandate).await?;
            self.send_to_mandate(
                &mandate.project_id,
                &frame,
                wisp_app::mandates::round_prompt(&mandate),
            )
            .await?;
            // The round is the turn: wait for it to end.
            let live = self.session(&frame);
            while live.live.lock().unwrap().running {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Ok::<_, anyhow::Error>(frame)
        }
        .await;
        if let Err(error) = &result {
            tracing::warn!(%error, mandate = %mandate.id, "Mandate round failed");
        }
        // A round that never called end_round still leaves a ledger entry,
        // so the next one is not briefed as if nothing happened.
        let reported = store
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
            Ok(frame) => store
                .load_messages(frame)
                .await
                .ok()
                .and_then(|messages| crate::channels::final_answer(&messages))
                .unwrap_or_default(),
            Err(error) => format!("The round failed before finishing: {error:#}"),
        };
        if let Err(error) = wisp_app::mandates::record_unreported_round(
            store,
            &mandate.id,
            &answer,
            chrono::Utc::now().timestamp(),
        )
        .await
        {
            tracing::warn!(%error, mandate = %mandate.id, "Could not record an unreported round");
        }
    }

    /// The mandate's conversation, opening a new one when it has none yet
    /// or the old one was deleted or archived.
    async fn round_frame(&self, mandate: &MandateRecord) -> Result<String> {
        let store = &self.0.store;
        if let Some(frame) = mandate.frame_id.as_deref() {
            let live = store.live_frame_project_id(frame).await?.is_some();
            if live && !self.archived(frame).await? {
                return Ok(frame.to_owned());
            }
        }
        let frame = self.create_conversation(&mandate.project_id).await?;
        store
            .rename_session(&frame, &mandate.project_id, &mandate.name)
            .await?;
        store.set_mandate_frame(&mandate.id, Some(&frame)).await?;
        Ok(frame)
    }

    async fn send_to_mandate(&self, project: &str, frame: &str, message: String) -> Result<()> {
        let root = self.project_root(project).await?;
        let request = wisp_dto::native_conversations::SendRequest {
            session_id: frame.to_owned(),
            request_id: uuid::Uuid::new_v4().to_string(),
            message,
            resume: false,
            resume_after: None,
            attachments: Vec::new(),
            references: Vec::new(),
            acp_agent_id: None,
        };
        self.start_turn(
            project,
            &root,
            request,
            crate::channels::TurnOrigin::Desktop,
        )
        .await?;
        // A mandate's conversation keeps the mandate's name.
        if let Some(mandate) = self.0.store.mandate_for_frame(project, frame).await? {
            self.0
                .store
                .rename_session(frame, project, &mandate.name)
                .await?;
        }
        Ok(())
    }

    /// Puts `text` in front of the researcher: in the research assistant's
    /// conversation, once it is at rest.
    pub(crate) async fn announce(&self, text: &str) -> Result<()> {
        let (_, frame) = self.open_assistant().await?;
        let assistant = self.session(&frame);
        let store = &self.0.store;
        loop {
            {
                let _accept = assistant.accept.lock().await;
                if !assistant.live.lock().unwrap().running {
                    let seq = store.max_message_seq(&frame).await? + 1;
                    store
                        .append_message(&frame, seq, &wisp_llm::Message::assistant(text))
                        .await?;
                    let mut live = assistant.live.lock().unwrap();
                    live.generation = live.generation.wrapping_add(1);
                    return Ok(());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// The mandate tools' way of reaching the researcher. The notice goes
    /// in the background: the round that asked does not wait on it.
    pub(crate) fn assistance_notifier(&self) -> wisp_app::mandates::AssistanceNotifier {
        let server = self.clone();
        std::sync::Arc::new(move |mandate: MandateRecord, request: MandateRequest| {
            let server = server.clone();
            tokio::spawn(async move {
                let project = match server.0.store.get_project(&mandate.project_id).await {
                    Ok(Some((name, _))) => name,
                    _ => mandate.project_id.clone(),
                };
                let text = request_notice(&project, &mandate, &request);
                if let Err(error) = server.announce(&text).await {
                    tracing::warn!(%error, mandate = %mandate.id, "Could not announce a mandate's request");
                }
            });
        })
    }

    /// The tools a mandate's turns work with.
    pub(crate) fn mandate_tools(&self, mandate: &str) -> Vec<Box<dyn wisp_tools::Tool>> {
        vec![
            Box::new(wisp_app::mandates::EndRoundTool::new(
                self.0.store.clone(),
                mandate,
            )),
            Box::new(wisp_app::mandates::RequestAssistanceTool::new(
                self.0.store.clone(),
                mandate,
                Some(self.assistance_notifier()),
            )),
        ]
    }
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

        let context = turn_context(&store, "p1", "f1", None).await.unwrap();
        assert_eq!(context.mandate_id, mandate.id);
        assert!(context
            .brief
            .contains("Goal: Track new single-cell papers."));
        assert!(context.brief.contains("no round has reported yet"));
        assert!(context.review_mutations, "review is the default");
        wisp_app::mandates::record_unreported_round(&store, &mandate.id, "Queued 3 runs", 1_500)
            .await
            .unwrap();
        let context = turn_context(&store, "p1", "f1", None).await.unwrap();
        assert!(context.brief.contains("Done: Queued 3 runs"));
        assert!(turn_context(&store, "p1", "f2", None).await.is_none());

        store
            .set_mandate_status(&mandate.id, "paused", 2_000)
            .await
            .unwrap();
        assert!(
            turn_context(&store, "p1", "f1", None).await.is_some(),
            "the researcher can still work with a paused mandate"
        );
        store
            .set_mandate_status(&mandate.id, "done", 2_000)
            .await
            .unwrap();
        assert!(turn_context(&store, "p1", "f1", None).await.is_none());
        let summary = summary_text(&store).await.unwrap();
        assert!(summary.contains("RNA-seq · "), "{summary}");
        assert!(summary.contains(&mandate.id));
    }
}
