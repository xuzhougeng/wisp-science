//! Explicit, previewed conversation file operations. The caller holds exclusive
//! project activity guards. A disk journal fences filesystem changes around the
//! SQLite commits (including the two independent project databases).
use crate::{ArtifactVersion, MessageResourceLink, Store};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, Transaction};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use wisp_dto::{RetainedSessionArtifact, SessionArtifactPreview};

/// Project evidence and references from surviving conversations must outlive
/// the conversation that first registered the logical artifact.
pub(super) async fn disposable_ids(
    tx: &mut Transaction<'_, Sqlite>,
    frame: &str,
) -> Result<Vec<String>> {
    let rows = sqlx::query(
        "SELECT a.id FROM artifacts a WHERE a.root_frame_id=? AND a.exploration_id IS NULL \
         AND NOT EXISTS(SELECT 1 FROM research_nodes n JOIN research_edges e ON e.source_id=n.id OR e.target_id=n.id WHERE n.kind='artifact' AND n.ref_id=a.id) \
         AND NOT EXISTS(SELECT 1 FROM turn_file_undo u JOIN frames f ON f.id=u.frame_id \
             WHERE f.root_frame_id<>a.root_frame_id AND a.logical_key='path:'||u.path) \
         AND NOT EXISTS(SELECT 1 FROM session_ui_events e JOIN frames f ON f.id=e.frame_id JOIN json_tree(e.event_json) j \
             WHERE f.root_frame_id<>a.root_frame_id AND (j.value=a.id OR j.value IN (SELECT id FROM artifact_versions WHERE artifact_id=a.id))) \
         AND NOT EXISTS(SELECT 1 FROM run_artifacts r WHERE r.artifact_id=a.id) \
         AND NOT EXISTS(SELECT 1 FROM exploration_baseline_artifact_heads h WHERE h.artifact_id=a.id) \
         AND NOT EXISTS(SELECT 1 FROM message_resource_links l JOIN frames f ON f.id=l.frame_id \
             WHERE l.artifact_id=a.id AND f.root_frame_id<>a.root_frame_id) \
         AND NOT EXISTS(SELECT 1 FROM artifact_versions v WHERE v.artifact_id=a.id AND (\
             v.producing_run_id IS NOT NULL OR v.source_discarded_at IS NOT NULL \
             OR EXISTS(SELECT 1 FROM run_inputs r WHERE r.artifact_version_id=v.id) \
             OR EXISTS(SELECT 1 FROM run_outputs r WHERE r.artifact_version_id=v.id) \
             OR EXISTS(SELECT 1 FROM evidence_bindings r WHERE r.artifact_version_id=v.id) \
             OR EXISTS(SELECT 1 FROM reproduction_results r WHERE r.expected_artifact_version_id=v.id) \
             OR EXISTS(SELECT 1 FROM method_search_runs r WHERE r.spec_artifact_version_id=v.id) \
             OR EXISTS(SELECT 1 FROM message_resource_links l JOIN frames f ON f.id=l.frame_id \
                 WHERE l.artifact_version_id=v.id AND f.root_frame_id<>a.root_frame_id))) ORDER BY a.id")
        .bind(frame).fetch_all(&mut **tx).await?;
    let mut ids: BTreeSet<String> = rows.into_iter().map(|r| r.get("id")).collect();
    // Keep the whole connected lineage when any endpoint survives this session.
    let edges: Vec<(String, String)> = sqlx::query_as(
        "SELECT v.artifact_id,p.artifact_id FROM artifact_versions v JOIN artifact_versions p ON p.id=v.parent_version_id \
         UNION SELECT v.artifact_id,p.artifact_id FROM artifact_dependencies d \
         JOIN artifact_versions v ON v.id=d.artifact_version_id JOIN artifact_versions p ON p.id=d.depends_on_version_id")
        .fetch_all(&mut **tx).await?;
    loop {
        let before = ids.len();
        for (a, b) in &edges {
            if ids.contains(a) != ids.contains(b) {
                ids.remove(a);
                ids.remove(b);
            }
        }
        if before == ids.len() {
            break;
        }
    }
    Ok(ids.into_iter().collect())
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Artifact {
    id: String,
    filename: String,
    content_type: String,
    storage_path: String,
    logical_key: Option<String>,
    latest_version_id: Option<String>,
    created_at: i64,
    versions: Vec<ArtifactVersion>,
}
#[derive(Clone, Serialize, Deserialize)]
struct FileChange {
    relative: String,
    checksum: String,
    /// Content-addressed blobs can also be used by a retained artifact.
    remove_source: bool,
}
#[derive(Serialize)]
pub(super) struct ArtifactPlan {
    #[serde(skip)]
    pub(super) operation_id: Option<String>,
    source_project: String,
    pub preview: SessionArtifactPreview,
    artifacts: Vec<Artifact>,
    files: Vec<FileChange>,
    source: PathBuf,
    target: Option<PathBuf>,
}
impl ArtifactPlan {
    pub(super) async fn validate_records(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        frame: &str,
    ) -> Result<()> {
        for artifact in &self.artifacts {
            let row = sqlx::query(
                "SELECT * FROM artifacts WHERE id=? AND project_id=? AND root_frame_id=?",
            )
            .bind(&artifact.id)
            .bind(&self.source_project)
            .bind(frame)
            .fetch_optional(&mut **tx)
            .await?
            .context("Session artifacts changed. Review a fresh preview before continuing.")?;
            let versions = sqlx::query(
                "SELECT * FROM artifact_versions WHERE artifact_id=? ORDER BY version_number",
            )
            .bind(&artifact.id)
            .fetch_all(&mut **tx)
            .await?
            .into_iter()
            .map(crate::artifact_version_from_row)
            .collect::<Result<Vec<_>>>()?;
            if row.try_get::<Option<String>, _>("latest_version_id")? != artifact.latest_version_id
                || row.try_get::<String, _>("storage_path")? != artifact.storage_path
                || row.try_get::<Option<String>, _>("logical_key")? != artifact.logical_key
                || row.try_get::<String, _>("filename")? != artifact.filename
                || row.try_get::<String, _>("content_type")? != artifact.content_type
                || versions != artifact.versions
            {
                bail!("Session artifacts changed. Review a fresh preview before continuing.");
            }
        }
        Ok(())
    }

    pub(super) async fn commit_receipt(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        project: &str,
        frame: &str,
        role: &str,
    ) -> Result<()> {
        sqlx::query("INSERT INTO session_file_operations(operation_id,role,project_id,frame_id,committed_at) VALUES(?,?,?,?,?)")
            .bind(self.operation_id.as_deref().context("Missing session operation identity")?)
            .bind(role).bind(project).bind(frame).bind(chrono::Utc::now().timestamp()).execute(&mut **tx).await?;
        Ok(())
    }
    pub(super) async fn source_commit_receipt(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        frame: &str,
    ) -> Result<()> {
        self.commit_receipt(tx, &self.source_project, frame, "source")
            .await
    }
    pub(super) fn ids(&self) -> Vec<String> {
        self.artifacts.iter().map(|a| a.id.clone()).collect()
    }
}

fn checksum(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 128 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}

/// Reject symlinks/junctions, traversal, remote paths, directories and internal
/// project state. This runs again immediately before every filesystem mutation.
fn safe_path(root: &Path, value: &str) -> Result<(String, PathBuf)> {
    let normalized = value.replace('\\', "/");
    let input = Path::new(&normalized);
    let relative = if input.is_absolute() {
        input.strip_prefix(root)?
    } else {
        input
    };
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
        || relative
            .components()
            .any(|p| p.as_os_str().to_string_lossy().contains(':'))
    {
        bail!("Unsafe artifact path: {value}");
    }
    let name = relative.to_string_lossy().replace('\\', "/");
    if name.split('/').any(|p| p.eq_ignore_ascii_case(".git"))
        || (name.to_lowercase().starts_with(".wisp/") && !name.starts_with(".wisp/artifacts/"))
    {
        bail!("Artifact points to project state: {value}");
    }
    let mut path = root.to_path_buf();
    let parts: Vec<_> = relative.components().collect();
    for (i, part) in parts.iter().enumerate() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() || dunce::canonicalize(&path)? != path {
                    bail!("Artifact path contains a link: {value}");
                }
                if i + 1 < parts.len() && !meta.is_dir() {
                    bail!("Artifact parent is not a directory: {value}");
                }
                if i + 1 == parts.len() && !meta.is_file() {
                    bail!("Artifact is not a regular file: {value}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok((name, path))
}

impl Store {
    pub async fn preview_session_artifacts(
        &self,
        frame: &str,
        project: &str,
        target: Option<&str>,
    ) -> Result<SessionArtifactPreview> {
        Ok(self
            .session_artifact_plan(frame, project, target)
            .await?
            .preview)
    }

    async fn session_artifact_plan(
        &self,
        frame: &str,
        project: &str,
        target: Option<&str>,
    ) -> Result<ArtifactPlan> {
        let source_store = self
            .route_project(project)
            .await?
            .unwrap_or_else(|| self.clone());
        let source = dunce::canonicalize(
            self.get_project(project)
                .await?
                .context("Project not found")?
                .1,
        )?;
        let target_root = match target {
            Some(id) if id == project => bail!("Source and target projects must be different"),
            Some(id) => Some(dunce::canonicalize(
                self.get_project(id)
                    .await?
                    .context("Target project not found")?
                    .1,
            )?),
            None => None,
        };
        if let Some(root) = &target_root {
            if root.starts_with(&source) || source.starts_with(root) {
                bail!("Project workspaces must not overlap");
            }
        }
        let mut tx = source_store.begin_write().await?;
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM frames WHERE id=? AND project_id=? AND parent_frame_id=id AND status<>'deleted' AND exploration_id IS NULL)")
            .bind(frame).bind(project).fetch_one(&mut *tx).await?;
        if !valid {
            bail!("Session not found or is an exploration conversation");
        }
        let disposable: BTreeSet<_> = disposable_ids(&mut tx, frame).await?.into_iter().collect();
        let rows = sqlx::query("SELECT a.* FROM artifacts a WHERE a.project_id=? AND (a.root_frame_id=? \
            OR EXISTS(SELECT 1 FROM message_resource_links l JOIN frames f ON f.id=l.frame_id WHERE f.root_frame_id=? \
                AND (l.artifact_id=a.id OR l.artifact_version_id IN (SELECT id FROM artifact_versions WHERE artifact_id=a.id)))) ORDER BY a.id")
            .bind(project).bind(frame).bind(frame)
            .fetch_all(&mut *tx)
            .await?;
        let all_paths: Vec<(String,String,Option<String>)> = sqlx::query_as(
            "SELECT a.id,v.storage_path,a.logical_key FROM artifacts a JOIN artifact_versions v ON v.artifact_id=a.id WHERE a.project_id=?1 UNION SELECT id,storage_path,logical_key FROM artifacts WHERE project_id=?1")
            .bind(project).fetch_all(&mut *tx).await?;
        // Resolve each known path once; large conversations must not perform
        // a filesystem walk for every artifact × every other artifact.
        let mut path_owners = BTreeMap::<String, BTreeSet<String>>::new();
        for (id, path, key) in &all_paths {
            for path in std::iter::once(path.as_str())
                .chain(key.as_deref().and_then(|k| k.strip_prefix("path:")))
            {
                if let Ok((relative, _)) = safe_path(&source, path) {
                    path_owners.entry(relative).or_default().insert(id.clone());
                }
            }
        }
        let mut checksums = BTreeMap::<PathBuf, String>::new();
        let mut read_checksum = |path: &Path| -> Result<String> {
            if let Some(value) = checksums.get(path) {
                return Ok(value.clone());
            }
            let value = checksum(path)?;
            checksums.insert(path.to_path_buf(), value.clone());
            Ok(value)
        };
        let mut preview = SessionArtifactPreview::default();
        let mut artifacts = Vec::new();
        let mut files = BTreeMap::<String, FileChange>::new();
        for row in rows {
            let id: String = row.try_get("id")?;
            let name: String = row.try_get("filename")?;
            let logical_key: Option<String> = row.try_get("logical_key")?;
            let storage_path: String = row.try_get("storage_path")?;
            let mut artifact = Artifact {
                id: id.clone(),
                filename: name.clone(),
                content_type: row.try_get("content_type")?,
                storage_path,
                logical_key,
                latest_version_id: row.try_get("latest_version_id")?,
                created_at: row.try_get("created_at")?,
                versions: vec![],
            };
            let versions = sqlx::query(
                "SELECT * FROM artifact_versions WHERE artifact_id=? ORDER BY version_number",
            )
            .bind(&id)
            .fetch_all(&mut *tx)
            .await?;
            for version in versions {
                artifact
                    .versions
                    .push(crate::artifact_version_from_row(version)?);
            }
            let logical = artifact
                .logical_key
                .as_deref()
                .and_then(|k| k.strip_prefix("path:"));
            let upload = logical
                .is_some_and(|p| p.replace('\\', "/").to_lowercase().starts_with("uploads/"))
                || safe_path(&source, &artifact.storage_path)
                    .is_ok_and(|(p, _)| p.to_lowercase().starts_with("uploads/"));
            let mut reason = if upload {
                Some("upload")
            } else if !disposable.contains(&id) {
                Some("shared")
            } else {
                None
            };
            let mut candidates = BTreeMap::new();
            if reason.is_none() {
                for version in &artifact.versions {
                    let Ok((relative, path)) = safe_path(&source, &version.storage_path) else {
                        reason = Some("unavailable");
                        break;
                    };
                    if version.checksum.is_none() {
                        reason = Some("unavailable");
                        break;
                    }
                    let Ok(hash) = read_checksum(&path) else {
                        reason = Some("unavailable");
                        break;
                    };
                    if version
                        .checksum
                        .as_ref()
                        .is_some_and(|expected| expected != &hash)
                    {
                        reason = Some("changed");
                        break;
                    }
                    candidates.insert(
                        relative.clone(),
                        FileChange {
                            relative,
                            checksum: hash,
                            remove_source: true,
                        },
                    );
                }
                if !candidates.contains_key(
                    &safe_path(&source, &artifact.storage_path)
                        .map(|p| p.0)
                        .unwrap_or_default(),
                ) {
                    reason = Some("unavailable");
                }
                if let Some(logical) = logical {
                    match safe_path(&source, logical) {
                        Ok((relative, path)) if path.exists() => {
                            let latest = artifact
                                .versions
                                .iter()
                                .find(|v| Some(&v.id) == artifact.latest_version_id.as_ref());
                            let hash = read_checksum(&path)?;
                            if latest.and_then(|v| v.checksum.as_ref()) != Some(&hash) {
                                reason = Some("changed");
                            } else {
                                candidates.insert(
                                    relative.clone(),
                                    FileChange {
                                        relative,
                                        checksum: hash,
                                        remove_source: true,
                                    },
                                );
                            }
                        }
                        Ok(_) => {} // A snapshot can survive deletion of its workspace original.
                        Err(_) => reason = Some("unavailable"),
                    }
                }
            }
            if let Some(reason) = reason {
                preview.retained.push(RetainedSessionArtifact {
                    name,
                    reason: reason.into(),
                });
                continue;
            }
            for file in candidates.values_mut() {
                let shared_path = path_owners
                    .get(&file.relative)
                    .is_some_and(|owners| owners.iter().any(|other| other != &id));
                // Shared snapshot bytes remain in the source, even when their
                // separate artifact records are safe to transfer.
                if shared_path {
                    file.remove_source = false;
                }
                if !file.relative.starts_with(".wisp/") {
                    let other_turn: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM turn_file_undo u JOIN frames f ON f.id=u.frame_id WHERE f.root_frame_id<>? AND u.path=?)")
                        .bind(frame).bind(&file.relative).fetch_one(&mut *tx).await?;
                    if other_turn {
                        file.remove_source = false;
                    }
                }
                if let Some(root) = &target_root {
                    // Immutable storage is placed in a fresh import namespace.
                    if !file.relative.starts_with(".wisp/")
                        && safe_path(root, &file.relative)?.1.exists()
                    {
                        bail!(
                            "Target file already exists: {}. Rename it or choose another project.",
                            file.relative
                        );
                    }
                }
                if file.remove_source || target.is_some() {
                    preview.files.push(file.relative.clone());
                }
                if !file.remove_source {
                    preview.retained.push(RetainedSessionArtifact {
                        name: file.relative.clone(),
                        reason: "shared".into(),
                    });
                }
            }
            files.extend(candidates);
            preview.artifacts.push(name);
            artifacts.push(artifact);
        }
        let edges: Vec<(String,String)> = sqlx::query_as(
            "SELECT v.artifact_id,p.artifact_id FROM artifact_versions v JOIN artifact_versions p ON p.id=v.parent_version_id UNION SELECT v.artifact_id,p.artifact_id FROM artifact_dependencies d JOIN artifact_versions v ON v.id=d.artifact_version_id JOIN artifact_versions p ON p.id=d.depends_on_version_id"
        ).fetch_all(&mut *tx).await?;
        let mut eligible: BTreeSet<String> = artifacts.iter().map(|a| a.id.clone()).collect();
        loop {
            let before = eligible.len();
            for (a, b) in &edges {
                if eligible.contains(a) != eligible.contains(b) {
                    eligible.remove(a);
                    eligible.remove(b);
                }
            }
            if eligible.len() == before {
                break;
            }
        }
        artifacts.retain(|a| {
            if eligible.contains(&a.id) {
                true
            } else {
                preview.retained.push(RetainedSessionArtifact {
                    name: a.filename.clone(),
                    reason: "shared".into(),
                });
                false
            }
        });
        let used_paths: BTreeSet<String> = artifacts
            .iter()
            .flat_map(|a| {
                a.versions.iter().map(|v| v.storage_path.as_str()).chain(
                    a.logical_key
                        .as_deref()
                        .and_then(|k| k.strip_prefix("path:")),
                )
            })
            .filter_map(|p| safe_path(&source, p).ok().map(|p| p.0))
            .collect();
        files.retain(|path, _| used_paths.contains(path));
        preview.files.retain(|path| used_paths.contains(path));
        preview.artifacts = artifacts.iter().map(|a| a.filename.clone()).collect();
        tx.commit().await?;
        preview.files.sort();
        preview.files.dedup();
        let mut plan = ArtifactPlan {
            operation_id: None,
            source_project: project.into(),
            preview,
            artifacts,
            files: files.into_values().collect(),
            source,
            target: target_root,
        };
        plan.preview.fingerprint = hex::encode(Sha256::digest(serde_json::to_vec(&plan)?));
        Ok(plan)
    }
}

#[derive(Serialize, Deserialize)]
struct Journal {
    operation: String,
    frame: String,
    project: String,
    target_project: Option<String>,
    target_frame: Option<String>,
    source: PathBuf,
    target: Option<PathBuf>,
    files: Vec<FileChange>,
    destinations: BTreeMap<String, String>,
}

fn journal_directory(root: &Path, id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id)?;
    let mut path = root.to_path_buf();
    for part in [".wisp", "session-file-operations", id] {
        path.push(part);
        if path.exists() {
            let meta = fs::symlink_metadata(&path)?;
            if !meta.is_dir()
                || meta.file_type().is_symlink()
                || dunce::canonicalize(&path)? != path
            {
                bail!("Unsafe session operation directory");
            }
        } else {
            fs::create_dir(&path)?;
        }
    }
    Ok(path)
}

