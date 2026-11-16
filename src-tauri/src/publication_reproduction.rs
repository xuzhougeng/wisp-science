//! Isolated verification for a Frozen Publication revision, from the
//! WebView host.
//!
//! The rerun itself is `wisp_runs::publication_reproduction`, shared with the
//! embedded desktop host; this host lends it the process runner and the
//! freeze module's security scanner.

use crate::run_context::{ProcessRunRunner, RunCommandRunner};
pub(crate) use wisp_runs::publication_reproduction::{
    effective_capability, ReproductionComparisonRequest,
};
use wisp_store::{ReproductionRun, Store};

pub(crate) async fn verify_publication_revision_with_runner(
    store: &Store,
    revision_id: &str,
    source_run_id: &str,
    requests: &[ReproductionComparisonRequest],
    runner: &dyn RunCommandRunner,
) -> Result<ReproductionRun, String> {
    wisp_runs::publication_reproduction::verify_publication_revision_with_runner(
        store,
        revision_id,
        source_run_id,
        requests,
        runner,
        crate::publication_freeze::capsule_security_violations,
    )
    .await
}

pub(crate) async fn verify_publication_revision(
    store: &Store,
    revision_id: &str,
    source_run_id: &str,
    requests: &[ReproductionComparisonRequest],
) -> Result<ReproductionRun, String> {
    verify_publication_revision_with_runner(
        store,
        revision_id,
        source_run_id,
        requests,
        &ProcessRunRunner,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publication_freeze::{
        capsule_security_violations, freeze_publication_revision_in_store,
    };
    use crate::run_context::{
        RunCommand, RunCommandOutput, RunCommandRunner, PUBLICATION_REPRODUCTION_CONTEXT_ID,
    };
    use crate::snapshot_store::{capture_file, SnapshotPolicy};
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use wisp_runs::publication_reproduction::reproduction_environment;
    use wisp_store::{
        ArtifactCaptureTiming, ArtifactMaterialization, ArtifactVersionDraft, EvidenceBindingDraft,
        EvidenceSelectionState, EvidenceSourceKind, EvidenceVisibility, LineageBasis,
        LineageConfidence, PublicationCapabilityLevel, PublicationFreezePolicy, PublicationItem,
        PublicationItemKind, ReproductionComparatorKind, RunCodeSnapshot, RunInput, RunOutput,
        RunRecord, RunStatus,
    };

    struct AssertIsolatedRunner {
        workspace: Arc<Mutex<Option<PathBuf>>>,
    }

    #[async_trait::async_trait]
    impl RunCommandRunner for AssertIsolatedRunner {
        async fn run(
            &self,
            command: RunCommand,
            _timeout: Duration,
        ) -> Result<RunCommandOutput, String> {
            assert_eq!(command.context_id, PUBLICATION_REPRODUCTION_CONTEXT_ID);
            #[cfg(not(target_os = "windows"))]
            assert_eq!(command.args.first().map(String::as_str), Some("-c"));
            let workspace = command.cwd.expect("verification runner needs a workspace");
            assert!(command.envs.iter().all(|(name, _)| matches!(
                name.as_str(),
                "PATH"
                    | "Path"
                    | "PATHEXT"
                    | "SystemRoot"
                    | "WINDIR"
                    | "TEMP"
                    | "TMP"
                    | "TMPDIR"
                    | "LANG"
                    | "LC_ALL"
                    | "TZ"
            )));
            assert!(command
                .envs
                .iter()
                .filter(|(name, _)| matches!(name.as_str(), "TEMP" | "TMP" | "TMPDIR"))
                .all(|(_, value)| Path::new(value).starts_with(&workspace)));
            assert_eq!(
                std::fs::read(workspace.join("data/input.txt")).unwrap(),
                b"declared input\n"
            );
            assert_eq!(
                std::fs::read_to_string(workspace.join("analysis.py")).unwrap(),
                "print('fixture')\n"
            );
            assert!(!workspace.join("undeclared.txt").exists());
            std::fs::create_dir_all(workspace.join("results")).unwrap();
            std::fs::write(workspace.join("results/out.txt"), b"42\n").unwrap();
            *self.workspace.lock().unwrap() = Some(workspace);
            Ok(RunCommandOutput {
                exit_code: 0,
                stdout: "fixture completed".into(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn isolated_command_rejects_credentials_found_by_the_freeze_scanner() {
        let verify = |command: &str| {
            wisp_runs::publication_reproduction::command_is_isolated(
                capsule_security_violations,
                command,
            )
        };
        assert!(verify("python analysis.py"));
        assert!(!verify("python analysis.py --api_key=supersecretvalue"));
        assert!(!verify("python /Users/alice/analysis.py"));
    }

    #[tokio::test]
    async fn frozen_run_verifies_from_only_allowlisted_snapshots() {
        let root = std::env::temp_dir().join(format!(
            "wisp-reproduction-fixture-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::create_dir_all(root.join("results")).unwrap();
        std::fs::write(root.join("data/input.txt"), b"declared input\n").unwrap();
        std::fs::write(root.join("results/out.txt"), b"42\n").unwrap();
        std::fs::write(root.join("undeclared.txt"), b"must stay outside\n").unwrap();
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        store
            .create_project("project", "Project", &root.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("frame", "project", "OPERON", "model")
            .await
            .unwrap();

        let mut run = RunRecord::new("run", "project", "local", "Analysis", "command");
        run.frame_id = Some("frame".into());
        run.command = Some("python analysis.py".into());
        run.status = RunStatus::Succeeded;
        run.exit_code = Some(0);
        run.ended_at = Some(chrono::Utc::now().timestamp());
        store.create_run(&run).await.unwrap();

        let input_snapshot =
            capture_file(&root, &root.join("data/input.txt"), SnapshotPolicy::Always).unwrap();
        let input_version_id = store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: Some("input-version".into()),
                artifact_id: "input-artifact".into(),
                project_id: "project".into(),
                root_frame_id: "frame".into(),
                filename: "input.txt".into(),
                content_type: "text/plain".into(),
                storage_path: input_snapshot.storage_path,
                logical_key: Some("data:input".into()),
                size_bytes: Some(input_snapshot.size_bytes as i64),
                checksum: Some(input_snapshot.checksum),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        let output_snapshot =
            capture_file(&root, &root.join("results/out.txt"), SnapshotPolicy::Always).unwrap();
        let output_version_id = store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: Some("output-version".into()),
                artifact_id: "output-artifact".into(),
                project_id: "project".into(),
                root_frame_id: "frame".into(),
                filename: "out.txt".into(),
                content_type: "text/plain".into(),
                storage_path: output_snapshot.storage_path,
                logical_key: Some("result:out".into()),
                size_bytes: Some(output_snapshot.size_bytes as i64),
                checksum: Some(output_snapshot.checksum),
                producing_run_id: Some("run".into()),
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        store
            .save_run_input(&RunInput {
                id: "input".into(),
                run_id: "run".into(),
                artifact_version_id: Some(input_version_id),
                external_resource_id: None,
                source_ref: "data/input.txt".into(),
                role: "data".into(),
                required: true,
                basis: LineageBasis::Declared,
                confidence: LineageConfidence::Exact,
                created_at: 1,
            })
            .await
            .unwrap();
        store
            .save_run_output(&RunOutput {
                id: "output".into(),
                run_id: "run".into(),
                artifact_version_id: output_version_id.clone(),
                role: "result".into(),
                logical_output_key: "result:out".into(),
                source_path: "results/out.txt".into(),
                created_at: 1,
            })
            .await
            .unwrap();
        let code = "print('fixture')\n";
        store
            .save_run_code_snapshot(&RunCodeSnapshot {
                id: "code".into(),
                run_id: "run".into(),
                source_kind: "script".into(),
                source_path: Some("analysis.py".into()),
                source_text: code.into(),
                checksum: hex::encode(Sha256::digest(code.as_bytes())),
                storage_path: None,
                git_commit: None,
                dirty_patch: None,
                created_at: 1,
            })
            .await
            .unwrap();
        let context = store.get_execution_context("local").await.unwrap().unwrap();
        let (environment_json, environment_hash) = reproduction_environment(&context);
        assert_eq!(
            store
                .record_run_environment_snapshot(
                    "run",
                    Some("local"),
                    &serde_json::from_str(&environment_json).unwrap(),
                )
                .await
                .unwrap(),
            environment_hash
        );

        store
            .create_publication("publication", "project", "Paper", "")
            .await
            .unwrap();
        store
            .create_publication_revision("revision", "publication", None, "Submission")
            .await
            .unwrap();
        store
            .save_publication_item(&PublicationItem {
                id: "methods".into(),
                revision_id: "revision".into(),
                parent_item_id: None,
                kind: PublicationItemKind::Methods,
                title: "Analysis".into(),
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
                id: "run-binding".into(),
                revision_id: "revision".into(),
                item_id: Some("methods".into()),
                source_kind: EvidenceSourceKind::Run,
                source_id: "run".into(),
                purpose: "Analysis command".into(),
                supported_claim_item_id: None,
                selection_state: EvidenceSelectionState::Selected,
                visibility: EvidenceVisibility::Private,
            })
            .await
            .unwrap();
        let frozen = freeze_publication_revision_in_store(
            &store,
            "revision",
            PublicationFreezePolicy {
                target_visibility: EvidenceVisibility::Private,
                phi_pii_reviewed: true,
                redistribution_reviewed: true,
                snapshot_restricted_bytes: true,
            },
        )
        .await
        .unwrap();
        assert!(frozen.frozen, "{:?}", frozen.readiness.blockers);
        let revision_before = frozen.revision.clone();

        let workspace = Arc::new(Mutex::new(None));
        let verification = verify_publication_revision_with_runner(
            &store,
            "revision",
            "run",
            &[],
            &AssertIsolatedRunner {
                workspace: workspace.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            verification.capability_level,
            PublicationCapabilityLevel::Reproduced
        );
        assert!(verification.environment_matched);
        let workspace = workspace.lock().unwrap().clone().unwrap();
        assert!(!workspace.exists());
        let results = store
            .list_reproduction_results(&verification.id)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].passed);
        assert_eq!(
            results[0].comparator_kind,
            ReproductionComparatorKind::Sha256
        );
        let reports = store.list_reproduction_runs("revision").await.unwrap();
        assert_eq!(
            effective_capability(
                revision_before.capability_level,
                revision_before.manifest_json.as_deref(),
                &reports,
            ),
            PublicationCapabilityLevel::Reproduced
        );
        assert_eq!(
            store
                .get_publication_revision("revision")
                .await
                .unwrap()
                .unwrap(),
            revision_before
        );

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
