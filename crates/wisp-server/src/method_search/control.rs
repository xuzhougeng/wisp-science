//! A method search's Run from the window, a port of the WebView's
//! `get_method_search_run`, `start_method_search`, `pause_method_search`,
//! `resume_method_search` and `cancel_method_search` commands
//! (`src-tauri/src/method_search.rs`). A search waits in Draft until the
//! researcher has read its frozen contract and pressed Start; it can then be
//! paused at a candidate boundary, resumed from its checkpoint, or
//! cancelled.

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use serde::Serialize;
use wisp_core::method_search::MethodSearchSpec;
use wisp_store::{
    AgentWorkflowRunActivity, MethodCandidate, MethodSearchRunState, MethodStrategyStat, RunOutput,
    RunRecord, RunStatus, StateScope,
};

use super::coordinator::{
    self, ActiveMethodSearchGuard, CandidateGenerator, CandidateGeneratorSource,
    CoordinatorOutcome, ProviderCandidateGenerator,
};
use super::prepare::{MethodSearchAuditReport, MethodSearchEvaluator};
use crate::Server;

/// The most a candidate proposal may write, as in the WebView.
const CANDIDATE_MAX_TOKENS: u64 = 8_192;

/// The most of one promoted output the card reads.
const OUTPUT_MAX_BYTES: usize = 512 * 1024;

/// A method search as its card shows it: the Run, its durable state, the
/// frozen contract and audit, its candidates, strategies and outputs, and
/// the batch step that started it, if one did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodSearchRunDetails {
    pub run: RunRecord,
    pub state: MethodSearchRunState,
    pub spec: MethodSearchSpec,
    pub audit: MethodSearchAuditReport,
    pub audit_artifact_version_id: String,
    pub candidates: Vec<MethodCandidate>,
    pub strategies: Vec<MethodStrategyStat>,
    pub outputs: Vec<RunOutput>,
    pub activity: Option<AgentWorkflowRunActivity>,
}

/// The model a frozen contract names, from this host's settings, for the
/// conversation that started the search: `worker` is the delegation worker
/// (the configured sub-agent model, or the conversation's own),
/// `conversation` the conversation's own, and anything else a model
/// profile by its id.
pub(crate) struct HostGenerators {
    pub(crate) server: Server,
    pub(crate) frame: Option<String>,
}

#[async_trait]
impl CandidateGeneratorSource for HostGenerators {
    async fn generator(
        &self,
        model_profile_id: &str,
        _run_id: &str,
    ) -> Result<Arc<dyn CandidateGenerator>, String> {
        let provider = self
            .server
            .method_search_provider(model_profile_id, self.frame.as_deref())
            .await
            .map_err(|error| format!("{error:#}"))?;
        Ok(Arc::new(ProviderCandidateGenerator::new(Arc::from(
            provider,
        ))))
    }
}

impl Server {
    /// The provider candidates are proposed with; see [`HostGenerators`].
    pub(crate) async fn method_search_provider(
        &self,
        model_profile_id: &str,
        frame: Option<&str>,
    ) -> Result<Box<dyn wisp_llm::Provider>> {
        let store = &self.0.store;
        let own = match frame {
            Some(frame) => store
                .get_setting(&crate::desktop::model_choice_key(frame))
                .await?
                .unwrap_or_default(),
            None => String::new(),
        };
        let choice = match model_profile_id {
            "worker" => {
                let configured = store
                    .get_setting(crate::desktop::SUBAGENT_MODEL)
                    .await?
                    .unwrap_or_default();
                if configured.trim().is_empty() {
                    own
                } else {
                    configured
                }
            }
            "conversation" => own,
            profile => profile.to_owned(),
        };
        let mut model = self.0.model.read().unwrap().pick(&choice)?;
        self.fresh_subscription(&mut model, &self.provider_of(&choice))
            .await?;
        let mut provider = model.provider;
        let routes = crate::network::load(store).await;
        if let Some(route) = crate::network::route(&routes.model_proxy_url) {
            provider.proxy = Some(route);
        }
        provider.max_tokens = match provider.max_tokens {
            0 => CANDIDATE_MAX_TOKENS,
            cap => cap.min(CANDIDATE_MAX_TOKENS),
        };
        Ok((self.0.models)(provider))
    }

