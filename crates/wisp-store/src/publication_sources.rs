//! Bounded source discovery for the Publication editor, independent of freeze.
use crate::Store;
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use sqlx::Row;
use wisp_dto::{PublicationSourceChoice, PublicationSourcePage};

impl Store {
    pub async fn publication_source_page(
        &self,
        project_id: &str,
        kind: &str,
        query: &str,
        offset: u32,
    ) -> Result<PublicationSourcePage> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.publication_source_page(project_id, kind, query, offset)).await;
        }
        let query = query.trim().to_lowercase();
        let sql = match kind {
            "files" => {
                "SELECT v.id,a.filename AS title,CAST(v.version_number AS TEXT) AS detail \
                FROM artifacts a JOIN artifact_versions v ON v.id=a.latest_version_id \
                WHERE a.project_id=? AND a.exploration_id IS NULL \
                AND instr(lower(a.filename),?)>0 ORDER BY a.created_at DESC,a.id LIMIT 51 OFFSET ?"
            }
            "runs" => {
                "SELECT id,title,status AS detail FROM runs \
                WHERE project_id=? AND exploration_id IS NULL AND instr(lower(title),?)>0 \
                ORDER BY created_at DESC,id LIMIT 51 OFFSET ?"
            }
            "messages" => {
                "SELECT m.id,COALESCE(f.title,f.agent_name) AS title,m.role AS detail,\
                m.content,m.frame_id,m.seq FROM messages m JOIN frames f ON f.id=m.frame_id \
                WHERE f.project_id=? AND f.exploration_id IS NULL AND f.status<>'deleted' \
                AND m.seq>0 AND m.role IN ('user','assistant') \
                AND NOT EXISTS (SELECT 1 FROM context_epochs ce WHERE ce.frame_id=m.frame_id \
                    AND m.seq BETWEEN ce.first_seq AND ce.initial_head_seq) \
                AND length(m.content)<=262144 \
                AND (instr(lower(COALESCE(f.title,'')),?)>0 OR instr(lower(m.content),?)>0) \
                ORDER BY m.ts DESC,m.id LIMIT 51 OFFSET ?"
            }
            _ => bail!("Unsupported publication source category"),
        };
        let mut request = sqlx::query(sql).bind(project_id).bind(&query);
        if kind == "messages" {
            request = request.bind(&query);
        }
        let rows = request
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await?;
        let has_more = rows.len() > 50;
        let mut sources = Vec::new();
        for row in rows.into_iter().take(50) {
            let mut choice = PublicationSourceChoice {
                kind: match kind {
                    "files" => "artifact_version",
                    "runs" => "run",
                    _ => "message_span",
                }
                .into(),
                id: row.try_get("id")?,
                title: row.try_get("title")?,
                detail: row.try_get("detail")?,
                text: None,
                text_sha256: None,
                frame_id: None,
                message_seq: None,
            };
            if kind == "messages" {
                let encoded: String = row.try_get("content")?;
                let Ok(content) = serde_json::from_str::<wisp_llm::Content>(&encoded) else {
                    continue;
                };
                let text = content.as_text();
                let mut end = text.len().min(16_384);
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                if text[..end].trim().is_empty() {
                    continue;
                }
                choice.text = Some(text[..end].into());
                choice.text_sha256 = Some(hex::encode(Sha256::digest(text.as_bytes())));
                choice.frame_id = Some(row.try_get("frame_id")?);
                choice.message_seq = Some(row.try_get("seq")?);
            }
            sources.push(choice);
        }
        Ok(PublicationSourcePage { sources, has_more })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn file_choices_remain_exact_and_run_pages_do_not_leak_other_projects() {
        let store = Store::open(std::path::Path::new(":memory:")).await.unwrap();
        for id in ["p1", "p2"] {
            store
                .create_project(id, id, &format!("/{id}"))
                .await
                .unwrap();
            store.create_frame(id, id, "main", "model").await.unwrap();
            store
                .save_artifact(
                    &format!("a-{id}"),
                    id,
                    id,
                    "figure.png",
                    "image/png",
                    "figure.png",
                )
                .await
                .unwrap();
        }
        let first = store
            .publication_source_page("p1", "files", "figure", 0)
            .await
            .unwrap();
        assert_eq!(first.sources.len(), 1);
        assert_eq!(first.sources[0].kind, "artifact_version");
        store
            .save_artifact(
                "a-p1",
                "p1",
                "p1",
                "figure.png",
                "image/png",
                "new-figure.png",
            )
            .await
            .unwrap();
        let second = store
            .publication_source_page("p1", "files", "", 0)
            .await
            .unwrap();
        assert_ne!(first.sources[0].id, second.sources[0].id);
        assert!(store
            .get_artifact_version(&first.sources[0].id)
            .await
            .unwrap()
            .is_some());
        for index in 0..51 {
            let run = crate::RunRecord::new(
                format!("run-{index}"),
                "p1",
                "local",
                format!("Run {index}"),
                "command",
            );
            store.create_run(&run).await.unwrap();
        }
        store
            .create_run(&crate::RunRecord::new(
                "foreign", "p2", "local", "Foreign", "command",
            ))
            .await
            .unwrap();
        let page = store
            .publication_source_page("p1", "runs", "", 0)
            .await
            .unwrap();
        assert_eq!(page.sources.len(), 50);
        assert!(page.has_more);
        let next = store
            .publication_source_page("p1", "runs", "", 50)
            .await
            .unwrap();
        assert_eq!(next.sources.len(), 1);
        assert!(!next.has_more);
        assert!(!page
            .sources
            .iter()
            .any(|s| s.id == next.sources[0].id || s.id == "foreign"));
    }

    #[tokio::test]
    async fn source_picker_is_project_scoped_and_preserves_unicode_message_text() {
        let store = Store::open(std::path::Path::new(":memory:")).await.unwrap();
        for id in ["p1", "p2"] {
            sqlx::query("INSERT INTO projects(id,name,workspace_dir,created_at,updated_at) VALUES(?,?,?,0,0)")
                .bind(id).bind(id).bind(format!("/{id}")).execute(&store.pool).await.unwrap();
            sqlx::query("INSERT INTO frames(id,agent_name,status,project_id,created_at,updated_at) VALUES(?,'main','idle',?,0,0)")
                .bind(id).bind(id).execute(&store.pool).await.unwrap();
            sqlx::query("INSERT INTO messages(id,frame_id,seq,role,content,ts) VALUES(?,?,1,'assistant',?,0)")
                .bind(id).bind(id).bind(serde_json::to_string(&wisp_llm::Content::text("采用人工校正的轮廓 🌱")).unwrap())
                .execute(&store.pool).await.unwrap();
        }
        let page = store
            .publication_source_page("p1", "messages", "", 0)
            .await
            .unwrap();
        assert_eq!(page.sources.len(), 1);
        assert_eq!(page.sources[0].frame_id.as_deref(), Some("p1"));
        assert_eq!(
            page.sources[0].text.as_deref(),
            Some("采用人工校正的轮廓 🌱")
        );
        assert!(!page.has_more);
        store
            .create_publication("paper", "p1", "Paper", "")
            .await
            .unwrap();
        store
            .create_publication_revision("revision", "paper", None, "Draft")
            .await
            .unwrap();
        let choice = &page.sources[0];
        let mut binding = crate::EvidenceBindingDraft {
            id: "binding".into(),
            revision_id: "revision".into(),
            item_id: None,
            source_kind: crate::EvidenceSourceKind::MessageSpan,
            source_id: crate::canonical_json(&serde_json::json!({
                "frame_id":"p1", "message_seq":1, "byte_start":0,
                "byte_end":choice.text.as_ref().unwrap().len(),
                "message_content_sha256":choice.text_sha256,
            })),
            purpose: "Review rationale".into(),
            supported_claim_item_id: None,
            selection_state: crate::EvidenceSelectionState::Selected,
            visibility: crate::EvidenceVisibility::Private,
        };
        store.save_evidence_binding(&binding).await.unwrap();
        sqlx::query("UPDATE messages SET content=? WHERE frame_id='p1'")
            .bind(serde_json::to_string(&wisp_llm::Content::text("changed message")).unwrap())
            .execute(&store.pool)
            .await
            .unwrap();
        binding.id = "stale-binding".into();
        assert!(store
            .save_evidence_binding(&binding)
            .await
            .unwrap_err()
            .to_string()
            .contains("Message changed after preview"));
        assert!(store
            .publication_source_page("p1", "messages", "不存在", 0)
            .await
            .unwrap()
            .sources
            .is_empty());
        sqlx::query("UPDATE frames SET exploration_id='branch' WHERE id='p1'")
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(store
            .publication_source_page("p1", "messages", "", 0)
            .await
            .unwrap()
            .sources
            .is_empty());
        assert!(store
            .publication_source_page("p1", "unsupported", "", 0)
            .await
            .is_err());
    }
}
