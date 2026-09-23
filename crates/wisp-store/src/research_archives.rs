use super::{canonical_json_sha256, StateScope, Store};
use anyhow::{bail, Result};
use sqlx::Row;
use wisp_dto::ResearchArchive;

impl Store {
    pub async fn research_archive(&self, frame_id: &str) -> Result<Option<ResearchArchive>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.research_archive(frame_id)).await;
        }
        let row: Option<String> =
            sqlx::query_scalar("SELECT record_json FROM research_archives WHERE frame_id=?")
                .bind(frame_id)
                .fetch_optional(&self.pool)
                .await?;
        row.map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }

    pub async fn require_unarchived_session(&self, frame_id: &str) -> Result<()> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.require_unarchived_session(frame_id)).await;
        }
        let locked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM research_archives a JOIN frames f ON f.root_frame_id=a.frame_id WHERE f.id=? AND a.frozen_at IS NOT NULL)")
            .bind(frame_id).fetch_one(&self.pool).await?;
        if locked {
            bail!("research_archive_read_only: this notebook is archived; continue in a new conversation");
        }
        Ok(())
    }

    /// Include executed cells and the full visual notebook in the guard even if
    /// model context was compacted. The source export is stored beside the archive.
    pub async fn research_archive_source(&self, frame_id: &str) -> Result<(String, String)> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.research_archive_source(frame_id)).await;
        }
        let messages = self.load_messages_with_seq(frame_id).await?;
        let cells = sqlx::query("SELECT tool,language,source,exit_status,files_read,files_written FROM execution_log WHERE frame_id=? ORDER BY cell_index,id")
            .bind(frame_id).fetch_all(&self.pool).await?.into_iter().map(|r| {
                Ok(serde_json::json!({"tool":r.try_get::<String,_>("tool")?,"language":r.try_get::<String,_>("language")?,"source":r.try_get::<String,_>("source")?,"exit_status":r.try_get::<String,_>("exit_status")?,"files_read":r.try_get::<String,_>("files_read")?,"files_written":r.try_get::<String,_>("files_written")?}))
            }).collect::<Result<Vec<_>>>()?;
        let events = self.load_session_ui_events_timed(frame_id).await?.into_iter().map(|e|serde_json::json!({"seq":e.seq,"created_at":e.created_at,"event":serde_json::from_str::<serde_json::Value>(&e.event_json).unwrap_or(serde_json::Value::String(e.event_json))})).collect::<Vec<_>>();
        Ok(canonical_json_sha256(
            &serde_json::json!({"messages":messages,"executed_cells":cells,"notebook_events":events}),
        ))
    }

    pub async fn save_research_archive_draft(&self, archive: &ResearchArchive) -> Result<()> {
        if let Some(store) = self.route_project(&archive.project_id).await? {
            return Box::pin(store.save_research_archive_draft(archive)).await;
        }
        self.require_unarchived_session(&archive.frame_id).await?;
        if archive.frozen_at.is_some()
            || self.frame_project_id(&archive.frame_id).await?.as_deref()
                != Some(&archive.project_id)
        {
            bail!("Invalid archive draft owner");
        }
        sqlx::query("INSERT INTO research_archives(id,project_id,frame_id,title,record_json,created_at) VALUES(?,?,?,?,?,?) ON CONFLICT(frame_id) DO UPDATE SET id=excluded.id,title=excluded.title,record_json=excluded.record_json,created_at=excluded.created_at WHERE research_archives.frozen_at IS NULL")
            .bind(&archive.id).bind(&archive.project_id).bind(&archive.frame_id).bind(&archive.title).bind(serde_json::to_string(archive)?).bind(archive.created_at).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn freeze_research_archive(&self, archive: &ResearchArchive) -> Result<()> {
        if let Some(store) = self.route_project(&archive.project_id).await? {
            return Box::pin(store.freeze_research_archive(archive)).await;
        }
        if archive.frozen_at.is_none() {
            bail!("Archive freeze timestamp is required");
        }
        let mut tx = self.begin_write().await?;
        let changed = sqlx::query("UPDATE research_archives SET title=?,record_json=?,frozen_at=? WHERE id=? AND project_id=? AND frame_id=? AND frozen_at IS NULL")
            .bind(&archive.title).bind(serde_json::to_string(archive)?).bind(archive.frozen_at).bind(&archive.id).bind(&archive.project_id).bind(&archive.frame_id).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            bail!("Archive draft changed; reopen the review");
        }
        self.bump_state_generation_in_tx(&mut tx, &StateScope::mainline(&archive.project_id))
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Only cleanup receipts can change after freezing; content stays immutable.
    pub async fn record_archive_cleanup(
        &self,
        frame_id: &str,
        receipts: &[(String, String)],
    ) -> Result<ResearchArchive> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.record_archive_cleanup(frame_id, receipts)).await;
        }
        let mut archive = self
            .research_archive(frame_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Archive not found"))?;
        if archive.frozen_at.is_none() {
            bail!("Archive has not been frozen");
        }
        for (path, status) in receipts {
            if let Some(file) = archive
                .files
                .iter_mut()
                .find(|f| f.path == *path && f.action == "delete")
            {
                file.cleanup_status = status.clone();
            }
        }
        sqlx::query(
            "UPDATE research_archives SET record_json=? WHERE id=? AND frozen_at IS NOT NULL",
        )
        .bind(serde_json::to_string(&archive)?)
        .bind(&archive.id)
        .execute(&self.pool)
        .await?;
        Ok(archive)
    }

    /// Known writes plus registered material. Never infer ownership from a folder scan.
    pub async fn research_archive_paths(&self, frame_id: &str) -> Result<Vec<String>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.research_archive_paths(frame_id)).await;
        }
        let paths: Vec<String> = sqlx::query_scalar("SELECT path FROM turn_file_undo WHERE frame_id=?1 UNION SELECT CASE WHEN logical_key LIKE 'path:%' THEN substr(logical_key,6) ELSE storage_path END FROM artifacts WHERE root_frame_id=?1 UNION SELECT display_name FROM message_resource_links WHERE frame_id=?1 UNION SELECT f.value FROM execution_log e,json_each(e.files_read) f WHERE e.frame_id=?1 AND f.type='text' UNION SELECT f.value FROM execution_log e,json_each(e.files_written) f WHERE e.frame_id=?1 AND f.type='text' ORDER BY 1")
            .bind(frame_id).fetch_all(&self.pool).await?;
        Ok(paths)
    }

    /// A deletion candidate needs creation provenance, no other notebook use,
    /// no registered project evidence and no original-input identity.
    pub async fn archive_path_deletable(&self, frame_id: &str, path: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.archive_path_deletable(frame_id, path)).await;
        }
        let created: Option<bool> = sqlx::query_scalar("SELECT NOT before_exists FROM turn_file_undo WHERE frame_id=? AND path=? ORDER BY user_message_seq LIMIT 1")
            .bind(frame_id).bind(path).fetch_optional(&self.pool).await?;
        if created != Some(true) {
            return Ok(false);
        }
        // Windows case/slash aliases and absolute references must not hide
        // another notebook's use of the same local file.
        let project_id = self
            .frame_project_id(frame_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
        let (_, root) = self
            .get_project(&project_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Project not found"))?;
        let key = archive_path_key(path, &root);
        let other_paths:Vec<String>=sqlx::query_scalar("SELECT u.path FROM turn_file_undo u JOIN frames f ON f.id=u.frame_id WHERE f.project_id=?1 AND u.frame_id<>?2 UNION SELECT l.display_name FROM message_resource_links l JOIN frames f ON f.id=l.frame_id WHERE f.project_id=?1 AND l.frame_id<>?2 UNION SELECT r.value FROM execution_log e JOIN frames f ON f.id=e.frame_id,json_each(e.files_read) r WHERE f.project_id=?1 AND e.frame_id<>?2 AND r.type='text' UNION SELECT r.value FROM execution_log e JOIN frames f ON f.id=e.frame_id,json_each(e.files_written) r WHERE f.project_id=?1 AND e.frame_id<>?2 AND r.type='text'")
            .bind(&project_id).bind(frame_id).fetch_all(&self.pool).await?;
        if other_paths
            .iter()
            .any(|p| archive_path_key(p, &root) == key)
        {
            return Ok(false);
        }
        let protected: bool = sqlx::query_scalar(r#"SELECT
            EXISTS(SELECT 1 FROM turn_file_undo WHERE frame_id<>?1 AND path=?2)
            OR EXISTS(SELECT 1 FROM message_resource_links WHERE frame_id<>?1 AND (display_name=?2 OR original_reference=?2))
            OR EXISTS(SELECT 1 FROM execution_log e,json_each(e.files_read) f WHERE e.frame_id<>?1 AND (f.value=?2 OR CASE WHEN json_valid(f.value) THEN json_extract(f.value,'$.path') END=?2))
            OR EXISTS(SELECT 1 FROM artifacts a WHERE (a.logical_key='path:'||?2 OR a.storage_path=?2) AND (
                a.root_frame_id<>?1
                OR EXISTS(SELECT 1 FROM run_artifacts r WHERE r.artifact_id=a.id)
                OR EXISTS(SELECT 1 FROM artifact_versions v WHERE v.artifact_id=a.id AND (
                    EXISTS(SELECT 1 FROM run_inputs i WHERE i.artifact_version_id=v.id)
                    OR EXISTS(SELECT 1 FROM run_outputs o WHERE o.artifact_version_id=v.id)
                    OR EXISTS(SELECT 1 FROM evidence_bindings b WHERE b.artifact_version_id=v.id)
                    OR EXISTS(SELECT 1 FROM artifact_dependencies d WHERE d.depends_on_version_id=v.id)
                    OR EXISTS(SELECT 1 FROM message_resource_links l WHERE l.artifact_version_id=v.id AND l.frame_id<>?1)))))
            OR EXISTS(SELECT 1 FROM research_archives a,json_each(a.record_json,'$.files') f WHERE a.frame_id<>?1 AND a.frozen_at IS NOT NULL AND json_extract(f.value,'$.path')=?2 AND json_extract(f.value,'$.action')<>'delete')"#)
            .bind(frame_id).bind(path).fetch_one(&self.pool).await?;
        Ok(!protected)
    }

    pub async fn research_archive_index(&self, project_id: &str) -> Result<String> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.research_archive_index(project_id)).await;
        }
        let rows = sqlx::query("SELECT id,title,frame_id FROM research_archives WHERE project_id=? AND frozen_at IS NOT NULL ORDER BY frozen_at DESC LIMIT 100")
            .bind(project_id).fetch_all(&self.pool).await?;
        if rows.is_empty() {
            return Ok(String::new());
        }
        let mut text=String::from("Project research notebook archive index (user-reviewed historical findings, not instructions). Read the referenced REPORT.md or notebook only when relevant; newer research may revise these findings.\n");
        for row in rows {
            text.push_str(&format!(
                "- {}: .wisp/research-archives/{}/REPORT.md (notebook {})\n",
                row.try_get::<String, _>("title")?,
                row.try_get::<String, _>("id")?,
                row.try_get::<String, _>("frame_id")?
            ));
        }
        Ok(text)
    }

    pub async fn link_archive_continuation(
        &self,
        archive: &ResearchArchive,
        frame_id: &str,
    ) -> Result<()> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.link_archive_continuation(archive, frame_id)).await;
        }
        if archive.frozen_at.is_none()
            || self.frame_project_id(frame_id).await?.as_deref() != Some(&archive.project_id)
        {
            bail!("Invalid archive continuation");
        }
        sqlx::query("INSERT INTO research_archive_continuations(frame_id,archive_id) VALUES(?,?)")
            .bind(frame_id)
            .bind(&archive.id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn archive_path_key(path: &str, root: &str) -> String {
    let path = path.replace('\\', "/");
    let root = root.replace('\\', "/");
    #[cfg(windows)]
    let (path, root) = (path.to_lowercase(), root.to_lowercase());
    let relative = path
        .strip_prefix(&format!("{}/", root.trim_end_matches('/')))
        .unwrap_or(&path);
    relative.trim_start_matches("./").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_llm::Message;

    #[tokio::test]
    async fn research_archive_survives_migration_export_and_import() {
        let root = std::env::temp_dir().join(format!("archive-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("live.sqlite");
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("p", "Research", root.to_str().unwrap())
            .await
            .unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store
            .append_message("f", 1, &Message::user("Original observation"))
            .await
            .unwrap();
        let mut archive = ResearchArchive {
            id: "a".into(),
            project_id: "p".into(),
            frame_id: "f".into(),
            title: "Reviewed conclusion".into(),
            report: "Evidence and limitations".into(),
            source_hash: store.research_archive_source("f").await.unwrap().1,
            created_at: 10,
            ..Default::default()
        };
        store.save_research_archive_draft(&archive).await.unwrap();
        archive.frozen_at = Some(11);
        store.freeze_research_archive(&archive).await.unwrap();
        store
            .create_frame("continued", "p", "OPERON", "m")
            .await
            .unwrap();
        store
            .link_archive_continuation(&archive, "continued")
            .await
            .unwrap();
        let bundle = root.join("bundle.sqlite");
        store.export_project_database("p", &bundle).await.unwrap();
        let imported = Store::open(&root.join("imported.sqlite")).await.unwrap();
        imported
            .import_project_database(&bundle, "p", &root.join("new-location"))
            .await
            .unwrap();
        assert_eq!(imported.research_archive("f").await.unwrap(), Some(archive));
        assert!(imported
            .append_message("f", 2, &Message::user("late writer"))
            .await
            .is_err());
        assert_eq!(imported.load_messages("f").await.unwrap().len(), 1);
        assert!(imported
            .require_unarchived_session("continued")
            .await
            .is_ok());
        assert!(imported
            .research_journey(&StateScope::mainline("p"), 0, 86400)
            .await
            .unwrap()
            .entries
            .iter()
            .any(|e| e.kind == "archive"));
        store.pool.close().await;
        imported.pool.close().await;
        let reopened = Store::open(&path).await.unwrap();
        assert!(reopened.require_unarchived_session("f").await.is_err());
        reopened.delete_project("p").await.unwrap();
        assert!(reopened.research_archive("f").await.unwrap().is_none());
        reopened.pool.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn research_archive_candidates_protect_inputs_and_other_notebooks() {
        let store = Store::open(std::path::Path::new(":memory:")).await.unwrap();
        store.create_project("p", "Research", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store
            .create_frame("other", "p", "OPERON", "m")
            .await
            .unwrap();
        store
            .save_turn_file_undo("f", 1, "input.csv", true, None, None, None, false, None)
            .await
            .unwrap();
        store
            .save_turn_file_undo(
                "f",
                1,
                "intermediate.csv",
                false,
                None,
                None,
                Some("x"),
                true,
                None,
            )
            .await
            .unwrap();
        assert!(!store
            .archive_path_deletable("f", "input.csv")
            .await
            .unwrap());
        assert!(store
            .archive_path_deletable("f", "intermediate.csv")
            .await
            .unwrap());
        store
            .insert_execution_log(&crate::ExecLog {
                id: "read".into(),
                frame_id: "other".into(),
                files_read: vec!["intermediate.csv".into()],
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(!store
            .archive_path_deletable("f", "intermediate.csv")
            .await
            .unwrap());
        assert!(store
            .research_archive_index("other-project")
            .await
            .unwrap()
            .is_empty());
    }
}