    pub(crate) fn method_search_evaluator(&self) -> Arc<dyn MethodSearchEvaluator> {
        self.0
            .method_search_evaluator
            .read()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Arc::new(coordinator::local_evaluator()))
    }

    /// The revision of this computer's execution context, which a method
    /// search step is sealed with: a search refuses to run on a context
    /// changed since it was approved.
    pub(crate) async fn local_context_revision(&self) -> Option<String> {
        let context = self
            .0
            .store
            .get_execution_context(wisp_runtime::LOCAL_CONTEXT_ID)
            .await
            .ok()
            .flatten()?;
        let value = serde_json::to_value(&context).ok()?;
        Some(wisp_store::canonical_json_sha256(&value).1)
    }

    /// Lets tests evaluate candidates without the local Python.
    #[cfg(test)]
    pub(crate) fn set_method_search_evaluator(&self, evaluator: Arc<dyn MethodSearchEvaluator>) {
        *self.0.method_search_evaluator.write().unwrap() = Some(evaluator);
    }

    /// A method search of `project` as its card shows it.
    pub async fn method_search_run(
        &self,
        project: &str,
        run: &str,
    ) -> Result<MethodSearchRunDetails> {
        let root = self.project_root(project).await?;
        self.method_search_details(Path::new(&root), project, run)
            .await
    }

    /// One file the search promoted (a method's source, its history or the
    /// verification report), as text for its card. Only the run's own
    /// outputs are read, from their checksummed snapshots.
    pub async fn method_search_output(
        &self,
        project: &str,
        run_id: &str,
        version_id: &str,
    ) -> Result<String> {
        let root = self.project_root(project).await?;
        let store = &self.0.store;
        let run = store
            .get_run(run_id)
            .await?
            .context("Method-search Run does not exist")?;
        anyhow::ensure!(
            run.project_id == project && run.kind == "method_search",
            "Method-search Run does not belong to this project"
        );
        anyhow::ensure!(
            store
                .list_run_outputs(run_id)
                .await?
                .iter()
                .any(|output| output.artifact_version_id == version_id),
            "This file is not one of the search's outputs"
        );
        let (_, bytes) = coordinator::load_artifact_bytes(
            store,
            Path::new(&root),
            project,
            version_id,
            OUTPUT_MAX_BYTES,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        String::from_utf8(bytes).context("The output is not text")
    }

    async fn method_search_details(
        &self,
        root: &Path,
        project: &str,
        run_id: &str,
    ) -> Result<MethodSearchRunDetails> {
        let store = &self.0.store;
        let run = store
            .get_run(run_id)
            .await?
            .context("Method-search Run does not exist")?;
        anyhow::ensure!(
            run.project_id == project && run.kind == "method_search",
            "Method-search Run does not belong to this project"
        );
        let state = store
            .get_method_search_run_state(run_id)
            .await?
            .context("Method-search Run has no durable state")?;
        let (spec, audit, audit_artifact_version_id) =
            coordinator::load_method_search_review_contract(store, root, project, &state)
                .await
                .map_err(anyhow::Error::msg)?;
        Ok(MethodSearchRunDetails {
            run,
            state,
            spec,
            audit,
            audit_artifact_version_id,
            candidates: store.list_method_candidates(run_id).await?,
            strategies: store.list_method_strategy_stats(run_id).await?,
            outputs: store.list_run_outputs(run_id).await?,
            activity: store.get_agent_workflow_run_activity_by_run(run_id).await?,
        })
    }

    /// The project's root, held for an action on one of its searches: a
    /// project being moved or deleted refuses it.
    async fn method_search_project(
        &self,
        project: &str,
    ) -> Result<(String, tokio::sync::OwnedRwLockReadGuard<()>)> {
        let root = self.project_root(project).await?;
        let activity = self
            .0
            .project_locks
            .project(project)
            .try_read_owned()
            .map_err(|_| anyhow!("The project is busy; try again in a moment"))?;
        Ok((root, activity))
    }

    /// Start and resume work in the mainline only, and only where it can be
    /// written, as in the WebView.
    async fn method_search_scope(&self, run: &RunRecord) -> Result<()> {
        let Some(frame) = run.frame_id.as_deref() else {
            return Ok(());
        };
        let (_, scope) = crate::explorations::service::working_project_in(
            &self.0.store,
            &self.0.config.data,
            frame,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            !matches!(scope, StateScope::Exploration { .. }),
            "A method search cannot run inside an exploration"
        );
        crate::explorations::service::require_writable_scope(&self.0.store, &scope)
            .await
            .map_err(anyhow::Error::msg)
    }

    /// Starts a search that waits in Draft, after its contract was read.
    pub async fn start_method_search(
        &self,
        project: &str,
        run_id: &str,
    ) -> Result<MethodSearchRunDetails> {
        let (root, _activity) = self.method_search_project(project).await?;
        let root = Path::new(&root);
        let details = self.method_search_details(root, project, run_id).await?;
        self.method_search_scope(&details.run).await?;
        let profile =
            coordinator::validate_method_search_start(&self.0.store, root, project, run_id)
                .await
                .map_err(anyhow::Error::msg)?;
        // The model is found before leaving Draft, so a start that fails
        // uses up nothing of the search.
        let generators = HostGenerators {
            server: self.clone(),
            frame: details.run.frame_id.clone(),
        };
        let generator = generators
            .generator(&profile, run_id)
            .await
            .map_err(anyhow::Error::msg)?;
        if coordinator::method_search_is_active(run_id) {
            // A batch step's driver is waiting on it; it runs the search.
            anyhow::ensure!(
                self.0.store.submit_method_search_run(run_id).await?,
                "Method-search Run could not start"
            );
            return self.method_search_details(root, project, run_id).await;
        }
        let guard = ActiveMethodSearchGuard::claim(run_id).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            self.0.store.submit_method_search_run(run_id).await?,
            "Method-search Run could not start"
        );
        self.spawn_method_search(root, run_id, guard, generator);
        self.method_search_details(root, project, run_id).await
    }

    /// Asks a running search to pause at its next candidate boundary.
    pub async fn pause_method_search(
        &self,
        project: &str,
        run_id: &str,
    ) -> Result<MethodSearchRunDetails> {
        let (root, _activity) = self.method_search_project(project).await?;
        let root = Path::new(&root);
        self.method_search_details(root, project, run_id).await?;
        anyhow::ensure!(
            self.0.store.request_method_search_pause(run_id).await?,
            "Method-search Run is not running or already has a pause request"
        );
        self.method_search_details(root, project, run_id).await
    }

    /// Resumes a paused search from its checkpoint.
    pub async fn resume_method_search(
        &self,
        project: &str,
        run_id: &str,
    ) -> Result<MethodSearchRunDetails> {
        let (root, _activity) = self.method_search_project(project).await?;
        let root = Path::new(&root);
        let details = self.method_search_details(root, project, run_id).await?;
        self.method_search_scope(&details.run).await?;
        let profile =
            coordinator::validate_method_search_resume(&self.0.store, root, project, run_id)
                .await
                .map_err(anyhow::Error::msg)?;
        if coordinator::method_search_is_active(run_id) {
            anyhow::ensure!(
                self.0.store.resume_method_search_run(run_id).await?,
                "Method-search Run could not resume"
            );
            return self.method_search_details(root, project, run_id).await;
        }
        let generators = HostGenerators {
            server: self.clone(),
            frame: details.run.frame_id.clone(),
        };
        let generator = generators
            .generator(&profile, run_id)
            .await
            .map_err(anyhow::Error::msg)?;
        let guard = ActiveMethodSearchGuard::claim(run_id).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            self.0.store.resume_method_search_run(run_id).await?,
            "Method-search Run could not resume"
        );
        self.spawn_method_search(root, run_id, guard, generator);
        self.method_search_details(root, project, run_id).await
    }

    /// Cancels a search; one waiting in Draft, or paused with nothing
    /// running it, ends at once.
    pub async fn cancel_method_search(
        &self,
        project: &str,
        run_id: &str,
    ) -> Result<MethodSearchRunDetails> {
        let (root, _activity) = self.method_search_project(project).await?;
        let root = Path::new(&root);
        let details = self.method_search_details(root, project, run_id).await?;
        if details.run.status.is_terminal() {
            return Ok(details);
        }
        anyhow::ensure!(
            self.0.store.request_run_cancellation(run_id).await?,
            "Method-search Run could not request cancellation"
        );
        if matches!(details.run.status, RunStatus::Draft | RunStatus::Paused)
            && !coordinator::method_search_is_active(run_id)
        {
            let guard = ActiveMethodSearchGuard::claim(run_id).map_err(anyhow::Error::msg)?;
            let server = self.clone();
            let run = run_id.to_owned();
            tokio::spawn(async move {
                let _guard = guard;
                let store = &server.0.store;
                if let Err(error) = coordinator::finish_cancelling_method_search(store, &run).await
                {
                    let _ = store.fail_method_search_run(&run, &error).await;
                }
                server.continue_after_method_search(&run).await;
            });
        }
        self.method_search_details(root, project, run_id).await
    }

    /// Runs the search in the background until it pauses or ends.
    fn spawn_method_search(
        &self,
        root: &Path,
        run_id: &str,
        guard: ActiveMethodSearchGuard,
        generator: Arc<dyn CandidateGenerator>,
    ) {
        let server = self.clone();
        let root = root.to_path_buf();
        let run = run_id.to_owned();
        let evaluator = self.method_search_evaluator();
        tokio::spawn(async move {
            let _guard = guard;
            let store = &server.0.store;
            match coordinator::run_method_search_coordinator(
                store,
                &root,
                &run,
                generator.as_ref(),
                evaluator.as_ref(),
            )
            .await
            {
                Ok(CoordinatorOutcome::Paused) => {}
                Ok(CoordinatorOutcome::Terminal) => server.continue_after_method_search(&run).await,
                Err(error) => {
                    let _ = store.fail_method_search_run(&run, &error).await;
                    server.continue_after_method_search(&run).await;
                }
            }
        });
    }

    /// Once a search has ended, the batch step that started it, if one
    /// did, is settled with its result.
    /// Here no batch is waiting on the search in this run of the app (it
    /// was resumed after a restart), so the batch is carried on from its
    /// recorded steps, as the WebView's `continue_recovered_workflow` does.
    pub(crate) async fn continue_after_method_search(&self, run_id: &str) {
        match coordinator::settle_linked_workflow_attempt(&self.0.store, run_id).await {
            Ok(Some(workflow)) => {
                if let Err(error) = self.continue_delegated_batch(&workflow).await {
                    tracing::error!(%run_id, %workflow, %error, "Could not carry on a batch after its method search");
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::error!(%run_id, %error, "Could not settle the batch step of a method search");
            }
        }
    }
}

