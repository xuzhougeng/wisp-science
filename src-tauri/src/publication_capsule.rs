//! Building a Publication Evidence Capsule from the WebView host.
//!
//! The capsule itself is built by `wisp_runs::publication_capsule`, shared
//! with the embedded desktop host; this host lends it the freeze module's
//! security scanner and its ZIP writer, and asks where to save it.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use wisp_runs::publication_capsule::{capsule_filename, CapsuleArchive, CapsuleHost};
use wisp_store::{CapsuleBuild, Store};

use crate::AppState;

struct ZipCapsule(zip::ZipWriter<File>);

impl CapsuleArchive for ZipCapsule {
    fn start_file(&mut self, path: &str, size_bytes: u64) -> Result<(), String> {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(6))
            .last_modified_time(zip::DateTime::default())
            .large_file(size_bytes > u32::MAX as u64)
            .unix_permissions(0o644);
        self.0
            .start_file(path, options)
            .map_err(|error| error.to_string())
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.0.write_all(bytes).map_err(|error| error.to_string())
    }

    fn finish(self: Box<Self>) -> Result<File, String> {
        self.0.finish().map_err(|error| error.to_string())
    }
}

fn open_zip(file: File) -> Box<dyn CapsuleArchive> {
    Box::new(ZipCapsule(zip::ZipWriter::new(file)))
}

pub(crate) const CAPSULE_HOST: CapsuleHost = CapsuleHost {
    scan: crate::publication_freeze::capsule_security_violations,
    open: open_zip,
};

pub(crate) async fn build_publication_capsule_to(
    store: &Store,
    revision_id: &str,
    destination: &Path,
) -> Result<CapsuleBuild, String> {
    wisp_runs::publication_capsule::build_publication_capsule_to(
        store,
        revision_id,
        destination,
        CAPSULE_HOST,
    )
    .await
}

