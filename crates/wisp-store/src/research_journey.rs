use super::{StateScope, Store};
use anyhow::{bail, Result};
use sqlx::Row;
use wisp_dto::{
    ResearchJournalInput, ResearchJourney, ResearchJourneyEntry, ResearchJourneyInput,
    ResearchJourneySource, ResearchRecap, ResearchRecapEdit, ResearchRecapItem,
};

const ENTRY_LIMIT: usize = 2000;
const RECAP_STATUSES: [&str; 3] = ["draft", "confirmed", "dismissed"];
const RECAP_ITEMS: usize = 12;

fn validate_recap(recap: &ResearchRecap) -> Result<()> {
    if !RECAP_STATUSES.contains(&recap.status.as_str()) {
        bail!("Unknown recap status");
    }
    if recap.headline.trim().is_empty() || recap.headline.chars().count() > 200 {
        bail!("A recap needs a headline of 1–200 characters");
    }
    for items in [&recap.done, &recap.findings, &recap.issues, &recap.next] {
        if items.len() > RECAP_ITEMS
            || items.iter().any(|item| {
                item.text.trim().is_empty()
                    || item.text.chars().count() > 1000
                    || item.refs.iter().any(|r| *r >= recap.sources.len())
            })
        {
            bail!("Each recap section holds at most 12 non-empty items that cite known sources");
        }
    }
    Ok(())
}

fn recap_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ResearchRecap> {
    let mut recap: ResearchRecap = serde_json::from_str(row.try_get("recap_json")?)?;
    recap.id = row.try_get("id")?;
    recap.day_start = row.try_get("day_start")?;
    recap.status = row.try_get("status")?;
    Ok(recap)
}

/// Lines the researcher left unchanged keep their sources; rewritten text
/// no longer claims the generated citations.
fn keep_refs(edited: &[ResearchRecapItem], before: &[ResearchRecapItem]) -> Vec<ResearchRecapItem> {
    edited
        .iter()
        .map(|item| ResearchRecapItem {
            text: item.text.trim().to_string(),
            refs: before
                .iter()
                .find(|old| old.text == item.text.trim())
                .map(|old| old.refs.clone())
                .unwrap_or_default(),
        })
        .collect()
}

/// Read visibility is intentionally the same as the graph: branch-owned objects
/// and frozen baseline objects, never sibling exploration state.
fn visible(alias: &str, entity: &str) -> String {
    format!("{alias}.project_id=?1 AND ((?2 IS NULL AND {alias}.exploration_id IS NULL) OR {alias}.exploration_id=?2 OR ({alias}.exploration_id IS NULL AND EXISTS(SELECT 1 FROM explorations x JOIN exploration_baseline_entities b ON b.checkpoint_id=x.checkpoint_id WHERE x.id=?2 AND b.entity_kind='{entity}' AND b.entity_id={alias}.id)))")
}

impl Store {
    /// Read explicitly requested projects without changing the active project
    /// or following any project's active exploration branch.
    pub async fn research_calendar(
        &self,
        project_ids: &[String],
        from: i64,
        until: i64,
    ) -> Result<Vec<wisp_dto::ResearchCalendarProject>> {
        if from >= until || until.saturating_sub(from) > 32 * 86400 {
            bail!("Research history requires a date range of at most 32 days");
        }
        let mut projects = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in project_ids {
            if !seen.insert(id) {
                continue;
            }
            let result = async {
                if self.get_project(id).await?.is_none() {
                    bail!("Project no longer exists");
                }
                self.research_journey(&StateScope::mainline(id), from, until)
                    .await
            }
            .await;
            let (history, error) = match result {
                Ok(history) => (history, None),
                Err(error) => (ResearchJourney::default(), Some(error.to_string())),
            };
            projects.push(wisp_dto::ResearchCalendarProject {
                project_id: id.clone(),
                history,
                error,
            });
        }
        Ok(projects)
    }

