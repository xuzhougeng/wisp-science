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
    pub max_file_mb: Option<u64>,
    pub max_total_mb: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarvestedArtifact {
    pub artifact_id: String,
    pub path: String,
    pub kind: String,
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
        if is_uri(&spec.glob) {
            out.push(
                register_reference_artifact(
                    store,
                    project_id,
                    root_frame_id,
                    run_id,
                    &spec.kind,
                    &spec.glob,
                    None,
                )
                .await?,
            );
            continue;
        }

        let mut total = 0u64;
        let pattern = base_dir.join(&spec.glob).to_string_lossy().into_owned();
        let paths = glob::glob(&pattern).map_err(|e| e.to_string())?;
        for entry in paths {
            let path = entry.map_err(|e| e.to_string())?;
            if !path.is_file() {
                continue;
            }
            let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
            let max_file = spec.max_file_mb.map(mb_to_bytes).unwrap_or(u64::MAX);
            let max_total = spec.max_total_mb.map(mb_to_bytes).unwrap_or(u64::MAX);
            let as_reference = spec.residency == OutputResidency::Remote
                || size > max_file
                || total + size > max_total;
            total = total.saturating_add(size);
            if as_reference {
                let uri = format!("file://{}", path.to_string_lossy());
                out.push(
                    register_reference_artifact(
                        store,
                        project_id,
                        root_frame_id,
                        run_id,
                        &spec.kind,
                        &uri,
                        Some(size),
                    )
                    .await?,
                );
            } else {
                out.push(
                    register_local_artifact(
                        store,
                        project_id,
                        root_frame_id,
                        run_id,
                        &spec.kind,
                        &path,
                        size,
                    )
                    .await?,
                );
            }
        }
    }
    Ok(out)
}

async fn register_local_artifact(
    store: &wisp_store::Store,
    project_id: &str,
    root_frame_id: &str,
    run_id: &str,
    kind: &str,
    path: &Path,
    size: u64,
) -> Result<HarvestedArtifact, String> {
    let artifact_id = uuid::Uuid::new_v4().to_string();
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact");
    let storage_path = path.to_string_lossy().into_owned();
    let version_id = store
        .save_artifact(
            &artifact_id,
            project_id,
            root_frame_id,
            filename,
            kind,
            &storage_path,
        )
        .await
        .map_err(|e| e.to_string())?;
    store
        .set_artifact_version_provenance(&version_id, Some(run_id), None)
        .await
        .map_err(|e| e.to_string())?;
    link_run_artifact(store, run_id, &artifact_id, kind).await?;
    Ok(HarvestedArtifact {
        artifact_id,
        path: storage_path,
        kind: kind.into(),
        residency: OutputResidency::Local,
        size: Some(size),
    })
}

async fn register_reference_artifact(
    store: &wisp_store::Store,
    project_id: &str,
    root_frame_id: &str,
    run_id: &str,
    kind: &str,
    uri: &str,
    size: Option<u64>,
) -> Result<HarvestedArtifact, String> {
    let artifact_id = uuid::Uuid::new_v4().to_string();
    let filename = uri
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("remote-artifact");
    let version_id = store
        .save_artifact(&artifact_id, project_id, root_frame_id, filename, kind, uri)
        .await
        .map_err(|e| e.to_string())?;
    store
        .set_artifact_version_provenance(&version_id, Some(run_id), None)
        .await
        .map_err(|e| e.to_string())?;
    link_run_artifact(store, run_id, &artifact_id, kind).await?;
    Ok(HarvestedArtifact {
        artifact_id,
        path: uri.into(),
        kind: kind.into(),
        residency: OutputResidency::Remote,
        size,
    })
}

async fn link_run_artifact(
    store: &wisp_store::Store,
    run_id: &str,
    artifact_id: &str,
    role: &str,
) -> Result<(), String> {
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
                max_file_mb: Some(1),
                max_total_mb: Some(1),
            }],
        )
        .await
        .unwrap();

        assert_eq!(harvested.len(), 1);
        assert_eq!(harvested[0].kind, "table");
        assert_eq!(harvested[0].residency, OutputResidency::Local);
        let artifacts = store.list_artifacts("f").await.unwrap();
        assert_eq!(artifacts.len(), 1);
        assert!(std::path::Path::new(&artifacts[0].3)
            .ends_with(std::path::Path::new("results").join("table.tsv")));
        let links = store.list_run_artifacts("r").await.unwrap();
        assert_eq!(
            links,
            vec![(harvested[0].artifact_id.clone(), "table".into())]
        );

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
                max_file_mb: Some(0),
                max_total_mb: None,
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
        assert!(artifact.2.starts_with("file://"), "{artifact:?}");

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
                max_file_mb: None,
                max_total_mb: None,
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