#[tauri::command]
pub(crate) async fn build_publication_capsule(
    app: AppHandle,
    state: State<'_, AppState>,
    revision_id: String,
) -> Result<Option<CapsuleBuild>, String> {
    let revision = state
        .store
        .get_publication_revision(&revision_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Publication revision not found".to_string())?;
    let publication = state
        .store
        .get_publication(&revision.publication_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Publication not found".to_string())?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Publication Evidence Capsule", &["zip"])
        .set_file_name(capsule_filename(&publication.title, &revision_id))
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(destination) = receiver
        .await
        .map_err(|error| error.to_string())?
        .map(|path| path.into_path().map_err(|error| error.to_string()))
        .transpose()?
    else {
        return Ok(None);
    };
    build_publication_capsule_to(&state.store, &revision_id, &destination)
        .await
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publication_freeze::{
        capsule_security_violations, freeze_publication_revision_in_store,
    };
    use crate::snapshot_store::{capture_file, SnapshotPolicy};
    use serde_json::Value;
    use std::io::Read;
    use std::path::PathBuf;
    use wisp_runs::publication_capsule::CAPSULE_KIND;
    use wisp_store::{
        ArtifactCaptureTiming, ArtifactMaterialization, ArtifactVersionDraft, EvidenceBindingDraft,
        EvidenceSelectionState, EvidenceSourceKind, EvidenceVisibility, PublicationFreezePolicy,
        PublicationItem, PublicationItemKind,
    };

    struct FrozenFixture {
        root: PathBuf,
        store: Store,
        source: PathBuf,
        snapshot: PathBuf,
        version_id: String,
    }

    async fn frozen_artifact_fixture(name: &str, visibility: EvidenceVisibility) -> FrozenFixture {
        let root =
            std::env::temp_dir().join(format!("wisp_capsule_{name}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("results")).unwrap();
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        store
            .create_project("project", "Project", &root.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("frame", "project", "OPERON", "model")
            .await
            .unwrap();
        let source = root.join("results/supplement.txt");
        std::fs::write(&source, b"stable result\n").unwrap();
        let captured = capture_file(&root, &source, SnapshotPolicy::Always).unwrap();
        let snapshot = root.join(&captured.storage_path);
        let version_id = store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: Some("version-1".into()),
                artifact_id: "artifact".into(),
                project_id: "project".into(),
                root_frame_id: "frame".into(),
                filename: "supplement.txt".into(),
                content_type: "text/plain".into(),
                storage_path: captured.storage_path,
                logical_key: Some("supplement".into()),
                size_bytes: Some(captured.size_bytes as i64),
                checksum: Some(captured.checksum),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        store
            .create_publication("publication", "project", "T cell study", "")
            .await
            .unwrap();
        store
            .create_publication_revision("revision", "publication", None, "Submission")
            .await
            .unwrap();
        store
            .save_publication_item(&PublicationItem {
                id: "item".into(),
                revision_id: "revision".into(),
                parent_item_id: None,
                kind: PublicationItemKind::Supplement,
                title: "Supplement".into(),
                content: String::new(),
                ordinal: 0,
                metadata_json: "{}".into(),
                created_at: 0,
                updated_at: 0,
            })
            .await
            .unwrap();
        store
            .save_evidence_binding(&EvidenceBindingDraft {
                id: "binding".into(),
                revision_id: "revision".into(),
                item_id: Some("item".into()),
                source_kind: EvidenceSourceKind::ArtifactVersion,
                source_id: version_id.clone(),
                purpose: "Published supplement".into(),
                supported_claim_item_id: None,
                selection_state: EvidenceSelectionState::Selected,
                visibility,
            })
            .await
            .unwrap();
        let outcome = freeze_publication_revision_in_store(
            &store,
            "revision",
            PublicationFreezePolicy {
                target_visibility: EvidenceVisibility::Public,
                phi_pii_reviewed: true,
                redistribution_reviewed: true,
                snapshot_restricted_bytes: false,
            },
        )
        .await
        .unwrap();
        assert!(outcome.frozen);
        FrozenFixture {
            root,
            store,
            source,
            snapshot,
            version_id,
        }
    }

    fn zip_names(path: &Path) -> Vec<String> {
        let input = File::open(path).unwrap();
        let mut archive = zip::ZipArchive::new(input).unwrap();
        (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_string())
            .collect()
    }

    fn zip_text(path: &Path, name: &str) -> String {
        let input = File::open(path).unwrap();
        let mut archive = zip::ZipArchive::new(input).unwrap();
        let mut text = String::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    #[test]
    fn capsule_security_rejects_credentials_and_public_machine_details() {
        assert_eq!(
            capsule_security_violations("api_key=supersecretvalue", false),
            vec!["credential_pattern"]
        );
        assert_eq!(
            capsule_security_violations("python /tmp/private/analysis.py", true),
            vec!["machine_local_detail"]
        );
        assert!(capsule_security_violations("https://example.org/data/v1", true).is_empty());
    }

    #[tokio::test]
    async fn frozen_capsules_are_byte_deterministic_and_ignore_live_file_changes() {
        let fixture = frozen_artifact_fixture("deterministic", EvidenceVisibility::Public).await;
        std::fs::write(&fixture.source, b"changed live file\n").unwrap();
        let first_path = fixture.root.join("capsule-one.zip");
        let second_path = fixture.root.join("capsule-two.zip");
        let first = build_publication_capsule_to(&fixture.store, "revision", &first_path)
            .await
            .unwrap();
        let second = build_publication_capsule_to(&fixture.store, "revision", &second_path)
            .await
            .unwrap();
        assert_eq!(first.status, "succeeded");
        assert_eq!(first.archive_sha256, second.archive_sha256);
        assert_eq!(
            std::fs::read(&first_path).unwrap(),
            std::fs::read(&second_path).unwrap()
        );

        let names = zip_names(&first_path);
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        let mut archive = zip::ZipArchive::new(File::open(&first_path).unwrap()).unwrap();
        for index in 0..archive.len() {
            let entry = archive.by_index(index).unwrap();
            assert_eq!(entry.last_modified(), Some(zip::DateTime::default()));
            assert_eq!(entry.unix_mode().map(|mode| mode & 0o777), Some(0o644));
        }
        for required in [
            "capsule.json",
            "README.md",
            "REPRODUCE.md",
            "CITATION.cff",
            "checksums.sha256",
            "data/access.json",
            "evidence/manifest.json",
            "evidence/selection.json",
            "provenance/lineage.json",
            "reference-results/manifest.json",
            "verification/report.json",
        ] {
            assert!(names.iter().any(|name| name == required), "{required}");
        }
        let artifact_path = names
            .iter()
            .find(|name| name.starts_with("evidence/version-1-"))
            .unwrap();
        assert_eq!(zip_text(&first_path, artifact_path), "stable result\n");
        let capsule: Value = serde_json::from_str(&zip_text(&first_path, "capsule.json")).unwrap();
        assert_eq!(
            capsule.get("capsule_kind").and_then(Value::as_str),
            Some(CAPSULE_KIND)
        );
        assert_eq!(
            capsule
                .get("revision_manifest_sha256")
                .and_then(Value::as_str),
            Some(first.revision_manifest_sha256.as_str())
        );
        let checksums = zip_text(&first_path, "checksums.sha256");
        assert!(checksums.contains("  capsule.json\n"));
        assert!(!checksums.contains("  checksums.sha256\n"));

        let builds = fixture.store.list_capsule_builds("revision").await.unwrap();
        assert_eq!(builds.len(), 2);
        assert!(builds.iter().all(|build| build.status == "succeeded"));
        drop(fixture.store);
        let _ = std::fs::remove_dir_all(fixture.root);
    }

    #[tokio::test]
    async fn public_capsule_omits_restricted_bytes_and_never_reads_them() {
        let fixture = frozen_artifact_fixture("restricted", EvidenceVisibility::Restricted).await;
        std::fs::remove_file(&fixture.snapshot).unwrap();
        let destination = fixture.root.join("public.zip");
        let build = build_publication_capsule_to(&fixture.store, "revision", &destination)
            .await
            .unwrap();
        assert_eq!(build.status, "succeeded");
        let names = zip_names(&destination);
        assert!(!names.iter().any(|name| name.contains(&fixture.version_id)));
        let access: Value =
            serde_json::from_str(&zip_text(&destination, "data/access.json")).unwrap();
        assert!(access["reference_only_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["source_id"] == fixture.version_id));
        drop(fixture.store);
        let _ = std::fs::remove_dir_all(fixture.root);
    }

    #[tokio::test]
    async fn checksum_mismatch_fails_without_publishing_an_archive() {
        let fixture = frozen_artifact_fixture("corrupt", EvidenceVisibility::Public).await;
        std::fs::write(&fixture.snapshot, b"result stable\n").unwrap();
        let destination = fixture.root.join("corrupt.zip");
        let error = build_publication_capsule_to(&fixture.store, "revision", &destination)
            .await
            .unwrap_err();
        assert!(error.contains("checksum mismatch"), "{error}");
        assert!(!destination.exists());
        let builds = fixture.store.list_capsule_builds("revision").await.unwrap();
        assert_eq!(builds.len(), 1);
        assert_eq!(builds[0].status, "failed");
        assert!(builds[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("checksum mismatch")));
        assert!(!fixture.root.read_dir().unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        drop(fixture.store);
        let _ = std::fs::remove_dir_all(fixture.root);
    }

    #[tokio::test]
    async fn missing_public_snapshot_fails_closed() {
        let fixture = frozen_artifact_fixture("missing", EvidenceVisibility::Public).await;
        std::fs::remove_file(&fixture.snapshot).unwrap();
        let destination = fixture.root.join("missing.zip");
        let error = build_publication_capsule_to(&fixture.store, "revision", &destination)
            .await
            .unwrap_err();
        assert!(error.contains("unavailable"), "{error}");
        assert!(!destination.exists());
        assert_eq!(
            fixture.store.list_capsule_builds("revision").await.unwrap()[0].status,
            "failed"
        );
        drop(fixture.store);
        let _ = std::fs::remove_dir_all(fixture.root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn capsule_builder_rejects_symlinked_snapshots() {
        use std::os::unix::fs::symlink;

        let fixture = frozen_artifact_fixture("symlink", EvidenceVisibility::Public).await;
        let other = fixture.root.join("other.txt");
        std::fs::write(&other, b"stable result\n").unwrap();
        std::fs::remove_file(&fixture.snapshot).unwrap();
        symlink(&other, &fixture.snapshot).unwrap();
        let destination = fixture.root.join("symlink.zip");
        let error = build_publication_capsule_to(&fixture.store, "revision", &destination)
            .await
            .unwrap_err();
        assert!(error.contains("symlink"), "{error}");
        assert!(!destination.exists());
        drop(fixture.store);
        let _ = std::fs::remove_dir_all(fixture.root);
    }
}
