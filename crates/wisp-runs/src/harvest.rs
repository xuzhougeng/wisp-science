use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputResidency {
    Local,
    Remote,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSpec {
    pub glob: String,
    pub kind: String,
    pub residency: OutputResidency,
    #[serde(default)]
    pub logical_key: Option<String>,
    pub max_file_mb: Option<u64>,
    pub max_total_mb: Option<u64>,
    /// Pack every matched file into one tar.gz archive registered as a single
    /// ArtifactVersion. Required for massive many-file outputs so registration
    /// never scales with the remote file count.
    #[serde(default)]
    pub bundle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarvestedArtifact {
    pub artifact_id: String,
    pub artifact_version_id: String,
    pub path: String,
    pub kind: String,
    pub logical_output_key: String,
    pub checksum: Option<String>,
    pub residency: OutputResidency,
    pub size: Option<u64>,
}

pub async fn harvest_run_outputs(
    store: &wisp_store::Store,
    project_id: &str,
    root_frame_id: &str,
    run_id: &str,
    base_dir: &Path,
    specs: &[OutputSpec],
) -> Result<Vec<HarvestedArtifact>, String> {
    let mut out = Vec::new();
    for spec in specs {
        if spec
            .logical_key
            .as_deref()
            .is_some_and(|key| key.trim().is_empty())
        {
            return Err("output logical_key cannot be empty".into());
        }
        if is_uri(&spec.glob) {
            let logical_key = spec
                .logical_key
                .clone()
                .unwrap_or_else(|| format!("uri:{}", spec.glob));
            out.push(
                register_reference_artifact(
                    store,
                    project_id,
                    root_frame_id,
                    run_id,
                    &spec.kind,
                    &spec.glob,
                    None,
                    None,
                    &logical_key,
                )
                .await?,
            );
            continue;
        }

        let mut total = 0u64;
        let pattern = base_dir.join(&spec.glob).to_string_lossy().into_owned();
        let paths = glob::glob(&pattern)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        if spec.logical_key.is_some() && paths.len() > 1 {
            return Err(format!(
                "output logical_key '{}' matched more than one file",
                spec.logical_key.as_deref().unwrap_or_default()
            ));
        }
        for path in paths {
            let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
            let max_file = spec.max_file_mb.map(mb_to_bytes).unwrap_or_else(|| {
                if spec.residency == OutputResidency::Auto {
                    crate::snapshot_store::DEFAULT_SNAPSHOT_LIMIT
                } else {
                    u64::MAX
                }
            });
            let max_total = spec.max_total_mb.map(mb_to_bytes).unwrap_or(u64::MAX);
            let as_reference = spec.residency == OutputResidency::Remote
                || size > max_file
                || total + size > max_total;
            total = total.saturating_add(size);
            let source_path = path
                .strip_prefix(base_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let logical_key = spec
                .logical_key
                .clone()
                .unwrap_or_else(|| format!("path:{source_path}"));
            out.push(
                register_local_artifact(
                    store,
                    project_id,
                    root_frame_id,
                    run_id,
                    &spec.kind,
                    base_dir,
                    &path,
                    &logical_key,
                    &source_path,
                    as_reference,
                )
                .await?,
            );
        }
    }
    Ok(out)
}

/// A retried harvest must not append duplicate versions or lineage rows for
/// an output that an earlier attempt already registered for the same Run.
async fn already_registered(
    store: &wisp_store::Store,
    artifact_id: &str,
    run_id: &str,
    logical_key: &str,
    checksum: Option<&str>,
    storage_path: &str,
) -> Result<Option<String>, String> {
    let Some(version) = store
        .latest_artifact_version(artifact_id)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let same_content = match checksum {
        Some(checksum) => version.checksum.as_deref() == Some(checksum),
        None => version.storage_path == storage_path,
    };
    if !same_content || version.producing_run_id.as_deref() != Some(run_id) {
        return Ok(None);
    }
    let linked = store
        .list_run_outputs(run_id)
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .any(|output| output.logical_output_key == logical_key);
    Ok(linked.then_some(version.id))
}

#[allow(clippy::too_many_arguments)]
pub async fn register_local_artifact(
    store: &wisp_store::Store,
    project_id: &str,
    root_frame_id: &str,
    run_id: &str,
    kind: &str,
    base_dir: &Path,
    path: &Path,
    logical_key: &str,
    source_path: &str,
    as_reference: bool,
) -> Result<HarvestedArtifact, String> {
    let artifact_id = wisp_store::logical_artifact_id(project_id, logical_key);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact");
    let captured = crate::snapshot_store::capture_file(
        base_dir,
        path,
        if as_reference {
            crate::snapshot_store::SnapshotPolicy::Reference
        } else {
            crate::snapshot_store::SnapshotPolicy::Always
        },
    )?;
    if let Some(version_id) = already_registered(
        store,
        &artifact_id,
        run_id,
        logical_key,
        Some(&captured.checksum),
        &captured.storage_path,
    )
    .await?
    {
        return Ok(HarvestedArtifact {
            artifact_id,
            artifact_version_id: version_id,
            path: captured.storage_path,
            kind: kind.into(),
            logical_output_key: logical_key.into(),
            checksum: Some(captured.checksum),
            residency: if as_reference {
                OutputResidency::Remote
            } else {
                OutputResidency::Local
            },
            size: Some(captured.size_bytes),
        });
    }
    let size_bytes =
        i64::try_from(captured.size_bytes).map_err(|_| "artifact is too large".to_string())?;
    let env_snapshot_hash = store
        .get_run_environment_snapshot(run_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|snapshot| snapshot.hash);
    let version_id = store
        .save_artifact_version(&wisp_store::ArtifactVersionDraft {
            version_id: None,
            artifact_id: artifact_id.clone(),
            project_id: project_id.to_string(),
            root_frame_id: root_frame_id.to_string(),
            filename: filename.to_string(),
            content_type: kind.to_string(),
            storage_path: captured.storage_path.clone(),
            logical_key: Some(logical_key.to_string()),
            size_bytes: Some(size_bytes),
            checksum: Some(captured.checksum.clone()),
            producing_run_id: Some(run_id.to_string()),
            env_snapshot_hash,
            materialization: captured.materialization,
            capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
        })
        .await
        .map_err(|e| e.to_string())?;
    link_run_artifact(
        store,
        run_id,
        &artifact_id,
        &version_id,
        kind,
        logical_key,
        source_path,
    )
    .await?;
    Ok(HarvestedArtifact {
        artifact_id,
        artifact_version_id: version_id,
        path: captured.storage_path,
        kind: kind.into(),
        logical_output_key: logical_key.into(),
        checksum: Some(captured.checksum),
        residency: if as_reference {
            OutputResidency::Remote
        } else {
            OutputResidency::Local
        },
        size: Some(captured.size_bytes),
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn register_reference_artifact(
    store: &wisp_store::Store,
    project_id: &str,
    root_frame_id: &str,
    run_id: &str,
    kind: &str,
    uri: &str,
    size: Option<u64>,
    checksum: Option<String>,
    logical_key: &str,
) -> Result<HarvestedArtifact, String> {
    let artifact_id = wisp_store::logical_artifact_id(project_id, logical_key);
    if let Some(version_id) = already_registered(
        store,
        &artifact_id,
        run_id,
        logical_key,
        checksum.as_deref(),
        uri,
    )
    .await?
    {
        return Ok(HarvestedArtifact {
            artifact_id,
            artifact_version_id: version_id,
            path: uri.into(),
            kind: kind.into(),
            logical_output_key: logical_key.into(),
            checksum,
            residency: OutputResidency::Remote,
            size,
        });
    }
    let filename = uri
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("remote-artifact");
    let size_bytes = size
        .map(i64::try_from)
        .transpose()
        .map_err(|_| "artifact is too large".to_string())?;
    let env_snapshot_hash = store
        .get_run_environment_snapshot(run_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|snapshot| snapshot.hash);
    let version_id = store
        .save_artifact_version(&wisp_store::ArtifactVersionDraft {
            version_id: None,
            artifact_id: artifact_id.clone(),
            project_id: project_id.to_string(),
            root_frame_id: root_frame_id.to_string(),
            filename: filename.to_string(),
            content_type: kind.to_string(),
            storage_path: uri.to_string(),
            logical_key: Some(logical_key.to_string()),
            size_bytes,
            checksum: checksum.clone(),
            producing_run_id: Some(run_id.to_string()),
            env_snapshot_hash,
            materialization: wisp_store::ArtifactMaterialization::External,
            capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
        })
        .await
        .map_err(|e| e.to_string())?;
    link_run_artifact(
        store,
        run_id,
        &artifact_id,
        &version_id,
        kind,
        logical_key,
        uri,
    )
    .await?;
    Ok(HarvestedArtifact {
        artifact_id,
        artifact_version_id: version_id,
        path: uri.into(),
        kind: kind.into(),
        logical_output_key: logical_key.into(),
        checksum,
        residency: OutputResidency::Remote,
        size,
    })
}

async fn link_run_artifact(
    store: &wisp_store::Store,
    run_id: &str,
    artifact_id: &str,
    artifact_version_id: &str,
    role: &str,
    logical_output_key: &str,
    source_path: &str,
) -> Result<(), String> {
    store
        .save_run_output(&wisp_store::RunOutput {
            id: uuid::Uuid::new_v4().to_string(),
            run_id: run_id.to_string(),
            artifact_version_id: artifact_version_id.to_string(),
            role: role.to_string(),
            logical_output_key: logical_output_key.to_string(),
            source_path: source_path.to_string(),
            created_at: chrono::Utc::now().timestamp(),
        })
        .await
        .map_err(|e| e.to_string())?;
    for input in store
        .list_run_inputs(run_id)
        .await
        .map_err(|e| e.to_string())?
    {
        let Some(input_version_id) = input.artifact_version_id else {
            continue;
        };
        store
            .save_artifact_dependency(
                &uuid::Uuid::new_v4().to_string(),
                artifact_version_id,
                &input_version_id,
                Some(&input.role),
                input.basis,
                input.confidence,
            )
            .await
            .map_err(|e| e.to_string())?;
    }
    store
        .save_run_artifact_link(&uuid::Uuid::new_v4().to_string(), run_id, artifact_id, role)
        .await
        .map_err(|e| e.to_string())
}

fn is_uri(s: &str) -> bool {
    s.contains("://")
}

fn mb_to_bytes(mb: u64) -> u64 {
    mb.saturating_mul(1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn harvest_registers_small_local_file_and_run_link() {
        let tmp = std::env::temp_dir().join(format!("wisp_harvest_small_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(tmp.join("results")).unwrap();
        std::fs::write(tmp.join("results/table.tsv"), b"a\tb\n1\t2\n").unwrap();
        let db = tmp.join("wisp.sqlite");
        let store = wisp_store::Store::open(&db).await.unwrap();
        seed_run(&store).await;

        let harvested = harvest_run_outputs(
            &store,
            "p",
            "f",
            "r",
            &tmp,
            &[OutputSpec {
                glob: "results/*.tsv".into(),
                kind: "table".into(),
                residency: OutputResidency::Auto,
                logical_key: None,
                max_file_mb: Some(1),
                max_total_mb: Some(1),
                bundle: false,
            }],
        )
        .await
        .unwrap();

        assert_eq!(harvested.len(), 1);
        assert_eq!(harvested[0].kind, "table");
        assert_eq!(harvested[0].residency, OutputResidency::Local);
        let artifacts = store.list_artifacts("f").await.unwrap();
        assert_eq!(artifacts.len(), 1);
        assert!(artifacts[0].3.contains(".wisp/artifacts/sha256"));
        let outputs = store.list_run_outputs("r").await.unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(
            outputs[0].artifact_version_id,
            harvested[0].artifact_version_id
        );
        assert_eq!(outputs[0].logical_output_key, "path:results/table.tsv");
        let version = store
            .get_artifact_version(&outputs[0].artifact_version_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            version.materialization,
            wisp_store::ArtifactMaterialization::Snapshot
        );
        assert!(version.checksum.is_some());
        let graph = store.research_graph("p").await.unwrap();
        assert!(graph.edges.iter().any(|edge| {
            edge.source_id == "run:r"
                && edge.target_id == format!("artifact:{}", harvested[0].artifact_id)
                && edge.relation == "produced"
        }));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn harvest_oversized_local_file_as_reference() {
        let tmp = std::env::temp_dir().join(format!("wisp_harvest_large_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(tmp.join("results")).unwrap();
        std::fs::write(tmp.join("results/big.tsv"), vec![b'x'; 1024]).unwrap();
        let db = tmp.join("wisp.sqlite");
        let store = wisp_store::Store::open(&db).await.unwrap();
        seed_run(&store).await;

        let harvested = harvest_run_outputs(
            &store,
            "p",
            "f",
            "r",
            &tmp,
            &[OutputSpec {
                glob: "results/*.tsv".into(),
                kind: "table".into(),
                residency: OutputResidency::Auto,
                logical_key: None,
                max_file_mb: Some(0),
                max_total_mb: None,
                bundle: false,
            }],
        )
        .await
        .unwrap();

        assert_eq!(harvested[0].residency, OutputResidency::Remote);
        let artifact = store
            .get_artifact(&harvested[0].artifact_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(artifact.2, "results/big.tsv", "{artifact:?}");
        let version = store
            .get_artifact_version(&harvested[0].artifact_version_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            version.materialization,
            wisp_store::ArtifactMaterialization::Reference
        );
        assert!(version.checksum.is_some());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn harvest_registers_remote_uri_reference() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_harvest_remote_{}", uuid::Uuid::new_v4()));
        let db = tmp.join("wisp.sqlite");
        let store = wisp_store::Store::open(&db).await.unwrap();
        seed_run(&store).await;

        let harvested = harvest_run_outputs(
            &store,
            "p",
            "f",
            "r",
            &tmp,
            &[OutputSpec {
                glob: "ssh://gpu-box/scratch/out.bam".into(),
                kind: "data".into(),
                residency: OutputResidency::Remote,
                logical_key: None,
                max_file_mb: None,
                max_total_mb: None,
                bundle: false,
            }],
        )
        .await
        .unwrap();

        assert_eq!(harvested.len(), 1);
        assert_eq!(harvested[0].path, "ssh://gpu-box/scratch/out.bam");
        assert_eq!(harvested[0].residency, OutputResidency::Remote);
        let artifact = store
            .get_artifact(&harvested[0].artifact_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(artifact.2, "ssh://gpu-box/scratch/out.bam");
        let version = store
            .get_artifact_version(&harvested[0].artifact_version_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            version.materialization,
            wisp_store::ArtifactMaterialization::External
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn logical_output_key_forms_one_artifact_version_chain() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_harvest_logical_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(tmp.join("results")).unwrap();
        std::fs::write(tmp.join("results/figure.png"), b"figure-v1").unwrap();
        let store = wisp_store::Store::open(&tmp.join("wisp.sqlite"))
            .await
            .unwrap();
        seed_run(&store).await;
        let spec = OutputSpec {
            glob: "results/figure.png".into(),
            kind: "image/png".into(),
            residency: OutputResidency::Auto,
            logical_key: Some("figure:t-cell-exhaustion".into()),
            max_file_mb: Some(1),
            max_total_mb: Some(1),
            bundle: false,
        };
        let first = harvest_run_outputs(&store, "p", "f", "r", &tmp, &[spec.clone()])
            .await
            .unwrap();

        store
            .create_run(&wisp_store::RunRecord::new(
                "r2", "p", "local", "Run 2", "command",
            ))
            .await
            .unwrap();
        std::fs::write(tmp.join("results/figure.png"), b"figure-v2").unwrap();
        let second = harvest_run_outputs(&store, "p", "f", "r2", &tmp, &[spec])
            .await
            .unwrap();

        assert_eq!(first[0].artifact_id, second[0].artifact_id);
        assert_ne!(first[0].artifact_version_id, second[0].artifact_version_id);
        let latest = store
            .get_artifact_version(&second[0].artifact_version_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            latest.parent_version_id.as_deref(),
            Some(first[0].artifact_version_id.as_str())
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    async fn seed_run(store: &wisp_store::Store) {
        store.create_project("p", "proj", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store
            .upsert_execution_context(&wisp_store::ExecutionContext::new("local", "Local").unwrap())
            .await
            .unwrap();
        store
            .create_run(&wisp_store::RunRecord::new(
                "r", "p", "local", "Run", "command",
            ))
            .await
            .unwrap();
    }
}
