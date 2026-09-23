use super::{StateScope, Store};
use anyhow::{bail, Result};
use sqlx::Row;
use wisp_dto::{
    ResearchJournalInput, ResearchJourney, ResearchJourneyEntry, ResearchJourneyInput,
    ResearchJourneySource,
};

const ENTRY_LIMIT: usize = 2000;

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
                NULL AS version_number, 0 AS source_discarded, 0 AS manual
              FROM runs r WHERE {runs}
              UNION ALL
              SELECT 'run-end:'||r.id, 'run', r.title, '', r.ended_at, r.ended_at,
                r.id, r.frame_id, r.status, '', NULL, 0, 0 FROM runs r WHERE {runs} AND r.ended_at IS NOT NULL
              UNION ALL
              SELECT 'version:'||v.id, 'artifact', a.filename, '', v.created_at, v.created_at,
                v.id, a.root_frame_id, 'registered', v.content_type, v.version_number,
                v.source_discarded_at IS NOT NULL, 0
              FROM artifact_versions v JOIN artifacts a ON a.id=v.artifact_id WHERE {artifacts}
              UNION ALL
              SELECT 'node:'||n.id, n.kind, n.title,
                CAST(COALESCE(json_extract(n.metadata_json,'$.body'),json_extract(n.metadata_json,'$.reason'),json_extract(n.metadata_json,'$.rationale'),json_extract(n.metadata_json,'$.summary'),'') AS TEXT),
                n.created_at, n.created_at, n.id, NULL, 'recorded', '', NULL, 0, 0
              FROM research_nodes n WHERE {nodes} AND n.kind IN ('decision','paper','data_asset')
              UNION ALL
              SELECT 'message:'||m.id, 'session', COALESCE(NULLIF(f.title,''),substr(m.content,1,120),'Conversation'),
                '', m.ts, m.ts, f.id, f.id, 'discussed', '', NULL, 0, 0
              FROM messages m JOIN frames f ON f.id=m.frame_id
              WHERE f.project_id=?1 AND ((?2 IS NULL AND f.exploration_id IS NULL) OR f.exploration_id=?2)
                AND m.role='user' AND trim(COALESCE(m.content,''))<>''
                AND NOT EXISTS (SELECT 1 FROM context_epochs ce WHERE ce.frame_id=m.frame_id
                    AND m.seq BETWEEN ce.first_seq AND ce.initial_head_seq)
              UNION ALL
              SELECT 'journal:'||j.id, j.category, j.title, j.body, j.occurred_at, j.created_at,
                j.id, NULL, 'recorded', '', NULL, 0, 1 FROM research_journal_entries j
              WHERE j.project_id=?1 AND ((?2 IS NULL AND j.exploration_id IS NULL) OR j.exploration_id=?2
                OR (j.exploration_id IS NULL AND EXISTS(SELECT 1 FROM explorations x
                    JOIN exploration_baseline_entities b ON b.checkpoint_id=x.checkpoint_id WHERE x.id=?2 AND b.entity_kind='research_journal_entry' AND b.entity_id=j.id)))
              UNION ALL
              SELECT 'archive:'||a.id, 'archive', a.title, json_extract(a.record_json,'$.report'), a.frozen_at, a.created_at,
                a.id, a.frame_id, 'archived', '', NULL, 0, 0 FROM research_archives a
              WHERE a.project_id=?1 AND ?2 IS NULL AND a.frozen_at IS NOT NULL
              UNION ALL
              SELECT 'archive-continuation:'||c.frame_id, 'progress', 'Continue research / 继续研究', a.title,
                f.created_at,f.created_at,a.id,c.frame_id,'continued','',NULL,0,0
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
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ResearchJourney { entries, truncated })
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
        let first = store.research_journey(&scope, 0, 86400).await.unwrap();
        assert_eq!(first.entries.len(), 1);
        assert_eq!(first.entries[0].status, "started");
        let second = store.research_journey(&scope, 86400, 172800).await.unwrap();
        assert_eq!(second.entries.len(), 3);
        assert!(second.entries.iter().any(|e| e.status == "failed"));
        let artifact = second
            .entries
            .iter()
            .find(|e| e.kind == "artifact")
            .unwrap();
        assert_eq!(artifact.source_id, version);
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