/// A contract over a scripted evaluator, for tests here and of the run
/// activity.
#[cfg(test)]
pub(crate) mod testing {
    use super::super::prepare::{
        prepare_method_search_with_evaluator, EvaluationExecution, EvaluationRequest,
        MethodSearchEvaluator, MethodSearchPreparationInput, PrepareMethodSearchRequest,
        PreparedMethodSearch,
    };
    use async_trait::async_trait;

    /// Scores a candidate by the number its `fit_model` adds; answers the
    /// reachability sentinel by failing, as Python would.
    pub(crate) struct ScriptedEvaluator;

    #[async_trait]
    impl MethodSearchEvaluator for ScriptedEvaluator {
        async fn evaluate(
            &self,
            request: &EvaluationRequest,
        ) -> Result<EvaluationExecution, String> {
            let source = std::fs::read_to_string(request.workspace.join(&request.target_path))
                .map_err(|error| error.to_string())?;
            if source.contains("wisp_method_search_reachability_sentinel") {
                return Ok(EvaluationExecution {
                    exit_code: Some(1),
                    stderr: "RuntimeError: wisp_method_search_reachability_sentinel".into(),
                    ..Default::default()
                });
            }
            let added = source
                .lines()
                .find_map(|line| line.trim().strip_prefix("return len(rows) + "))
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0);
            let primary = 0.5 + f64::from(added) / 100.0;
            Ok(EvaluationExecution {
                exit_code: Some(0),
                stdout: format!(
                    "wisp_evaluate: {{\"primary\":{primary},\"metrics\":{{\"accuracy\":{primary}}}}}"
                ),
                ..Default::default()
            })
        }
    }

    /// Writes a small target, evaluator and data into `root` and freezes
    /// a contract for `frame` of `project`.
    pub(crate) async fn prepare(
        store: &wisp_store::Store,
        project: &str,
        root: &std::path::Path,
        frame: &str,
        max_candidates: u32,
    ) -> PreparedMethodSearch {
        std::fs::create_dir_all(root.join("analysis")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(
            root.join("analysis/model.py"),
            "def fit_model(rows):\n    return len(rows)\n",
        )
        .unwrap();
        std::fs::write(root.join("analysis/evaluate.py"), "# scripted\n").unwrap();
        std::fs::write(root.join("data/validation.csv"), "x,y\n1,1\n").unwrap();
        prepare_method_search_with_evaluator(
            store,
            project,
            root,
            frame,
            PrepareMethodSearchRequest {
                objective: "Improve validation accuracy".into(),
                target_path: "analysis/model.py".into(),
                target_symbol: "fit_model".into(),
                evaluator_path: "analysis/evaluate.py".into(),
                primary_metric: "accuracy".into(),
                direction: wisp_core::method_search::ScoreDirection::Maximize,
                guardrails: vec![],
                inputs: vec![MethodSearchPreparationInput {
                    role: "search_validation".into(),
                    path: "data/validation.csv".into(),
                }],
                constraints: vec![],
                strategy_sources: vec![],
                repetitions: 3,
                evaluator_timeout_seconds: 30,
                max_candidates,
                max_wall_seconds: 3_600,
                max_cost_microunits: 1_000_000,
                final_verification_path: None,
                python_executable: None,
            },
            &ScriptedEvaluator,
        )
        .await
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{prepare, ScriptedEvaluator};
    use super::*;

    /// A Draft search of `frame`, as a contract and its Run, without the
    /// batch step that would approve its model.
    async fn draft(server: &Server, root: &Path, frame: &str) -> String {
        let store = &server.0.store;
        let prepared = prepare(store, crate::PROJECT, root, frame, 3).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut run = RunRecord::new(
            &run_id,
            crate::PROJECT,
            "local",
            "Develop computational method",
            "method_search",
        );
        run.frame_id = Some(frame.into());
        let context = store.get_execution_context("local").await.unwrap().unwrap();
        run.env_snapshot_json = serde_json::to_string(&context).unwrap();
        store.create_run(&run).await.unwrap();
        store
            .create_method_search_run_state(
                &MethodSearchRunState::new(
                    &run_id,
                    &prepared.method_search_spec_artifact_version_id,
                    &prepared.method_search_spec_sha256,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        coordinator::bind_method_search_run_inputs(store, root, &run_id)
            .await
            .unwrap();
        run_id
    }

    async fn ended(server: &Server, run_id: &str) -> RunStatus {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let status = server
                    .0
                    .store
                    .method_search_run_status(run_id)
                    .await
                    .unwrap()
                    .unwrap();
                if status.is_terminal() {
                    return status;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the search must end")
    }

    #[tokio::test]
    async fn a_draft_search_shows_its_contract_and_can_be_cancelled() {
        let temp = tempfile::tempdir().unwrap();
        let server = Server::open_with_models(
            crate::tests::config(temp.path()),
            crate::tests::models(Arc::default(), false),
        )
        .await
        .unwrap();
        server.set_method_search_evaluator(Arc::new(ScriptedEvaluator));
        let root = std::path::PathBuf::from(server.project_root(crate::PROJECT).await.unwrap());
        let frame = server.create_conversation(crate::PROJECT).await.unwrap();
        let run_id = draft(&server, &root, &frame).await;

        // Its card reads the frozen contract and the audited baseline.
        let details = server
            .method_search_run(crate::PROJECT, &run_id)
            .await
            .unwrap();
        assert_eq!(details.run.status, RunStatus::Draft);
        assert_eq!(details.spec.target.symbol, "fit_model");
        assert!(details.audit.sentinel_reachable);
        assert!(details.candidates.is_empty());
        assert!(details.activity.is_none());
        assert!(server.method_search_run("other", &run_id).await.is_err());

        // Without the batch step that approved its model it does not start,
        // and a search that is not running cannot pause.
        assert!(server
            .start_method_search(crate::PROJECT, &run_id)
            .await
            .is_err());
        assert!(server
            .pause_method_search(crate::PROJECT, &run_id)
            .await
            .is_err());
        assert!(server
            .resume_method_search(crate::PROJECT, &run_id)
            .await
            .is_err());

        // Cancelled in Draft, it ends at once and makes no candidate.
        server
            .cancel_method_search(crate::PROJECT, &run_id)
            .await
            .unwrap();
        assert_eq!(ended(&server, &run_id).await, RunStatus::Cancelled);
        let details = server
            .method_search_run(crate::PROJECT, &run_id)
            .await
            .unwrap();
        assert!(details.candidates.is_empty());

        // The Runs tab's Cancel ends a method search through its own path.
        let other = draft(&server, &root, &frame).await;
        server.cancel_run(crate::PROJECT, &other).await.unwrap();
        assert_eq!(ended(&server, &other).await, RunStatus::Cancelled);
        server.shutdown().await;
    }
}