impl Journal {
    fn target_temporary(&self, index: usize) -> Result<PathBuf> {
        let target = self.target.as_ref().context("Missing target")?;
        Ok(safe_path(
            target,
            &format!(
                ".wisp/artifacts/session-imports/{}/staging/{index}",
                self.operation
            ),
        )?
        .1)
    }
    fn stage(&self, directory: &Path) -> Result<()> {
        // Durable intent precedes the first mutation; the manifest uses paths
        // relative to project roots and can be replayed after an interrupted app.
        let mut manifest = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("journal.json"))?;
        manifest.write_all(&serde_json::to_vec(self)?)?;
        manifest.sync_all()?;
        for (index, file) in self.files.iter().enumerate() {
            let source = safe_path(&self.source, &file.relative)?.1;
            if checksum(&source)? != file.checksum {
                bail!("Artifact changed since preview: {}", file.relative);
            }
            if let Some(target) = &self.target {
                let destination = safe_path(target, &self.destinations[&file.relative])?.1;
                fs::create_dir_all(destination.parent().context("Missing artifact parent")?)?;
                let destination = safe_path(target, &self.destinations[&file.relative])?.1;
                let temporary = self.target_temporary(index)?;
                fs::create_dir_all(temporary.parent().context("Missing staging parent")?)?;
                let temporary = self.target_temporary(index)?;
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)
                    .with_context(|| {
                        format!(
                            "Target file already exists or cannot be created: {}",
                            destination.display()
                        )
                    })?;
                std::io::copy(&mut fs::File::open(&source)?, &mut output)?;
                // Preserve executable scripts on Unix. Windows read-only flags
                // cannot be applied to staging links before cleanup completes.
                #[cfg(unix)]
                output.set_permissions(fs::metadata(&source)?.permissions())?;
                output.sync_all()?;
                if checksum(&temporary)? != file.checksum {
                    bail!("Artifact changed during copy: {}", file.relative);
                }
                drop(output);
                fs::hard_link(&temporary, &destination).with_context(|| {
                    format!(
                        "Target file already exists or cannot be created: {}",
                        destination.display()
                    )
                })?;
            }
            if file.remove_source {
                let source = safe_path(&self.source, &file.relative)?.1;
                fs::rename(source, directory.join(format!("{index}.file")))?;
            }
        }
        for (index, file) in self.files.iter().enumerate() {
            if file.remove_source
                && checksum(&directory.join(format!("{index}.file")))? != file.checksum
            {
                bail!("Artifact changed during operation: {}", file.relative);
            }
        }
        Ok(())
    }
    fn finish(
        &self,
        directory: &Path,
        source_committed: bool,
        target_committed: bool,
    ) -> Result<()> {
        for (index, file) in self.files.iter().enumerate() {
            let backup = directory.join(format!("{index}.file"));
            if backup.exists() {
                if fs::symlink_metadata(&backup)?.file_type().is_symlink()
                    || (source_committed && checksum(&backup)? != file.checksum)
                {
                    bail!("Recovery backup changed: {}", backup.display());
                }
                if source_committed {
                    // Windows refuses to unlink a read-only file. This is an
                    // exclusively owned backup of an explicitly deleted file;
                    // leave permissions intact whenever we are rolling back.
                    #[cfg(windows)]
                    {
                        let mut permissions = fs::metadata(&backup)?.permissions();
                        if permissions.readonly() {
                            permissions.set_readonly(false);
                            fs::set_permissions(&backup, permissions)?;
                        }
                    }
                    fs::remove_file(&backup)?;
                } else {
                    let source = safe_path(&self.source, &file.relative)?.1;
                    if source.exists() {
                        bail!(
                            "Recovery kept the original at {} because {} was recreated",
                            backup.display(),
                            source.display()
                        );
                    }
                    fs::create_dir_all(source.parent().context("Missing source parent")?)?;
                    fs::rename(&backup, source)?;
                }
            }
            if let Some(target) = &self.target {
                let temporary = self.target_temporary(index)?;
                let destination = safe_path(target, &self.destinations[&file.relative])?.1;
                if !target_committed
                    && temporary.exists()
                    && destination.exists()
                    && same_file::is_same_file(&temporary, &destination)?
                {
                    if checksum(&destination)? != file.checksum {
                        bail!(
                            "Recovery retained changed target file: {}",
                            destination.display()
                        );
                    }
                    fs::remove_file(destination)?;
                }
                if temporary.exists() {
                    fs::remove_file(temporary)?;
                }
            }
        }
        fs::remove_file(directory.join("journal.json"))?;
        fs::remove_dir(directory)?;
        Ok(())
    }
}

