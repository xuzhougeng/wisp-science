use crate::{CommitOutcome, CommitRequest, SyncHead, SyncRevision, SYNC_PROTOCOL_VERSION};
use anyhow::{Context, Result};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

#[async_trait]
pub trait SyncTransport: Send + Sync {
    async fn head(&self, project_id: &str) -> Result<Option<SyncHead>>;
    async fn revision(&self, project_id: &str, revision_id: &str) -> Result<SyncRevision>;
    async fn blob_exists(&self, blob_id: &str) -> Result<bool>;
    async fn get_blob(&self, blob_id: &str) -> Result<Vec<u8>>;
    async fn put_blob(&self, blob_id: &str, bytes: Vec<u8>) -> Result<()>;
    async fn commit(&self, project_id: &str, request: CommitRequest) -> Result<CommitOutcome>;
}

#[derive(Clone)]
pub struct FileRelay {
    root: PathBuf,
    commit_lock: Arc<Mutex<()>>,
}

impl FileRelay {
    pub async fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        tokio::fs::create_dir_all(root.join("projects")).await?;
        tokio::fs::create_dir_all(root.join("blobs")).await?;
        Ok(Self {
            root,
            commit_lock: Arc::new(Mutex::new(())),
        })
    }

    fn validate_component(value: &str, name: &str) -> Result<()> {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        {
            anyhow::bail!("invalid {name}");
        }
        Ok(())
    }

    fn validate_blob_id(value: &str) -> Result<()> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            anyhow::bail!("invalid blob id");
        }
        Ok(())
    }

    fn project_dir(&self, project_id: &str) -> Result<PathBuf> {
        Self::validate_component(project_id, "project id")?;
        Ok(self.root.join("projects").join(project_id))
    }

    fn blob_path(&self, blob_id: &str) -> Result<PathBuf> {
        Self::validate_blob_id(blob_id)?;
        Ok(self.root.join("blobs").join(blob_id))
    }

    async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
        let parent = path.parent().context("relay path has no parent")?;
        tokio::fs::create_dir_all(parent).await?;
        let tmp = parent.join(format!(
            ".{}.{}.tmp",
            path.file_name().unwrap_or_default().to_string_lossy(),
            uuid::Uuid::new_v4()
        ));
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        if let Err(error) = tokio::fs::rename(&tmp, path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error.into());
        }
        Ok(())
    }

    async fn read_head(&self, project_id: &str) -> Result<Option<SyncHead>> {
        let path = self.project_dir(project_id)?.join("head.json");
        match tokio::fs::read(path).await {
            Ok(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).context("invalid relay head")?,
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn validate_revision(&self, project_id: &str, request: &CommitRequest) -> Result<()> {
        let revision = &request.revision;
        if revision.protocol_version != SYNC_PROTOCOL_VERSION {
            anyhow::bail!("unsupported sync protocol version");
        }
        if revision.project_id != project_id {
            anyhow::bail!("revision project id does not match route");
        }
        Self::validate_component(&revision.revision_id, "revision id")?;
        Self::validate_component(&revision.device_id, "device id")?;
        if let Some(parent) = &request.base_revision {
            Self::validate_component(parent, "base revision id")?;
        }
        if revision.parent_revision != request.base_revision {
            anyhow::bail!("revision parent does not match commit base");
        }
        Self::validate_blob_id(&revision.state_hash)?;
        Self::validate_blob_id(&revision.auth_tag)?;
        Self::validate_blob_id(&revision.metadata_blob)?;
        Self::validate_blob_id(&revision.manifest_blob)?;
        if revision.workspace_blobs.len() > 100_000 {
            anyhow::bail!("revision contains too many workspace blobs");
        }
        for blob in std::iter::once(&revision.metadata_blob)
            .chain(std::iter::once(&revision.manifest_blob))
            .chain(revision.workspace_blobs.iter())
        {
            if !self.blob_exists(blob).await? {
                anyhow::bail!("revision references missing blob {blob}");
            }
        }
        Ok(())
    }
}

#[async_trait]
impl SyncTransport for FileRelay {
    async fn head(&self, project_id: &str) -> Result<Option<SyncHead>> {
        self.read_head(project_id).await
    }

    async fn revision(&self, project_id: &str, revision_id: &str) -> Result<SyncRevision> {
        Self::validate_component(revision_id, "revision id")?;
        let path = self
            .project_dir(project_id)?
            .join("revisions")
            .join(format!("{revision_id}.json"));
        let bytes = tokio::fs::read(path)
            .await
            .context("relay revision not found")?;
        let revision: SyncRevision = serde_json::from_slice(&bytes)?;
        if revision.project_id != project_id || revision.revision_id != revision_id {
            anyhow::bail!("relay revision identity mismatch");
        }
        Ok(revision)
    }

    async fn blob_exists(&self, blob_id: &str) -> Result<bool> {
        Ok(tokio::fs::try_exists(self.blob_path(blob_id)?).await?)
    }

    async fn get_blob(&self, blob_id: &str) -> Result<Vec<u8>> {
        Ok(tokio::fs::read(self.blob_path(blob_id)?).await?)
    }

    async fn put_blob(&self, blob_id: &str, bytes: Vec<u8>) -> Result<()> {
        let path = self.blob_path(blob_id)?;
        let actual = hex::encode(Sha256::digest(&bytes));
        if actual != blob_id {
            anyhow::bail!("blob hash does not match its id");
        }
        if tokio::fs::try_exists(&path).await? {
            return Ok(());
        }
        Self::write_atomic(&path, &bytes).await
    }

    async fn commit(&self, project_id: &str, request: CommitRequest) -> Result<CommitOutcome> {
        let _guard = self.commit_lock.lock().await;
        let current = self.read_head(project_id).await?;
        if current.as_ref().map(|h| h.revision_id.as_str()) != request.base_revision.as_deref() {
            return Ok(CommitOutcome::Conflict(current));
        }
        self.validate_revision(project_id, &request).await?;
        let project_dir = self.project_dir(project_id)?;
        let revision_path = project_dir
            .join("revisions")
            .join(format!("{}.json", request.revision.revision_id));
        Self::write_atomic(&revision_path, &serde_json::to_vec(&request.revision)?).await?;
        let head = SyncHead {
            revision_id: request.revision.revision_id,
        };
        Self::write_atomic(&project_dir.join("head.json"), &serde_json::to_vec(&head)?).await?;
        Ok(CommitOutcome::Committed(head))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sha256_hex, SyncRevision};

    fn revision(
        project_id: &str,
        id: &str,
        parent: Option<&str>,
        blobs: &[String],
    ) -> SyncRevision {
        SyncRevision {
            protocol_version: SYNC_PROTOCOL_VERSION,
            project_id: project_id.into(),
            revision_id: id.into(),
            parent_revision: parent.map(str::to_string),
            device_id: "device-1".into(),
            created_at: 1,
            metadata_blob: blobs[0].clone(),
            manifest_blob: blobs[1].clone(),
            workspace_blobs: blobs[2..].to_vec(),
            state_hash: sha256_hex(b"state"),
            auth_tag: sha256_hex(b"auth"),
        }
    }

    #[tokio::test]
    async fn relay_commits_with_compare_and_swap() {
        let root = std::env::temp_dir().join(format!("wisp-relay-{}", uuid::Uuid::new_v4()));
        let relay = FileRelay::open(&root).await.unwrap();
        let bodies = [
            b"metadata".as_slice(),
            b"manifest".as_slice(),
            b"file".as_slice(),
        ];
        let ids = bodies
            .iter()
            .map(|body| sha256_hex(body))
            .collect::<Vec<_>>();
        for (id, body) in ids.iter().zip(bodies) {
            relay.put_blob(id, body.to_vec()).await.unwrap();
        }
        let first = revision("project-1", "revision-1", None, &ids);
        assert!(matches!(
            relay
                .commit(
                    "project-1",
                    CommitRequest {
                        base_revision: None,
                        revision: first
                    }
                )
                .await
                .unwrap(),
            CommitOutcome::Committed(_)
        ));
        let stale = revision("project-1", "revision-stale", None, &ids);
        assert!(matches!(
            relay
                .commit(
                    "project-1",
                    CommitRequest {
                        base_revision: None,
                        revision: stale
                    }
                )
                .await
                .unwrap(),
            CommitOutcome::Conflict(Some(_))
        ));
        let second = revision("project-1", "revision-2", Some("revision-1"), &ids);
        relay
            .commit(
                "project-1",
                CommitRequest {
                    base_revision: Some("revision-1".into()),
                    revision: second,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            relay.head("project-1").await.unwrap().unwrap().revision_id,
            "revision-2"
        );
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn relay_rejects_missing_or_mismatched_blobs() {
        let root = std::env::temp_dir().join(format!("wisp-relay-{}", uuid::Uuid::new_v4()));
        let relay = FileRelay::open(&root).await.unwrap();
        let id = sha256_hex(b"actual");
        assert!(relay.put_blob(&id, b"wrong".to_vec()).await.is_err());
        let ids = vec![id.clone(), id.clone()];
        assert!(relay
            .commit(
                "project-1",
                CommitRequest {
                    base_revision: None,
                    revision: revision("project-1", "revision-1", None, &ids)
                }
            )
            .await
            .is_err());
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