    pub async fn research_journey(
        &self,
        scope: &StateScope,
        from: i64,
        until: i64,
    ) -> Result<ResearchJourney> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.research_journey(scope, from, until)).await;
        }
        scope.validate()?;
        if from >= until || until.saturating_sub(from) > 32 * 86400 {
            bail!("Research history requires a date range of at most 32 days");
        }
        let exploration = match scope {
            StateScope::Mainline { .. } => None,
            StateScope::Exploration { exploration_id, .. } => Some(exploration_id.as_str()),
        };
        let runs = visible("r", "run");
        let nodes = visible("n", "research_node");
        // Only baseline artifact versions are visible from an exploration.
        let artifacts = "a.project_id=?1 AND ((?2 IS NULL AND a.exploration_id IS NULL) OR a.exploration_id=?2 OR (a.exploration_id IS NULL AND EXISTS(SELECT 1 FROM explorations x JOIN exploration_baseline_artifact_heads b ON b.checkpoint_id=x.checkpoint_id WHERE x.id=?2 AND b.artifact_version_id=v.id)))";
        let query = format!(
            r#"
            SELECT * FROM (
              SELECT 'run-start:'||r.id AS id, 'run' AS kind, r.title, '' AS summary,
                COALESCE(r.started_at,r.created_at) AS occurred_at, r.created_at AS recorded_at,
                r.id AS source_id, r.frame_id, CASE WHEN r.started_at IS NULL THEN 'submitted' ELSE 'started' END AS status, '' AS content_type,
                NULL AS version_number, 0 AS source_discarded, 0 AS manual, r.id AS run_id
              FROM runs r WHERE {runs} AND r.kind<>'file_transfer'
              UNION ALL
              SELECT 'run-end:'||r.id, 'run', r.title, '', r.ended_at, r.ended_at,
                r.id, r.frame_id, r.status, '', NULL, 0, 0, r.id FROM runs r WHERE {runs} AND r.kind<>'file_transfer' AND r.ended_at IS NOT NULL
              UNION ALL
              SELECT 'version:'||v.id, 'artifact', a.filename, '', v.created_at, v.created_at,
                v.id, a.root_frame_id, 'registered', v.content_type, v.version_number,
                v.source_discarded_at IS NOT NULL, 0, v.producing_run_id
              FROM artifact_versions v JOIN artifacts a ON a.id=v.artifact_id WHERE {artifacts}
              UNION ALL
              SELECT 'node:'||n.id, n.kind, n.title,
                CAST(COALESCE(json_extract(n.metadata_json,'$.body'),json_extract(n.metadata_json,'$.reason'),json_extract(n.metadata_json,'$.rationale'),json_extract(n.metadata_json,'$.summary'),'') AS TEXT),
                n.created_at, n.created_at, n.id, NULL, 'recorded', '', NULL, 0, 0, NULL
              FROM research_nodes n WHERE {nodes} AND n.kind IN ('decision','paper','data_asset')
              UNION ALL
              SELECT 'message:'||m.id, 'session', COALESCE(NULLIF(f.title,''),substr(m.content,1,120),'Conversation'),
                '', m.ts, m.ts, f.id, f.id, 'discussed', '', NULL, 0, 0, NULL
              FROM messages m JOIN frames f ON f.id=m.frame_id
              WHERE f.project_id=?1 AND ((?2 IS NULL AND f.exploration_id IS NULL) OR f.exploration_id=?2)
                AND m.role='user' AND trim(COALESCE(m.content,''))<>''
                AND NOT EXISTS (SELECT 1 FROM context_epochs ce WHERE ce.frame_id=m.frame_id
                    AND m.seq BETWEEN ce.first_seq AND ce.initial_head_seq)
              UNION ALL
              SELECT 'journal:'||j.id, j.category, j.title, j.body, j.occurred_at, j.created_at,
                j.id, NULL, 'recorded', '', NULL, 0, 1, NULL FROM research_journal_entries j
              WHERE j.project_id=?1 AND ((?2 IS NULL AND j.exploration_id IS NULL) OR j.exploration_id=?2
                OR (j.exploration_id IS NULL AND EXISTS(SELECT 1 FROM explorations x
                    JOIN exploration_baseline_entities b ON b.checkpoint_id=x.checkpoint_id WHERE x.id=?2 AND b.entity_kind='research_journal_entry' AND b.entity_id=j.id)))
              UNION ALL
              SELECT 'archive:'||a.id, 'archive', a.title, json_extract(a.record_json,'$.report'), a.frozen_at, a.created_at,
                a.id, a.frame_id, 'archived', '', NULL, 0, 0, NULL FROM research_archives a
              WHERE a.project_id=?1 AND ?2 IS NULL AND a.frozen_at IS NOT NULL
              UNION ALL
              SELECT 'archive-continuation:'||c.frame_id, 'progress', 'Continue research / 继续研究', a.title,
                f.created_at,f.created_at,a.id,c.frame_id,'continued','',NULL,0,0,NULL
              FROM research_archive_continuations c JOIN research_archives a ON a.id=c.archive_id JOIN frames f ON f.id=c.frame_id
              WHERE a.project_id=?1 AND ?2 IS NULL
            ) WHERE occurred_at>=?3 AND occurred_at<?4 ORDER BY occurred_at DESC, id DESC LIMIT 2001
        "#
        );
        let rows = sqlx::query(&query)
            .bind(scope.project_id())
            .bind(exploration)
            .bind(from)
            .bind(until)
            .fetch_all(&self.pool)
            .await?;
        let truncated = rows.len() > ENTRY_LIMIT;
        // Recaps summarize the mainline only.
        let recaps = match exploration {
            None => {
                self.research_recaps(scope.project_id(), from, until)
                    .await?
            }
            Some(_) => Vec::new(),
        };
        let entries = rows
            .into_iter()
            .take(ENTRY_LIMIT)
            .map(|row| {
                Ok(ResearchJourneyEntry {
                    id: row.try_get("id")?,
                    kind: row.try_get("kind")?,
                    title: row.try_get("title")?,
                    summary: row.try_get("summary")?,
                    occurred_at: row.try_get("occurred_at")?,
                    recorded_at: row.try_get("recorded_at")?,
                    source_id: row.try_get("source_id")?,
                    frame_id: row.try_get("frame_id")?,
                    status: row.try_get("status")?,
                    content_type: row.try_get("content_type")?,
                    version_number: row.try_get("version_number")?,
                    source_discarded: row.try_get("source_discarded")?,
                    manual: row.try_get("manual")?,
                    run_id: row.try_get("run_id")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ResearchJourney {
            entries,
            truncated,
            recaps,
        })
    }

    /// The researcher's own mainline requests in `[from, until)` as
    /// `(frame_id, first 300 characters of text)`, oldest first. Replayed
    /// epoch copies are skipped like in the history query.
    pub async fn research_recap_requests(
        &self,
        project_id: &str,
        from: i64,
        until: i64,
    ) -> Result<Vec<(String, String)>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.research_recap_requests(project_id, from, until)).await;
        }
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT m.frame_id, m.content FROM messages m JOIN frames f ON f.id=m.frame_id \
             WHERE f.project_id=? AND f.exploration_id IS NULL AND m.role='user' \
               AND trim(COALESCE(m.content,''))<>'' AND m.ts>=? AND m.ts<? \
               AND NOT EXISTS (SELECT 1 FROM context_epochs ce WHERE ce.frame_id=m.frame_id \
                 AND m.seq BETWEEN ce.first_seq AND ce.initial_head_seq) \
             ORDER BY m.ts, m.id LIMIT 200",
        )
        .bind(project_id)
        .bind(from)
        .bind(until)
        .fetch_all(&self.pool)
        .await?;
        // Content is stored as JSON (plain text or multimodal parts).
        Ok(rows
            .into_iter()
            .filter_map(|(frame, json)| {
                let text = serde_json::from_str::<wisp_llm::Content>(&json)
                    .map(|content| content.as_text())
                    .unwrap_or(json);
                let text = text.trim();
                (!text.is_empty()).then(|| (frame, text.chars().take(300).collect()))
            })
            .collect())
    }

    /// Mainline recaps whose day starts in `[from, until)`, newest first.
    pub async fn research_recaps(
        &self,
        project_id: &str,
        from: i64,
        until: i64,
    ) -> Result<Vec<ResearchRecap>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.research_recaps(project_id, from, until)).await;
        }
        sqlx::query("SELECT id,day_start,status,recap_json FROM research_recaps WHERE project_id=? AND day_start>=? AND day_start<? ORDER BY day_start DESC")
            .bind(project_id)
            .bind(from)
            .bind(until)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(recap_from_row)
            .collect()
    }

    /// Store a generated recap for its day. Without `replace`, any existing
    /// recap for that day wins, including a dismissed one, and `None` is
    /// returned, so scheduled drafting never overwrites the researcher.
    pub async fn save_research_recap(
        &self,
        project_id: &str,
        recap: &ResearchRecap,
        replace: bool,
    ) -> Result<Option<ResearchRecap>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.save_research_recap(project_id, recap, replace)).await;
        }
        validate_recap(recap)?;
        let mut tx = self.begin_write().await?;
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM research_recaps WHERE project_id=? AND day_start=?")
                .bind(project_id)
                .bind(recap.day_start)
                .fetch_optional(&mut *tx)
                .await?;
        if existing.is_some() && !replace {
            return Ok(None);
        }
        let mut saved = recap.clone();
        saved.id = existing
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let now = chrono::Utc::now().timestamp();
        let json = serde_json::to_string(&saved)?;
        if existing.is_some() {
            sqlx::query("UPDATE research_recaps SET status=?,recap_json=?,updated_at=? WHERE id=?")
                .bind(&saved.status)
                .bind(&json)
                .bind(now)
                .bind(&saved.id)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("INSERT INTO research_recaps(id,project_id,day_start,status,recap_json,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
                .bind(&saved.id).bind(project_id).bind(saved.day_start).bind(&saved.status).bind(&json).bind(now).bind(now)
                .execute(&mut *tx).await?;
        }
        self.bump_state_generation_in_tx(&mut tx, &StateScope::mainline(project_id))
            .await?;
        tx.commit().await?;
        Ok(Some(saved))
    }

    /// Apply the researcher's review. Sources stay as generated.
    pub async fn update_research_recap(
        &self,
        project_id: &str,
        edit: &ResearchRecapEdit,
    ) -> Result<ResearchRecap> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.update_research_recap(project_id, edit)).await;
        }
        let mut tx = self.begin_write().await?;
        let row = sqlx::query(
            "SELECT id,day_start,status,recap_json FROM research_recaps WHERE id=? AND project_id=?",
        )
        .bind(&edit.id)
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            bail!("This recap no longer exists");
        };
        let mut recap = recap_from_row(row)?;
        recap.status = edit.status.clone();
        recap.headline = edit.headline.trim().to_string();
        recap.done = keep_refs(&edit.done, &recap.done);
        recap.findings = keep_refs(&edit.findings, &recap.findings);
        recap.issues = keep_refs(&edit.issues, &recap.issues);
        recap.next = keep_refs(&edit.next, &recap.next);
        validate_recap(&recap)?;
        sqlx::query("UPDATE research_recaps SET status=?,recap_json=?,updated_at=? WHERE id=?")
            .bind(&recap.status)
            .bind(serde_json::to_string(&recap)?)
            .bind(chrono::Utc::now().timestamp())
            .bind(&recap.id)
            .execute(&mut *tx)
            .await?;
        self.bump_state_generation_in_tx(&mut tx, &StateScope::mainline(project_id))
            .await?;
        tx.commit().await?;
        Ok(recap)
    }

    pub async fn add_research_journal_entry(
        &self,
        scope: &StateScope,
        input: &ResearchJournalInput,
    ) -> Result<String> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.add_research_journal_entry(scope, input)).await;
        }
        scope.validate()?;
        if input.title.trim().is_empty()
            || input.title.chars().count() > 200
            || input.body.chars().count() > 10000
        {
            bail!("A title of 1–200 characters and body of at most 10000 characters are required");
        }
        if !["progress", "finding", "decision", "next"].contains(&input.category.as_str()) {
            bail!("Unknown research journal category");
        }
        if chrono::DateTime::from_timestamp(input.occurred_at, 0).is_none() || input.occurred_at < 0
        {
            bail!("Invalid research journal date");
        }
        let exploration = match scope {
            StateScope::Mainline { .. } => None,
            StateScope::Exploration { exploration_id, .. } => Some(exploration_id.as_str()),
        };
        if let Some(id) = exploration {
            let project: Option<String> = sqlx::query_scalar("SELECT c.project_id FROM explorations x JOIN exploration_checkpoints c ON c.id=x.checkpoint_id WHERE x.id=?").bind(id).fetch_optional(&self.pool).await?;
            if project.as_deref() != Some(scope.project_id()) {
                bail!("Exploration does not belong to project");
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut tx = self.begin_write().await?;
        sqlx::query("INSERT INTO research_journal_entries(id,project_id,exploration_id,title,body,category,occurred_at,created_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(&id).bind(scope.project_id()).bind(exploration).bind(input.title.trim()).bind(input.body.trim()).bind(&input.category)
            .bind(input.occurred_at).bind(chrono::Utc::now().timestamp()).execute(&mut *tx).await?;
        self.bump_state_generation_in_tx(&mut tx, scope).await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn research_journey_source(
        &self,
        scope: &StateScope,
        version_id: &str,
    ) -> Result<ResearchJourneySource> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.research_journey_source(scope, version_id)).await;
        }
        scope.validate()?;
        let exploration = match scope {
            StateScope::Mainline { .. } => None,
            StateScope::Exploration { exploration_id, .. } => Some(exploration_id.as_str()),
        };
        let row = sqlx::query("SELECT v.producing_run_id FROM artifact_versions v JOIN artifacts a ON a.id=v.artifact_id WHERE v.id=?1 AND a.project_id=?2 AND ((?3 IS NULL AND a.exploration_id IS NULL) OR a.exploration_id=?3 OR (a.exploration_id IS NULL AND EXISTS(SELECT 1 FROM explorations x JOIN exploration_baseline_artifact_heads b ON b.checkpoint_id=x.checkpoint_id WHERE x.id=?3 AND b.artifact_version_id=v.id)))")
            .bind(version_id).bind(scope.project_id()).bind(exploration).fetch_optional(&self.pool).await?;
        let Some(row) = row else {
            bail!("Artifact version is not visible in this project scope");
        };
        let run_id: Option<String> = row.try_get("producing_run_id")?;
        let Some(run_id) = run_id else {
            return Ok(ResearchJourneySource::default());
        };
        if !self.run_visible_in_scope(&run_id, scope).await? {
            return Ok(ResearchJourneySource::default());
        }
        let run = self
            .get_run(&run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Run is unavailable"))?;
        let inputs = self
            .list_run_inputs(&run_id)
            .await?
            .into_iter()
            .map(|input| ResearchJourneyInput {
                title: input.source_ref,
                role: input.role,
                version_id: input.artifact_version_id,
                confidence: input.confidence.as_str().to_string(),
            })
            .collect();
        Ok(ResearchJourneySource {
            run_id: Some(run_id),
            run_title: run.title,
            run_status: run.status.as_str().into(),
            context_id: run.context_id,
            generated_at: run.ended_at,
            inputs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArtifactVersionDraft, RunRecord};

    #[tokio::test]
    async fn calendar_reads_requested_mainlines_and_keeps_project_errors_explicit() {
        let path = std::env::temp_dir().join(format!("calendar-{}.db", uuid::Uuid::new_v4()));
        let store = Store::open(&path).await.unwrap();
        for id in ["a", "b", "hidden"] {
            store.create_project(id, id, "").await.unwrap();
            store
                .add_research_journal_entry(
                    &StateScope::mainline(id),
                    &ResearchJournalInput {
                        title: format!("{id} finding"),
                        body: "Evidence".into(),
                        category: "finding".into(),
                        occurred_at: 100,
                    },
                )
                .await
                .unwrap();
        }
        let ids = vec!["a".into(), "missing".into(), "b".into(), "a".into()];
        let rows = store.research_calendar(&ids, 0, 86400).await.unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0].history,
            store
                .research_journey(&StateScope::mainline("a"), 0, 86400)
                .await
                .unwrap()
        );
        assert_eq!(rows[0].history.entries[0].title, "a finding");
        assert_eq!(rows[2].history.entries[0].title, "b finding");
        assert!(rows[1].error.is_some());
        assert!(rows[1].history.entries.is_empty());
        assert!(!rows.iter().any(|r| r.project_id == "hidden"));
        assert!(store.research_calendar(&ids, 100, 100).await.is_err());
        assert!(store.research_calendar(&[], 0, 33 * 86400).await.is_err());
        assert!(store
            .research_calendar(&[], 0, 86400)
            .await
            .unwrap()
            .is_empty());
        let empty = store
            .research_calendar(&["a".into()], 101, 86400)
            .await
            .unwrap();
        assert!(empty[0].history.entries.is_empty());
        assert!(empty[0].error.is_none());
        store.pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    fn recap(day_start: i64, headline: &str) -> ResearchRecap {
        ResearchRecap {
            day_start,
            status: "draft".into(),
            headline: headline.into(),
            done: vec![ResearchRecapItem {
                text: "Compared normalization methods".into(),
                refs: vec![0],
            }],
            sources: vec![wisp_dto::ResearchRecapSource {
                kind: "run".into(),
                id: "r".into(),
                title: "Compare methods".into(),
            }],
            model: "m".into(),
            generated_at: 5,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn recaps_keep_one_per_day_respect_the_researcher_and_travel_with_exports() {
        let token = uuid::Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("recaps-{token}.db"));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store.create_project("other", "Other", "").await.unwrap();
        let empty_export = std::env::temp_dir().join(format!("recaps-empty-{token}.db"));
        store
            .export_project_database("p", &empty_export)
            .await
            .unwrap();
        let empty_hash = Store::portable_project_database_hash(&empty_export)
            .await
            .unwrap();
        sqlx::query("DROP TABLE research_recaps")
            .execute(&Store::open_snapshot(&empty_export).await.unwrap().pool)
            .await
            .unwrap();
        assert_eq!(
            Store::portable_project_database_hash(&empty_export)
                .await
                .unwrap(),
            empty_hash,
            "an empty recap table must not change published fingerprints"
        );

        let first = store
            .save_research_recap("p", &recap(86400, "First"), false)
            .await
            .unwrap()
            .unwrap();
        // Scheduled drafting never replaces an existing day.
        assert!(store
            .save_research_recap("p", &recap(86400, "Second"), false)
            .await
            .unwrap()
            .is_none());
        let journey = store
            .research_journey(&StateScope::mainline("p"), 86400, 172800)
            .await
            .unwrap();
        assert_eq!(journey.recaps, vec![first.clone()]);
        assert!(journey.entries.is_empty(), "recaps are not history entries");
        assert!(store
            .research_journey(&StateScope::mainline("other"), 86400, 172800)
            .await
            .unwrap()
            .recaps
            .is_empty());

        let edited = store
            .update_research_recap(
                "p",
                &ResearchRecapEdit {
                    id: first.id.clone(),
                    status: "confirmed".into(),
                    headline: " Normalization settled ".into(),
                    done: vec![
                        ResearchRecapItem {
                            text: "Compared normalization methods".into(),
                            refs: vec![],
                        },
                        ResearchRecapItem {
                            text: "Chose method B".into(),
                            refs: vec![0],
                        },
                    ],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(edited.headline, "Normalization settled");
        assert_eq!(edited.done[0].refs, vec![0], "unchanged line keeps sources");
        assert!(
            edited.done[1].refs.is_empty(),
            "rewritten line drops sources"
        );
        assert!(store
            .update_research_recap(
                "other",
                &ResearchRecapEdit {
                    id: first.id.clone(),
                    status: "confirmed".into(),
                    headline: "x".into(),
                    ..Default::default()
                },
            )
            .await
            .is_err());
        assert!(store
            .update_research_recap(
                "p",
                &ResearchRecapEdit {
                    id: first.id.clone(),
                    status: "published".into(),
                    headline: "x".into(),
                    ..Default::default()
                },
            )
            .await
            .is_err());

        // A manual regeneration replaces the day in place.
        let regenerated = store
            .save_research_recap("p", &recap(86400, "Regenerated"), true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(regenerated.id, first.id);
        let mut bad = recap(0, "Bad");
        bad.done[0].refs = vec![3];
        assert!(store.save_research_recap("p", &bad, true).await.is_err());

        let export = std::env::temp_dir().join(format!("recaps-export-{token}.db"));
        store.export_project_database("p", &export).await.unwrap();
        assert_ne!(
            Store::portable_project_database_hash(&export)
                .await
                .unwrap(),
            empty_hash
        );
        let copy = Store::open_snapshot(&export).await.unwrap();
        assert_eq!(
            copy.research_recaps("p", 0, i64::MAX).await.unwrap(),
            vec![regenerated]
        );
        store.delete_project("p").await.unwrap();
        assert!(store
            .research_recaps("p", 0, i64::MAX)
            .await
            .unwrap()
            .is_empty());
        for file in [path, empty_export, export] {
            let _ = std::fs::remove_file(file);
        }
    }

    #[tokio::test]
    async fn journey_records_message_days_and_preserves_stated_decision_rationale() {
        let path =
            std::env::temp_dir().join(format!("journey-messages-{}.db", uuid::Uuid::new_v4()));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store
            .create_frame("f", "p", "agent", "model")
            .await
            .unwrap();
        store
            .append_message(
                "f",
                0,
                &wisp_llm::Message::user("Compare normalization methods"),
            )
            .await
            .unwrap();
        store
            .append_message(
                "f",
                1,
                &wisp_llm::Message::assistant("Unconfirmed observation"),
            )
            .await
            .unwrap();
        sqlx::query("UPDATE messages SET ts=1000")
            .execute(&store.pool)
            .await
            .unwrap();
        let mut decision = crate::ResearchNode::new(
            "decision",
            "p",
            crate::ResearchNodeKind::Decision,
            "Keep baseline",
        )
        .unwrap();
        decision.created_at = 1001;
        decision.metadata_json = r#"{"rationale":"Better fit for this design"}"#.into();
        store.save_research_node(&decision).await.unwrap();
        let entries = store
            .research_journey(&StateScope::mainline("p"), 0, 86400)
            .await
            .unwrap()
            .entries;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].summary, "Better fit for this design");
        assert_eq!(entries[1].kind, "session");
        assert_eq!(entries[1].source_id, "f");
        assert!(entries[1].summary.is_empty());
        assert!(!entries.iter().any(|e| e.title.contains("Unconfirmed")));
        assert_eq!(
            store.research_recap_requests("p", 0, 86400).await.unwrap(),
            vec![("f".to_string(), "Compare normalization methods".to_string())]
        );
        assert!(store
            .research_recap_requests("p", 1001, 86400)
            .await
            .unwrap()
            .is_empty());
        store.pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn journey_preserves_dates_versions_notes_and_project_boundaries() {
        let path = std::env::temp_dir().join(format!("journey-{}.db", uuid::Uuid::new_v4()));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store.create_project("other", "Other", "").await.unwrap();
        store
            .create_frame("f", "p", "agent", "model")
            .await
            .unwrap();
        let scope = StateScope::mainline("p");
        let mut run = RunRecord::new("r", "p", "local", "Compare methods", "command");
        run.created_at = 1000;
        run.started_at = Some(1000);
        run.ended_at = Some(90000);
        run.status = crate::RunStatus::Failed;
        store.create_run(&run).await.unwrap();
        store
            .save_artifact("a", "p", "f", "result.csv", "text/csv", "result.csv")
            .await
            .unwrap();
        let mut draft = ArtifactVersionDraft {
            version_id: None,
            artifact_id: "a".into(),
            project_id: "p".into(),
            root_frame_id: "f".into(),
            filename: "result.csv".into(),
            content_type: "text/csv".into(),
            storage_path: "result.csv".into(),
            logical_key: None,
            size_bytes: None,
            checksum: None,
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: crate::ArtifactMaterialization::Reference,
            capture_timing: crate::ArtifactCaptureTiming::AtCreation,
        };
        draft.producing_run_id = Some("r".into());
        let version = store.save_artifact_version(&draft).await.unwrap();
        sqlx::query("UPDATE artifact_versions SET created_at=90001 WHERE id=?")
            .bind(&version)
            .execute(&store.pool)
            .await
            .unwrap();
        let input = ResearchJournalInput {
            title: "Observed instability".into(),
            body: "Repeat with more samples".into(),
            category: "finding".into(),
            occurred_at: 90002,
        };
        store
            .add_research_journal_entry(&scope, &input)
            .await
            .unwrap();
        store
            .add_research_journal_entry(&StateScope::mainline("other"), &input)
            .await
            .unwrap();
        // Output transfers are plumbing, not research activity.
        let mut harvest = RunRecord::new("h", "p", "ssh:x", "Harvest run outputs", "file_transfer");
        harvest.started_at = Some(1001);
        harvest.ended_at = Some(1002);
        store.create_run(&harvest).await.unwrap();
        let first = store.research_journey(&scope, 0, 86400).await.unwrap();
        assert_eq!(first.entries.len(), 1);
        assert_eq!(first.entries[0].status, "started");
        assert_eq!(first.entries[0].run_id.as_deref(), Some("r"));
        let second = store.research_journey(&scope, 86400, 172800).await.unwrap();
        assert_eq!(second.entries.len(), 3);
        assert!(second.entries.iter().any(|e| e.status == "failed"));
        let artifact = second
            .entries
            .iter()
            .find(|e| e.kind == "artifact")
            .unwrap();
        assert_eq!(artifact.source_id, version);
        assert_eq!(artifact.run_id.as_deref(), Some("r"));
        assert_eq!(artifact.version_number, Some(2));
        assert_eq!(artifact.occurred_at, 90001);
        let note = second.entries.iter().find(|e| e.manual).unwrap();
        assert_eq!(note.summary, "Repeat with more samples");
        assert_ne!(note.recorded_at, note.occurred_at);
        assert_eq!(
            store
                .research_journey_source(&scope, &version)
                .await
                .unwrap()
                .run_id
                .as_deref(),
            Some("r")
        );
        assert!(store
            .research_journey_source(&StateScope::mainline("other"), &version)
            .await
            .is_err());
        assert!(store.research_journey(&scope, 0, 33 * 86400).await.is_err());
        let invalid = ResearchJournalInput {
            category: "anything".into(),
            ..input
        };
        assert!(store
            .add_research_journal_entry(&scope, &invalid)
            .await
            .is_err());
        drop(store);
        let reopened = Store::open(&path).await.unwrap();
        assert_eq!(
            reopened
                .research_journey(&scope, 86400, 172800)
                .await
                .unwrap(),
            second
        );
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }
}