pub(super) struct ArtifactTransfer {
    pub plan: ArtifactPlan,
    artifact_ids: BTreeMap<String, String>,
    version_ids: BTreeMap<String, String>,
    paths: BTreeMap<String, String>,
    links: Vec<MessageResourceLink>,
    dependencies: Vec<(String, String, Option<String>, String, String, i64)>,
    environments: Vec<sqlx::sqlite::SqliteRow>,
}
impl ArtifactTransfer {
    pub(super) async fn insert(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        project: &str,
        frame: &str,
    ) -> Result<()> {
        for row in &self.environments {
            let hash: String = row.try_get("hash")?;
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM env_snapshots WHERE hash=?)")
                    .bind(hash)
                    .fetch_one(&mut **tx)
                    .await?;
            if !exists {
                sqlx::query("INSERT INTO env_snapshots(hash,env_name,packages_json,snapshot_json,hash_algorithm,created_at) VALUES(?,?,?,?,?,?)")
                    .bind(row.try_get::<String,_>("hash")?).bind(row.try_get::<Option<String>,_>("env_name")?)
                    .bind(row.try_get::<String,_>("packages_json")?).bind(row.try_get::<String,_>("snapshot_json")?)
                    .bind(row.try_get::<String,_>("hash_algorithm")?).bind(row.try_get::<i64,_>("created_at")?).execute(&mut **tx).await?;
            }
        }
        let storage = |path: &str| -> Result<String> {
            let relative = safe_path(&self.plan.source, path)?.0;
            Ok(self
                .paths
                .get(&relative)
                .context("Artifact storage was not staged")?
                .clone())
        };
        for artifact in &self.plan.artifacts {
            let id = &self.artifact_ids[&artifact.id];
            sqlx::query("INSERT INTO artifacts(id,project_id,root_frame_id,filename,content_type,storage_path,created_at,latest_version_id,logical_key) VALUES(?,?,?,?,?,?,?,?,?)")
                .bind(id).bind(project).bind(frame).bind(&artifact.filename).bind(&artifact.content_type).bind(storage(&artifact.storage_path)?)
                .bind(artifact.created_at).bind(artifact.latest_version_id.as_ref().and_then(|v|self.version_ids.get(v))).bind(&artifact.logical_key).execute(&mut **tx).await?;
            for version in &artifact.versions {
                sqlx::query("INSERT INTO artifact_versions(id,artifact_id,version_number,content_type,storage_path,size_bytes,checksum,parent_version_id,env_snapshot_hash,materialization,capture_timing,created_at) VALUES(?,?,?,?,?,?,?,NULL,?,?,?,?)")
                    .bind(&self.version_ids[&version.id]).bind(id).bind(version.version_number).bind(&version.content_type).bind(storage(&version.storage_path)?)
                    .bind(version.size_bytes).bind(&version.checksum).bind(&version.env_snapshot_hash).bind(version.materialization.as_str()).bind(version.capture_timing.as_str()).bind(version.created_at)
                    .execute(&mut **tx).await?;
            }
            if let (Some(key), Some(version)) = (&artifact.logical_key, &artifact.latest_version_id)
            {
                sqlx::query("INSERT INTO artifact_heads(project_id,scope_key,logical_key,artifact_id,artifact_version_id,updated_at) VALUES(?,'mainline',?,?,?,?)")
                    .bind(project).bind(key).bind(id).bind(&self.version_ids[version]).bind(artifact.created_at).execute(&mut **tx).await?;
            }
            sqlx::query("INSERT INTO research_nodes(id,project_id,kind,title,ref_id,created_at,updated_at) VALUES(?,?,'artifact',?,?,?,?)")
                .bind(crate::artifact_node_id(id)).bind(project).bind(&artifact.filename).bind(id).bind(artifact.created_at).bind(artifact.created_at).execute(&mut **tx).await?;
        }
        for artifact in &self.plan.artifacts {
            for version in &artifact.versions {
                if let Some(parent) = version
                    .parent_version_id
                    .as_ref()
                    .and_then(|p| self.version_ids.get(p))
                {
                    sqlx::query("UPDATE artifact_versions SET parent_version_id=? WHERE id=?")
                        .bind(parent)
                        .bind(&self.version_ids[&version.id])
                        .execute(&mut **tx)
                        .await?;
                }
            }
        }
        for (v, p, name, basis, confidence, created) in &self.dependencies {
            if let (Some(v), Some(p)) = (self.version_ids.get(v), self.version_ids.get(p)) {
                sqlx::query("INSERT INTO artifact_dependencies(id,artifact_version_id,depends_on_version_id,reference_name,basis,confidence,created_at) VALUES(?,?,?,?,?,?,?)")
                    .bind(uuid::Uuid::new_v4().to_string()).bind(v).bind(p).bind(name).bind(basis).bind(confidence).bind(created).execute(&mut **tx).await?;
            }
        }
        for link in &self.links {
            let artifact = link
                .artifact_id
                .as_ref()
                .and_then(|id| self.artifact_ids.get(id));
            let version = link
                .artifact_version_id
                .as_ref()
                .and_then(|id| self.version_ids.get(id));
            if artifact.is_none() {
                continue;
            }
            sqlx::query("INSERT INTO message_resource_links(id,frame_id,message_seq,ordinal,original_reference,artifact_id,artifact_version_id,display_name,resource_kind,mime_type,status,error,created_artifact,created_version,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
                .bind(uuid::Uuid::new_v4().to_string()).bind(frame).bind(link.message_seq).bind(link.ordinal).bind(&link.original_reference).bind(artifact).bind(version)
                .bind(&link.display_name).bind(&link.resource_kind).bind(&link.mime_type).bind(&link.status).bind(&link.error).bind(link.created_artifact).bind(link.created_version).bind(link.created_at)
                .execute(&mut **tx).await?;
        }
        // Artifact cards in saved UI events must resolve the new identities too.
        let rows: Vec<(i64, String)> =
            sqlx::query_as("SELECT seq,event_json FROM session_ui_events WHERE frame_id=?")
                .bind(frame)
                .fetch_all(&mut **tx)
                .await?;
        for (seq, json) in rows {
            let mut value: serde_json::Value = serde_json::from_str(&json)?;
            fn remap(value: &mut serde_json::Value, ids: &BTreeMap<String, String>) {
                match value {
                    serde_json::Value::String(s) => {
                        if let Some(new) = ids.get(s) {
                            *s = new.clone();
                        }
                    }
                    serde_json::Value::Array(a) => {
                        for v in a {
                            remap(v, ids);
                        }
                    }
                    serde_json::Value::Object(o) => {
                        for v in o.values_mut() {
                            remap(v, ids);
                        }
                    }
                    _ => {}
                }
            }
            remap(&mut value, &self.artifact_ids);
            remap(&mut value, &self.version_ids);
            let mut event_paths = self.paths.clone();
            for (relative, destination) in &self.paths {
                event_paths.insert(
                    self.plan
                        .source
                        .join(relative)
                        .to_string_lossy()
                        .into_owned(),
                    destination.clone(),
                );
            }
            remap(&mut value, &event_paths);
            sqlx::query("UPDATE session_ui_events SET event_json=? WHERE frame_id=? AND seq=?")
                .bind(value.to_string())
                .bind(frame)
                .bind(seq)
                .execute(&mut **tx)
                .await?;
        }
        self.plan
            .commit_receipt(tx, project, frame, "target")
            .await?;
        Ok(())
    }
}

impl Store {
    /// Call under exclusive source/target project activity guards. A matching
    /// preview is mandatory; ordinary transcript-only APIs remain unchanged.
    pub async fn operate_session_with_artifacts(
        &self,
        frame: &str,
        project: &str,
        target: Option<(&str, &str)>,
        fingerprint: &str,
    ) -> Result<()> {
        if self
            .session_artifact_recovery_projects(project)
            .await?
            .iter()
            .any(|id| id != project && Some(id.as_str()) != target.map(|t| t.0))
        {
            bail!("Reopen the source project to recover an interrupted artifact operation before continuing.");
        }
        self.recover_session_artifact_operations(project).await?;
        let mut plan = self
            .session_artifact_plan(frame, project, target.map(|t| t.0))
            .await?;
        if plan.preview.fingerprint != fingerprint {
            bail!("Session artifacts changed. Review a fresh preview before continuing.");
        }
        let operation = uuid::Uuid::new_v4().to_string();
        plan.operation_id = Some(operation.clone());
        let destinations: BTreeMap<_, _> = plan
            .files
            .iter()
            .map(|f| {
                (
                    f.relative.clone(),
                    if f.relative.starts_with(".wisp/") {
                        format!(
                            ".wisp/artifacts/session-imports/{operation}/{}",
                            f.relative.trim_start_matches(".wisp/artifacts/")
                        )
                    } else {
                        f.relative.clone()
                    },
                )
            })
            .collect();
        let journal = Journal {
            operation: operation.clone(),
            frame: frame.into(),
            project: project.into(),
            target_project: target.map(|t| t.0.into()),
            target_frame: target.map(|t| t.1.into()),
            source: plan.source.clone(),
            target: plan.target.clone(),
            files: plan.files.clone(),
            destinations: destinations.clone(),
        };
        let source_store = self
            .route_project(project)
            .await?
            .unwrap_or_else(|| self.clone());
        let artifact_ids = plan
            .artifacts
            .iter()
            .map(|a| {
                (
                    a.id.clone(),
                    match &a.logical_key {
                        Some(key) => {
                            crate::logical_artifact_id(target.map(|t| t.0).unwrap_or(project), key)
                        }
                        None => uuid::Uuid::new_v4().to_string(),
                    },
                )
            })
            .collect();
        let version_ids = plan
            .artifacts
            .iter()
            .flat_map(|a| a.versions.iter())
            .map(|v| (v.id.clone(), uuid::Uuid::new_v4().to_string()))
            .collect();
        let paths = destinations
            .into_iter()
            .map(|(old, new)| {
                (
                    old,
                    plan.target
                        .as_ref()
                        .unwrap_or(&plan.source)
                        .join(new)
                        .to_string_lossy()
                        .into_owned(),
                )
            })
            .collect();
        let links = source_store
            .list_message_resource_links(frame, 0, None)
            .await?;
        let dependencies=sqlx::query_as("SELECT artifact_version_id,depends_on_version_id,reference_name,basis,confidence,created_at FROM artifact_dependencies").fetch_all(&source_store.pool).await?;
        let mut environments = Vec::new();
        let hashes: BTreeSet<_> = plan
            .artifacts
            .iter()
            .flat_map(|a| {
                a.versions
                    .iter()
                    .filter_map(|v| v.env_snapshot_hash.as_ref())
            })
            .collect();
        for hash in hashes {
            environments.push(
                sqlx::query("SELECT * FROM env_snapshots WHERE hash=?")
                    .bind(hash)
                    .fetch_one(&source_store.pool)
                    .await?,
            );
        }
        let transfer = ArtifactTransfer {
            plan,
            artifact_ids,
            version_ids,
            paths,
            links,
            dependencies,
            environments,
        };
        if let Some((project, _)) = target {
            let store = self
                .route_project(project)
                .await?
                .unwrap_or_else(|| self.clone());
            for artifact in &transfer.plan.artifacts {
                let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artifacts WHERE project_id=? AND (id=? OR logical_key=?))")
                    .bind(project).bind(&transfer.artifact_ids[&artifact.id]).bind(&artifact.logical_key).fetch_one(&store.pool).await?;
                if exists {
                    bail!("Target artifact already exists: {}", artifact.filename);
                }
            }
        }
        let directory = journal_directory(&journal.source, &operation)?;
        let result = match journal.stage(&directory) {
            Err(e) => Err(e),
            Ok(()) => match target {
                Some((target, new_frame)) => {
                    self.transfer_session_to_project(
                        frame,
                        project,
                        target,
                        new_frame,
                        true,
                        Some(&transfer),
                    )
                    .await
                }
                None => {
                    self.delete_session_impl(frame, project, Some(&transfer.plan))
                        .await
                }
            },
        };
        let (source_done, target_done) = self.journal_committed(&journal).await?;
        let cleanup = journal.finish(&directory, source_done, target_done);
        if cleanup.is_ok() {
            if let Err(error) = self.forget_session_artifact_receipts(&journal).await {
                tracing::warn!(%error,"session operation receipt cleanup deferred");
            }
        }
        match (result, cleanup) {
            (Err(error), Err(recovery)) => bail!(
                "{error}. File recovery pending at {}: {recovery}",
                directory.display()
            ),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => {
                // Database commit succeeded. Keep a replayable cleanup journal;
                // do not report the session operation as uncommitted/retryable.
                tracing::warn!(%error,path=%directory.display(),"session file cleanup deferred");
                Ok(())
            }
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    async fn forget_session_artifact_receipts(&self, journal: &Journal) -> Result<()> {
        for project in std::iter::once(&journal.project).chain(journal.target_project.iter()) {
            let store = self
                .route_project(project)
                .await?
                .unwrap_or_else(|| self.clone());
            sqlx::query(
                "DELETE FROM session_file_operations WHERE operation_id=? AND project_id=?",
            )
            .bind(&journal.operation)
            .bind(project)
            .execute(&store.pool)
            .await?;
        }
        Ok(())
    }

    async fn journal_committed(&self, journal: &Journal) -> Result<(bool, bool)> {
        let source = self
            .route_project(&journal.project)
            .await?
            .unwrap_or_else(|| self.clone());
        let source_done: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM session_file_operations WHERE operation_id=? AND role='source' AND project_id=? AND frame_id=?)")
            .bind(&journal.operation).bind(&journal.project).bind(&journal.frame).fetch_one(&source.pool).await?;
        let target_done = if let (Some(project), Some(frame)) =
            (&journal.target_project, &journal.target_frame)
        {
            let target = self
                .route_project(project)
                .await?
                .unwrap_or_else(|| self.clone());
            sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM session_file_operations WHERE operation_id=? AND role='target' AND project_id=? AND frame_id=?)")
                .bind(&journal.operation).bind(project).bind(frame).fetch_one(&target.pool).await?
                && self.frame_project_id(frame).await?.as_ref()==Some(project)
        } else {
            false
        };
        Ok((source_done, target_done))
    }

    pub async fn session_artifact_recovery_projects(&self, project: &str) -> Result<Vec<String>> {
        let workspace = self
            .get_project(project)
            .await?
            .context("Project not found")?
            .1;
        let base = Path::new(&workspace).join(".wisp/session-file-operations");
        if !base.exists() {
            return Ok(vec![]);
        }
        let mut projects = BTreeSet::new();
        for entry in fs::read_dir(base)? {
            let manifest = entry?.path().join("journal.json");
            if manifest.exists() {
                let journal: Journal = serde_json::from_slice(&fs::read(manifest)?)?;
                projects.insert(project.to_string());
                if let Some(target) = journal.target_project {
                    projects.insert(target);
                }
            }
        }
        Ok(projects.into_iter().collect())
    }

    pub async fn recover_session_artifact_operations(&self, project: &str) -> Result<()> {
        let root = dunce::canonicalize(
            self.get_project(project)
                .await?
                .context("Project not found")?
                .1,
        )?;
        let base = root.join(".wisp/session-file-operations");
        if !base.exists() {
            return Ok(());
        }
        if dunce::canonicalize(&base)? != base {
            bail!("Unsafe session recovery directory");
        }
        for entry in fs::read_dir(&base)? {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if uuid::Uuid::parse_str(&id).is_err() {
                continue;
            }
            let directory = journal_directory(&root, &id)?;
            let manifest = directory.join("journal.json");
            if !manifest.exists() {
                continue;
            }
            if fs::symlink_metadata(&manifest)?.file_type().is_symlink() {
                bail!("Unsafe session recovery manifest");
            }
            let journal: Journal = serde_json::from_slice(&fs::read(&manifest)?)?;
            if journal.source != root || journal.project != project {
                bail!("Session recovery project mismatch");
            }
            if let Some(target) = &journal.target_project {
                let target_root = dunce::canonicalize(
                    self.get_project(target)
                        .await?
                        .context("Recovery target project unavailable")?
                        .1,
                )?;
                if journal.target.as_ref() != Some(&target_root) {
                    bail!(
                        "Session recovery target moved; original files kept at {}",
                        directory.display()
                    );
                }
            }
            let (source, target) = self.journal_committed(&journal).await?;
            journal.finish(&directory, source, target)?;
            self.forget_session_artifact_receipts(&journal).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArtifactCaptureTiming, ArtifactMaterialization, ArtifactVersionDraft};

    async fn fixture(separate: bool) -> (tempfile::TempDir, Store, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open_application(&temp.path().join("registry.sqlite"))
            .await
            .unwrap();
        store
            .set_setting(
                "decentralized_project_storage",
                if separate { "true" } else { "false" },
            )
            .await
            .unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        for (id, root) in [("source", &source), ("target", &target)] {
            fs::create_dir(root).unwrap();
            store
                .create_project(id, id, root.to_str().unwrap())
                .await
                .unwrap();
        }
        store
            .create_frame("session", "source", "agent", "model")
            .await
            .unwrap();
        store
            .append_message("session", 1, &wisp_llm::Message::user("Generate figures"))
            .await
            .unwrap();
        store
            .append_message(
                "session",
                2,
                &wisp_llm::Message::assistant("[result](results/plot.svg)"),
            )
            .await
            .unwrap();
        (temp, store, source, target)
    }
    async fn output(
        store: &Store,
        root: &Path,
        name: &str,
        body: &str,
    ) -> (String, String, PathBuf) {
        let logical = format!("path:{name}");
        let id = crate::logical_artifact_id("source", &logical);
        let checksum = hex::encode(Sha256::digest(body.as_bytes()));
        let snapshot = root.join(format!(".wisp/artifacts/sha256/{checksum}.svg"));
        fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        fs::write(&snapshot, body).unwrap();
        fs::create_dir_all(root.join(name).parent().unwrap()).unwrap();
        fs::write(root.join(name), body).unwrap();
        let version = store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: None,
                artifact_id: id.clone(),
                project_id: "source".into(),
                root_frame_id: "session".into(),
                filename: name.into(),
                content_type: "image/svg+xml".into(),
                storage_path: snapshot.to_string_lossy().into(),
                logical_key: Some(logical),
                size_bytes: Some(body.len() as i64),
                checksum: Some(checksum),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        (id, version, snapshot)
    }
    async fn bind(store: &Store, frame: &str, artifact: &str, version: &str) {
        store
            .replace_message_resource_links(
                frame,
                2,
                &[MessageResourceLink {
                    id: uuid::Uuid::new_v4().to_string(),
                    frame_id: frame.into(),
                    message_seq: 2,
                    ordinal: 0,
                    original_reference: "results/plot.svg".into(),
                    artifact_id: Some(artifact.into()),
                    artifact_version_id: Some(version.into()),
                    display_name: "plot.svg".into(),
                    resource_kind: "image".into(),
                    mime_type: "image/svg+xml".into(),
                    status: "ready".into(),
                    error: None,
                    created_artifact: true,
                    created_version: true,
                    created_at: 1,
                }],
            )
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn moves_versions_files_and_message_bindings_in_both_storage_modes() {
        for separate in [false, true] {
            let (_temp, store, source, target) = fixture(separate).await;
            let (id, old, old_file) = output(&store, &source, "results/plot.svg", "old").await;
            let (_, latest, latest_file) =
                output(&store, &source, "results/plot.svg", "latest").await;
            bind(&store, "session", &id, &old).await;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    source.join("results/plot.svg"),
                    fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
            let source_store = store
                .route_project("source")
                .await
                .unwrap()
                .unwrap_or_else(|| store.clone());
            sqlx::query(r#"INSERT INTO env_snapshots(hash,env_name,packages_json,snapshot_json,hash_algorithm,created_at) VALUES('env','python','[]','{"runtime":"python"}','sha256',1)"#)
                .execute(&source_store.pool).await.unwrap();
            sqlx::query("UPDATE artifact_versions SET env_snapshot_hash='env' WHERE id=?")
                .bind(&latest)
                .execute(&source_store.pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO artifact_dependencies(id,artifact_version_id,depends_on_version_id,reference_name,basis,confidence,created_at) VALUES('dependency',?,?,'earlier figure','declared','certain',1)")
                .bind(&latest).bind(&old).execute(&source_store.pool).await.unwrap();
            store
                .append_session_ui_event(
                    "session",
                    1,
                    &serde_json::json!({"kind":"Artifact","frame_id":"session","id":id})
                        .to_string(),
                )
                .await
                .unwrap();
            let preview = store
                .preview_session_artifacts("session", "source", Some("target"))
                .await
                .unwrap();
            assert_eq!(preview.artifacts.len(), 1);
            assert_eq!(preview.files.len(), 3);
            store
                .operate_session_with_artifacts(
                    "session",
                    "source",
                    Some(("target", "moved")),
                    &preview.fingerprint,
                )
                .await
                .unwrap();
            assert!(!source.join("results/plot.svg").exists());
            assert!(!old_file.exists());
            assert!(!latest_file.exists());
            assert_eq!(
                fs::read_to_string(target.join("results/plot.svg")).unwrap(),
                "latest"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(target.join("results/plot.svg"))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o755
                );
            }
            assert_eq!(store.load_messages("moved").await.unwrap().len(), 2);
            assert!(store.frame_project_id("session").await.unwrap().is_none());
            let artifacts = store.list_artifacts("moved").await.unwrap();
            assert_eq!(artifacts.len(), 1);
            let new_id = crate::logical_artifact_id("target", "path:results/plot.svg");
            assert_eq!(artifacts[0].0, new_id);
            assert_eq!(fs::read_to_string(&artifacts[0].3).unwrap(), "latest");
            let head = store
                .get_latest_artifact_version_context(&new_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(head.version.version_number, 2);
            assert_ne!(head.version.id, latest);
            let parent = store
                .get_artifact_version(head.version.parent_version_id.as_deref().unwrap())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(fs::read_to_string(&parent.storage_path).unwrap(), "old");
            let links = store
                .list_message_resource_links("moved", 0, None)
                .await
                .unwrap();
            assert_eq!(links.len(), 1);
            assert_eq!(links[0].artifact_id.as_ref(), Some(&new_id));
            assert_eq!(links[0].artifact_version_id.as_ref(), Some(&parent.id));
            let target_store = store
                .route_project("target")
                .await
                .unwrap()
                .unwrap_or_else(|| store.clone());
            let environment: (String, String) = sqlx::query_as(
                "SELECT snapshot_json,hash_algorithm FROM env_snapshots WHERE hash='env'",
            )
            .fetch_one(&target_store.pool)
            .await
            .unwrap();
            assert_eq!(
                environment,
                (r#"{"runtime":"python"}"#.into(), "sha256".into())
            );
            let dependency:(String,String)=sqlx::query_as("SELECT depends_on_version_id,reference_name FROM artifact_dependencies WHERE artifact_version_id=?").bind(&head.version.id).fetch_one(&target_store.pool).await.unwrap();
            assert_eq!(dependency, (parent.id.clone(), "earlier figure".into()));
            assert!(store.load_session_ui_events("moved").await.unwrap()[0].contains(&new_id));
            assert!(store
                .session_artifact_recovery_projects("source")
                .await
                .unwrap()
                .is_empty());
        }
    }
    #[tokio::test]
    async fn delete_is_opt_in_and_keeps_uploads_changed_shared_and_unregistered_files() {
        let (_temp, store, source, _target) = fixture(false).await;
        let (id, _, snapshot) = output(&store, &source, "results/plot.svg", "generated").await;
        let (upload, _, upload_snapshot) =
            output(&store, &source, "uploads/input.csv", "input").await;
        let (changed, _, _) = output(&store, &source, "results/edited.svg", "old").await;
        fs::write(source.join("results/edited.svg"), "new user edit").unwrap();
        let (shared, version, _) = output(&store, &source, "results/shared.svg", "shared").await;
        store
            .create_frame("other", "source", "agent", "model")
            .await
            .unwrap();
        store
            .append_message("other", 1, &wisp_llm::Message::user("reuse"))
            .await
            .unwrap();
        bind(&store, "other", &shared, &version).await;
        fs::write(source.join("unregistered.txt"), "unrelated").unwrap();
        let preview = store
            .preview_session_artifacts("session", "source", None)
            .await
            .unwrap();
        assert_eq!(preview.artifacts, vec!["results/plot.svg"]);
        assert_eq!(preview.retained.len(), 3);
        store
            .operate_session_with_artifacts("session", "source", None, &preview.fingerprint)
            .await
            .unwrap();
        assert!(!snapshot.exists());
        assert!(!source.join("results/plot.svg").exists());
        assert!(store.get_artifact(&id).await.unwrap().is_none());
        for id in [upload, changed, shared] {
            assert!(store.get_artifact(&id).await.unwrap().is_some());
        }
        assert!(upload_snapshot.exists());
        assert!(source.join("unregistered.txt").exists());
        assert_eq!(
            fs::read_to_string(source.join("results/edited.svg")).unwrap(),
            "new user edit"
        );
        assert_eq!(store.list_sessions("source").await.unwrap().len(), 1);
        let (_temp, store, source, _) = fixture(false).await;
        let (_, _, snapshot) =
            output(&store, &source, "results/plot.svg", "retained by default").await;
        store.delete_session("session", "source").await.unwrap();
        assert!(snapshot.exists());
        assert!(source.join("results/plot.svg").exists());
    }
    #[tokio::test]
    async fn stale_preview_and_target_collisions_do_not_modify_either_project() {
        let (_temp, store, source, target) = fixture(false).await;
        output(&store, &source, "results/plot.svg", "generated").await;
        let preview = store
            .preview_session_artifacts("session", "source", Some("target"))
            .await
            .unwrap();
        fs::create_dir_all(target.join("results")).unwrap();
        fs::write(target.join("results/plot.svg"), "keep me").unwrap();
        assert!(store
            .operate_session_with_artifacts(
                "session",
                "source",
                Some(("target", "moved")),
                &preview.fingerprint
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("already exists"));
        assert_eq!(
            fs::read_to_string(target.join("results/plot.svg")).unwrap(),
            "keep me"
        );
        let preview = store
            .preview_session_artifacts("session", "source", None)
            .await
            .unwrap();
        fs::write(source.join("results/plot.svg"), "changed").unwrap();
        assert!(store
            .operate_session_with_artifacts("session", "source", None, &preview.fingerprint)
            .await
            .unwrap_err()
            .to_string()
            .contains("fresh preview"));
        assert_eq!(store.load_messages("session").await.unwrap().len(), 2);
    }
    #[tokio::test]
    async fn database_failures_restore_source_and_do_not_leave_target_copies() {
        for separate in [false, true] {
            for fail_source in [false, true] {
                let (_temp, store, source, target) = fixture(separate).await;
                output(&store, &source, "results/plot.svg", "generated").await;
                let failing = store
                    .route_project(if fail_source { "source" } else { "target" })
                    .await
                    .unwrap()
                    .unwrap_or_else(|| store.clone());
                sqlx::query(if fail_source {
                    "CREATE TRIGGER fail_operation BEFORE DELETE ON messages WHEN OLD.frame_id='session' BEGIN SELECT RAISE(ABORT,'injected failure'); END"
                } else { "CREATE TRIGGER fail_operation BEFORE INSERT ON artifacts WHEN NEW.project_id='target' BEGIN SELECT RAISE(ABORT,'injected failure'); END" }).execute(&failing.pool).await.unwrap();
                let preview = store
                    .preview_session_artifacts("session", "source", Some("target"))
                    .await
                    .unwrap();
                let error = store
                    .operate_session_with_artifacts(
                        "session",
                        "source",
                        Some(("target", "moved")),
                        &preview.fingerprint,
                    )
                    .await
                    .unwrap_err();
                assert!(error.to_string().contains("injected failure"), "{error}");
                assert_eq!(
                    fs::read_to_string(source.join("results/plot.svg")).unwrap(),
                    "generated"
                );
                assert!(!target.join("results/plot.svg").exists());
                assert_eq!(store.load_messages("session").await.unwrap().len(), 2);
                assert!(store.list_sessions("target").await.unwrap().is_empty());
            }
        }
    }
    #[tokio::test]
    async fn shared_snapshot_bytes_survive_and_recorded_lineage_is_preserved() {
        let (_temp, store, source, _) = fixture(false).await;
        let (first, _, snapshot) = output(&store, &source, "results/one.svg", "same bytes").await;
        let (second, _, _) = output(&store, &source, "uploads/input.svg", "same bytes").await;
        let preview = store
            .preview_session_artifacts("session", "source", None)
            .await
            .unwrap();
        store
            .operate_session_with_artifacts("session", "source", None, &preview.fingerprint)
            .await
            .unwrap();
        assert!(snapshot.exists());
        assert!(store.get_artifact(&first).await.unwrap().is_none());
        assert!(store.get_artifact(&second).await.unwrap().is_some());
    }
    #[tokio::test]
    async fn crash_recovery_restores_uncommitted_files_and_finishes_committed_deletion() {
        for committed in [false, true] {
            let (_temp, store, source, _) = fixture(false).await;
            output(&store, &source, "results/plot.svg", "generated").await;
            let mut plan = store
                .session_artifact_plan("session", "source", None)
                .await
                .unwrap();
            let operation = uuid::Uuid::new_v4().to_string();
            let directory = journal_directory(&source, &operation).unwrap();
            plan.operation_id = Some(operation.clone());
            let journal = Journal {
                operation,
                frame: "session".into(),
                project: "source".into(),
                target_project: None,
                target_frame: None,
                source: source.clone(),
                target: None,
                files: plan.files.clone(),
                destinations: BTreeMap::new(),
            };
            journal.stage(&directory).unwrap();
            assert!(!source.join("results/plot.svg").exists());
            if committed {
                store
                    .delete_session_impl("session", "source", Some(&plan))
                    .await
                    .unwrap();
            }
            store
                .recover_session_artifact_operations("source")
                .await
                .unwrap();
            assert_eq!(source.join("results/plot.svg").exists(), !committed);
            assert!(!directory.exists());
        }
    }
    #[tokio::test]
    async fn recovery_uses_commit_receipts_not_later_transcript_deletion() {
        let (_temp, store, source, _) = fixture(false).await;
        output(&store, &source, "results/plot.svg", "generated").await;
        let plan = store
            .session_artifact_plan("session", "source", None)
            .await
            .unwrap();
        let operation = uuid::Uuid::new_v4().to_string();
        let directory = journal_directory(&source, &operation).unwrap();
        let journal = Journal {
            operation,
            frame: "session".into(),
            project: "source".into(),
            target_project: None,
            target_frame: None,
            source: source.clone(),
            target: None,
            files: plan.files,
            destinations: BTreeMap::new(),
        };
        journal.stage(&directory).unwrap();
        // A different, transcript-only operation is not a receipt for this one.
        store.delete_session("session", "source").await.unwrap();
        store
            .recover_session_artifact_operations("source")
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(source.join("results/plot.svg")).unwrap(),
            "generated"
        );
    }

    #[tokio::test]
    async fn preview_includes_borrowed_artifacts_and_preserves_other_conversation_updates() {
        let (_temp, store, source, _) = fixture(false).await;
        let (id, version, _) = output(&store, &source, "results/plot.svg", "generated").await;
        store
            .create_frame("other", "source", "agent", "model")
            .await
            .unwrap();
        store
            .append_message("other", 1, &wisp_llm::Message::user("reuse"))
            .await
            .unwrap();
        bind(&store, "other", &id, &version).await;
        let preview = store
            .preview_session_artifacts("other", "source", None)
            .await
            .unwrap();
        assert!(preview.artifacts.is_empty());
        assert_eq!(preview.retained[0].reason, "shared");
        // Even non-Markdown generated cards carry a durable reference.
        store
            .replace_message_resource_links("other", 2, &[])
            .await
            .unwrap();
        store
            .append_session_ui_event(
                "other",
                1,
                &serde_json::json!({"kind":"Artifact","id":id}).to_string(),
            )
            .await
            .unwrap();
        let preview = store
            .preview_session_artifacts("session", "source", None)
            .await
            .unwrap();
        assert!(preview.artifacts.is_empty());
        assert_eq!(preview.retained[0].reason, "shared");
        store
            .operate_session_with_artifacts("session", "source", None, &preview.fingerprint)
            .await
            .unwrap();
        assert!(store.get_artifact(&id).await.unwrap().is_some());
        assert!(source.join("results/plot.svg").exists());
    }

    #[tokio::test]
    async fn migration_reinstalls_receipt_table_idempotently() {
        let (temp, store, _, _) = fixture(false).await;
        sqlx::query("DROP TABLE session_file_operations")
            .execute(&store.pool)
            .await
            .unwrap();
        Store::migrate(&store.pool).await.unwrap();
        Store::migrate(&store.pool).await.unwrap();
        let reopened = Store::open_application(&temp.path().join("registry.sqlite"))
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session_file_operations")
            .fetch_one(&reopened.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn a_writer_after_preflight_cannot_delete_an_unreviewed_version() {
        let (_temp, store, source, _) = fixture(false).await;
        let (id, _, _) = output(&store, &source, "results/plot.svg", "first").await;
        let mut plan = store
            .session_artifact_plan("session", "source", None)
            .await
            .unwrap();
        plan.operation_id = Some(uuid::Uuid::new_v4().to_string());
        let (_, latest, _) = output(&store, &source, "results/plot.svg", "concurrent writer").await;
        let error = store
            .delete_session_impl("session", "source", Some(&plan))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("fresh preview"));
        assert_eq!(
            store
                .get_latest_artifact_version_context(&id)
                .await
                .unwrap()
                .unwrap()
                .version
                .id,
            latest
        );
        assert_eq!(store.load_messages("session").await.unwrap().len(), 2);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn committed_deletion_cleans_read_only_windows_backups() {
        let (_temp, store, source, _) = fixture(false).await;
        let (_, _, snapshot) = output(&store, &source, "results/plot.svg", "generated").await;
        let file = source.join("results/plot.svg");
        for path in [&file, &snapshot] {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(path, permissions).unwrap();
        }
        let preview = store
            .preview_session_artifacts("session", "source", None)
            .await
            .unwrap();
        store
            .operate_session_with_artifacts("session", "source", None, &preview.fingerprint)
            .await
            .unwrap();
        assert!(!file.exists());
        assert!(!snapshot.exists());
        assert!(store
            .session_artifact_recovery_projects("source")
            .await
            .unwrap()
            .is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinks_external_paths_and_directories_are_never_removed() {
        let (_temp, store, source, target) = fixture(false).await;
        let (_id, _version, snapshot) =
            output(&store, &source, "results/plot.svg", "generated").await;
        fs::remove_file(source.join("results/plot.svg")).unwrap();
        fs::write(target.join("valuable.svg"), "external").unwrap();
        std::os::unix::fs::symlink(target.join("valuable.svg"), source.join("results/plot.svg"))
            .unwrap();
        let preview = store
            .preview_session_artifacts("session", "source", None)
            .await
            .unwrap();
        assert!(preview.artifacts.is_empty());
        assert_eq!(preview.retained[0].reason, "unavailable");
        store
            .operate_session_with_artifacts("session", "source", None, &preview.fingerprint)
            .await
            .unwrap();
        assert!(snapshot.exists());
        assert_eq!(
            fs::read_to_string(target.join("valuable.svg")).unwrap(),
            "external"
        );
        assert!(safe_path(&source, "../target/valuable.svg").is_err());
        assert!(safe_path(&source, "results").is_err());
        assert!(safe_path(&source, ".wisp/project.sqlite").is_err());
        assert!(safe_path(&source, "x:stream").is_err());
    }
}
