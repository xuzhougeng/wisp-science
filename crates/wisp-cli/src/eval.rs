use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use wisp_core::{Agent, MemoryManager, Output};
use wisp_llm::ProviderConfig;
use wisp_skills::SkillIndex;

const SUITE_VERSION: &str = "agent-eval-v0";
const MAX_CONTEXT_TOKENS: usize = 128_000;
const MAX_ROUNDS: usize = 12;
const EVAL_TOOLS: [&str; 6] = [
    "read",
    "write",
    "edit",
    "search",
    "grep",
    "attempt_completion",
];

#[derive(Clone, Copy)]
struct Scenario {
    id: &'static str,
    description: &'static str,
    prompt: &'static str,
}

fn scenarios() -> [Scenario; 6] {
    [
        Scenario {
            id: "read_facts",
            description: "Read a file and report grounded facts without changing it.",
            prompt: "Use read to inspect notes.txt. Do not create or modify files. Report the number \
                     of non-empty lines and the first word as `lines=<integer>; first=<word>`, then \
                     finish with attempt_completion.",
        },
        Scenario {
            id: "search_files",
            description: "Find matching files and report stable relative paths.",
            prompt: "Use search to find every Python file in the workspace. Do not create or modify \
                     files. Report the relative paths in ascending order, then finish with \
                     attempt_completion.",
        },
        Scenario {
            id: "grep_marker",
            description: "Locate a content marker without changing the workspace.",
            prompt: "Use grep to find every line containing the literal marker CALIBRATE. Do not \
                     create or modify files. Report each matching relative path and line, then \
                     finish with attempt_completion.",
        },
        Scenario {
            id: "targeted_edit",
            description: "Make one exact configuration edit and preserve surrounding content.",
            prompt: "Use edit to change only `iterations = 10` to `iterations = 25` in config.toml. \
                     Preserve every other byte and finish with attempt_completion.",
        },
        Scenario {
            id: "derive_file",
            description: "Read tabular input, calculate a value, and write one derived file.",
            prompt: "Read measurements.csv. Create results/summary.txt with exactly two lines: \
                     `count=<number of data rows>` and `mean=<one decimal mean of value>`, including \
                     a final newline. Use write for the new file and finish with \
                     attempt_completion.",
        },
        Scenario {
            id: "avoid_noop_edit",
            description: "Recognize an already-satisfied state and avoid an unnecessary edit.",
            prompt: "Read settings.json and check whether enabled is already true. If it is, do not \
                     modify any file. Report `enabled=<value>; limit=<value>` and finish with \
                     attempt_completion.",
        },
    ]
}

#[derive(Debug, Clone, Default)]
struct Captured {
    rounds: usize,
    tool_calls: Vec<String>,
    tool_errors: u64,
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    cached_tokens: u64,
    completion: Option<String>,
}

#[derive(Default)]
struct EvalOutput {
    captured: Mutex<Captured>,
}

impl EvalOutput {
    fn snapshot(&self) -> Captured {
        self.captured
            .lock()
            .expect("eval output mutex poisoned")
            .clone()
    }
}

impl Output for EvalOutput {
    fn tool_call(&self, name: &str, _preview: &str) {
        self.captured
            .lock()
            .expect("eval output mutex poisoned")
            .tool_calls
            .push(name.to_string());
    }

    fn tool_result(&self, name: &str, ok: bool, content: &str, _duration_ms: u64) {
        let mut captured = self.captured.lock().expect("eval output mutex poisoned");
        if !ok {
            captured.tool_errors += 1;
        }
        if name == "attempt_completion" && ok {
            captured.completion = Some(content.to_string());
        }
    }

    fn usage(
        &self,
        _round: usize,
        input: u64,
        output: u64,
        reasoning: u64,
        cached: u64,
        _ctx_tokens: usize,
        _max_context: usize,
    ) {
        let mut captured = self.captured.lock().expect("eval output mutex poisoned");
        captured.rounds += 1;
        captured.input_tokens += input;
        captured.output_tokens += output;
        captured.reasoning_tokens += reasoning;
        captured.cached_tokens += cached;
    }

    fn confirm(&self, _message: &str) -> bool {
        false
    }

    fn restrict_read_paths_to_project(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScenarioResult {
    id: String,
    description: String,
    passed: bool,
    failure: Option<String>,
    duration_ms: u64,
    rounds: usize,
    tool_calls: Vec<String>,
    tool_errors: u64,
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    cached_tokens: u64,
    completion: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ReportSummary {
    total: usize,
    passed: usize,
    duration_ms: u64,
    rounds: usize,
    tool_calls: u64,
    tool_errors: u64,
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    cached_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BaselineComparison {
    baseline: String,
    warnings: Vec<String>,
    passed_delta: i64,
    duration_ms_delta: i64,
    rounds_delta: i64,
    tool_calls_delta: i64,
    tool_errors_delta: i64,
    input_tokens_delta: i64,
    output_tokens_delta: i64,
    reasoning_tokens_delta: i64,
    cached_tokens_delta: i64,
    regressions: Vec<String>,
    improvements: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvalReport {
    suite_version: String,
    wisp_version: String,
    generated_at: String,
    provider: String,
    model: String,
    max_context_tokens: usize,
    max_rounds: usize,
    tools: Vec<String>,
    scenario_prompt_hash: String,
    tool_schema_hash: String,
    summary: ReportSummary,
    scenarios: Vec<ScenarioResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    comparison: Option<BaselineComparison>,
}

pub async fn run(cfg: ProviderConfig, save: Option<&Path>, compare: Option<&Path>) -> Result<()> {
    let baseline = compare
        .map(|path| {
            let bytes = std::fs::read(path)
                .with_context(|| format!("could not read baseline {}", path.display()))?;
            serde_json::from_slice::<EvalReport>(&bytes)
                .with_context(|| format!("invalid eval baseline {}", path.display()))
        })
        .transpose()?;

    let mut report = evaluate(cfg).await?;
    if let (Some(path), Some(baseline)) = (compare, baseline.as_ref()) {
        report.comparison = Some(compare_reports(&report, baseline, path));
    }

    let mut rendered = serde_json::to_string_pretty(&report)?;
    rendered.push('\n');
    if let Some(path) = save {
        std::fs::write(path, &rendered)
            .with_context(|| format!("could not save eval report {}", path.display()))?;
    }
    print!("{rendered}");

    if report.summary.passed != report.summary.total {
        bail!(
            "agent eval failed: {}/{} scenarios passed",
            report.summary.passed,
            report.summary.total
        );
    }
    Ok(())
}

async fn evaluate(cfg: ProviderConfig) -> Result<EvalReport> {
    let catalog = scenarios();
    let model = cfg.model.clone();
    let provider = format!("{:?}", cfg.kind);
    let mut results = Vec::with_capacity(catalog.len());
    let mut tool_schema_hash = None;

    for (index, scenario) in catalog.iter().copied().enumerate() {
        eprintln!(
            "[{}/{}] {} — {}",
            index + 1,
            catalog.len(),
            scenario.id,
            scenario.description
        );
        let (result, current_schema_hash) = run_scenario(cfg.clone(), scenario).await?;
        if let Some(expected) = tool_schema_hash.as_ref() {
            if expected != &current_schema_hash {
                bail!("eval tool schemas changed between isolated scenarios");
            }
        } else {
            tool_schema_hash = Some(current_schema_hash);
        }
        eprintln!(
            "  {} ({} ms, {} rounds, {} tool calls)",
            if result.passed { "pass" } else { "FAIL" },
            result.duration_ms,
            result.rounds,
            result.tool_calls.len()
        );
        results.push(result);
    }

    Ok(EvalReport {
        suite_version: SUITE_VERSION.into(),
        wisp_version: env!("CARGO_PKG_VERSION").into(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        provider,
        model,
        max_context_tokens: MAX_CONTEXT_TOKENS,
        max_rounds: MAX_ROUNDS,
        tools: EVAL_TOOLS.iter().map(|tool| (*tool).to_string()).collect(),
        scenario_prompt_hash: scenario_prompt_hash(&catalog),
        tool_schema_hash: tool_schema_hash.unwrap_or_default(),
        summary: summarize(&results),
        scenarios: results,
        comparison: None,
    })
}

async fn run_scenario(cfg: ProviderConfig, scenario: Scenario) -> Result<(ScenarioResult, String)> {
    let workspace = TempWorkspace::new(scenario.id)?;
    setup_fixture(scenario.id, workspace.path())?;
    let before = snapshot_workspace(workspace.path())?;
    let (mut agent, tool_schema_hash) = build_agent(cfg, workspace.path());
    let output = EvalOutput::default();

    let started = Instant::now();
    let agent_result = agent.run(scenario.prompt, &output, None).await;
    let duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    let captured = output.snapshot();
    let after = snapshot_workspace(workspace.path())?;
    let verification = match agent_result {
        Ok(()) => verify_scenario(scenario.id, &before, &after, &captured),
        Err(error) => Err(anyhow::anyhow!("agent error: {error}")),
    };
    let failure = verification.err().map(|error| error.to_string());

    Ok((
        ScenarioResult {
            id: scenario.id.into(),
            description: scenario.description.into(),
            passed: failure.is_none(),
            failure,
            duration_ms,
            rounds: captured.rounds,
            tool_calls: captured.tool_calls,
            tool_errors: captured.tool_errors,
            input_tokens: captured.input_tokens,
            output_tokens: captured.output_tokens,
            reasoning_tokens: captured.reasoning_tokens,
            cached_tokens: captured.cached_tokens,
            completion: captured.completion,
        },
        tool_schema_hash,
    ))
}

fn build_agent(cfg: ProviderConfig, root: &Path) -> (Agent, String) {
    let skills = Arc::new(SkillIndex::default());
    let memory = Arc::new(MemoryManager::new(root));
    let mut agent = Agent::new(
        cfg,
        skills.clone(),
        memory.clone(),
        root.to_path_buf(),
        MAX_CONTEXT_TOKENS,
        MAX_ROUNDS,
        false,
        None,
    );
    let allowed: Vec<String> = EVAL_TOOLS.iter().map(|tool| (*tool).to_string()).collect();
    agent.tools = wisp_core::build_registry(skills.clone(), memory, false).filtered(&allowed);
    agent.seed_system_prompt(&skills, None);
    let schemas = serde_json::to_vec(&agent.tools.schemas()).expect("tool schemas serialize");
    (agent, sha256_hex(&schemas))
}

fn setup_fixture(id: &str, root: &Path) -> Result<()> {
    match id {
        "read_facts" => write_fixture(root, "notes.txt", "alpha beta\n\ngamma delta\n"),
        "search_files" => {
            write_fixture(root, "src/analyze.py", "print('analysis')\n")?;
            write_fixture(root, "tools/export.py", "print('export')\n")?;
            write_fixture(root, "src/lib.rs", "pub fn analyze() {}\n")
        }
        "grep_marker" => {
            write_fixture(
                root,
                "src/analysis.py",
                "# TODO(CALIBRATE): validate offset\nvalue = 3\n",
            )?;
            write_fixture(root, "docs/notes.md", "Calibration notes are pending.\n")
        }
        "targeted_edit" => write_fixture(root, "config.toml", "iterations = 10\nmode = \"fast\"\n"),
        "derive_file" => {
            write_fixture(root, "measurements.csv", "sample,value\na,1\nb,2\nc,3\n")?;
            std::fs::create_dir_all(root.join("results"))?;
            Ok(())
        }
        "avoid_noop_edit" => {
            write_fixture(root, "settings.json", "{\"enabled\":true,\"limit\":5}\n")
        }
        _ => bail!("unknown eval scenario '{id}'"),
    }
}

fn write_fixture(root: &Path, relative: &str, content: &str) -> Result<()> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

fn verify_scenario(
    id: &str,
    before: &BTreeMap<String, Vec<u8>>,
    after: &BTreeMap<String, Vec<u8>>,
    captured: &Captured,
) -> Result<()> {
    require_tool(captured, "attempt_completion")?;
    let mut expected = before.clone();
    match id {
        "read_facts" => {
            require_tool(captured, "read")?;
            require_no_mutation_tools(captured)?;
            require_completion(captured, &["lines=2", "first=alpha"])?;
        }
        "search_files" => {
            require_tool(captured, "search")?;
            require_no_mutation_tools(captured)?;
            let completion = require_completion(captured, &["src/analyze.py", "tools/export.py"])?;
            let completion = completion.to_ascii_lowercase().replace('\\', "/");
            if completion.find("src/analyze.py") > completion.find("tools/export.py") {
                bail!("Python paths were not reported in ascending order");
            }
        }
        "grep_marker" => {
            require_tool(captured, "grep")?;
            require_no_mutation_tools(captured)?;
            require_completion(captured, &["src/analysis.py", "calibrate"])?;
        }
        "targeted_edit" => {
            require_tool(captured, "edit")?;
            expected.insert(
                "config.toml".into(),
                b"iterations = 25\nmode = \"fast\"\n".to_vec(),
            );
        }
        "derive_file" => {
            require_tool(captured, "read")?;
            require_tool(captured, "write")?;
            expected.insert(
                "results/summary.txt".into(),
                b"count=3\nmean=2.0\n".to_vec(),
            );
        }
        "avoid_noop_edit" => {
            require_tool(captured, "read")?;
            require_no_mutation_tools(captured)?;
            require_completion(captured, &["enabled=true", "limit=5"])?;
        }
        _ => bail!("unknown eval scenario '{id}'"),
    }
    require_workspace(&expected, after)
}

fn require_tool(captured: &Captured, name: &str) -> Result<()> {
    if captured.tool_calls.iter().any(|tool| tool == name) {
        Ok(())
    } else {
        bail!("required tool '{name}' was not called")
    }
}

fn require_no_mutation_tools(captured: &Captured) -> Result<()> {
    if let Some(tool) = captured
        .tool_calls
        .iter()
        .find(|tool| matches!(tool.as_str(), "write" | "edit"))
    {
        bail!("unexpected mutation tool '{tool}' was called");
    }
    Ok(())
}

fn require_completion<'a>(captured: &'a Captured, fragments: &[&str]) -> Result<&'a str> {
    let completion = captured
        .completion
        .as_deref()
        .context("attempt_completion returned no result")?;
    let lowercase = completion.to_ascii_lowercase().replace('\\', "/");
    for fragment in fragments {
        if !lowercase.contains(&fragment.to_ascii_lowercase()) {
            bail!("completion did not contain '{fragment}'");
        }
    }
    Ok(completion)
}

fn require_workspace(
    expected: &BTreeMap<String, Vec<u8>>,
    actual: &BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    let paths: BTreeSet<_> = expected
        .keys()
        .chain(actual.keys())
        .filter(|path| expected.get(*path) != actual.get(*path))
        .cloned()
        .collect();
    bail!(
        "workspace differed at: {}",
        paths.into_iter().collect::<Vec<_>>().join(", ")
    )
}

fn snapshot_workspace(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path.strip_prefix(root).expect("entry is under eval root");
            if relative
                .components()
                .next()
                .is_some_and(|part| part.as_os_str() == ".wisp")
            {
                continue;
            }
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                visit(root, &path, files)?;
            } else if file_type.is_file() {
                let key = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                files.insert(key, std::fs::read(path)?);
            }
        }
        Ok(())
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn summarize(results: &[ScenarioResult]) -> ReportSummary {
    ReportSummary {
        total: results.len(),
        passed: results.iter().filter(|result| result.passed).count(),
        duration_ms: results.iter().map(|result| result.duration_ms).sum(),
        rounds: results.iter().map(|result| result.rounds).sum(),
        tool_calls: results
            .iter()
            .map(|result| result.tool_calls.len() as u64)
            .sum(),
        tool_errors: results.iter().map(|result| result.tool_errors).sum(),
        input_tokens: results.iter().map(|result| result.input_tokens).sum(),
        output_tokens: results.iter().map(|result| result.output_tokens).sum(),
        reasoning_tokens: results.iter().map(|result| result.reasoning_tokens).sum(),
        cached_tokens: results.iter().map(|result| result.cached_tokens).sum(),
    }
}

fn compare_reports(current: &EvalReport, baseline: &EvalReport, path: &Path) -> BaselineComparison {
    let mut warnings = Vec::new();
    for (label, current_value, baseline_value) in [
        (
            "suite version",
            current.suite_version.as_str(),
            baseline.suite_version.as_str(),
        ),
        (
            "scenario prompts",
            current.scenario_prompt_hash.as_str(),
            baseline.scenario_prompt_hash.as_str(),
        ),
        (
            "tool schemas",
            current.tool_schema_hash.as_str(),
            baseline.tool_schema_hash.as_str(),
        ),
        (
            "provider",
            current.provider.as_str(),
            baseline.provider.as_str(),
        ),
        ("model", current.model.as_str(), baseline.model.as_str()),
    ] {
        if current_value != baseline_value {
            warnings.push(format!("{label} differ"));
        }
    }

    let baseline_passes: BTreeMap<_, _> = baseline
        .scenarios
        .iter()
        .map(|result| (result.id.as_str(), result.passed))
        .collect();
    let current_passes: BTreeMap<_, _> = current
        .scenarios
        .iter()
        .map(|result| (result.id.as_str(), result.passed))
        .collect();
    if baseline_passes.keys().collect::<Vec<_>>() != current_passes.keys().collect::<Vec<_>>() {
        warnings.push("scenario IDs differ".into());
    }
    let regressions = baseline_passes
        .iter()
        .filter(|(id, passed)| **passed && current_passes.get(*id) == Some(&false))
        .map(|(id, _)| (*id).to_string())
        .collect();
    let improvements = baseline_passes
        .iter()
        .filter(|(id, passed)| !**passed && current_passes.get(*id) == Some(&true))
        .map(|(id, _)| (*id).to_string())
        .collect();

    BaselineComparison {
        baseline: path.to_string_lossy().into_owned(),
        warnings,
        passed_delta: delta(
            current.summary.passed as u64,
            baseline.summary.passed as u64,
        ),
        duration_ms_delta: delta(current.summary.duration_ms, baseline.summary.duration_ms),
        rounds_delta: delta(
            current.summary.rounds as u64,
            baseline.summary.rounds as u64,
        ),
        tool_calls_delta: delta(current.summary.tool_calls, baseline.summary.tool_calls),
        tool_errors_delta: delta(current.summary.tool_errors, baseline.summary.tool_errors),
        input_tokens_delta: delta(current.summary.input_tokens, baseline.summary.input_tokens),
        output_tokens_delta: delta(
            current.summary.output_tokens,
            baseline.summary.output_tokens,
        ),
        reasoning_tokens_delta: delta(
            current.summary.reasoning_tokens,
            baseline.summary.reasoning_tokens,
        ),
        cached_tokens_delta: delta(
            current.summary.cached_tokens,
            baseline.summary.cached_tokens,
        ),
        regressions,
        improvements,
    }
}

fn delta(current: u64, baseline: u64) -> i64 {
    i128::from(current)
        .saturating_sub(i128::from(baseline))
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

fn scenario_prompt_hash(catalog: &[Scenario]) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(SUITE_VERSION.as_bytes());
    for scenario in catalog {
        bytes.extend_from_slice(scenario.id.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(scenario.prompt.as_bytes());
        bytes.push(0xff);
    }
    sha256_hex(&bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

static WORKSPACE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempWorkspace {
    path: PathBuf,
    base: PathBuf,
}

impl TempWorkspace {
    fn new(id: &str) -> Result<Self> {
        let base = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = WORKSPACE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!(
            "wisp-agent-eval-{}-{nonce}-{sequence}-{id}",
            std::process::id()
        ));
        std::fs::create_dir(&path)
            .with_context(|| format!("could not create eval workspace {}", path.display()))?;
        Ok(Self { path, base })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let is_eval_dir = self.path.parent() == Some(self.base.as_path())
            && self
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("wisp-agent-eval-"));
        if is_eval_dir {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_scenario_fixtures_accept_the_expected_outcome() {
        for scenario in scenarios() {
            let workspace = TempWorkspace::new(scenario.id).unwrap();
            setup_fixture(scenario.id, workspace.path()).unwrap();
            let before = snapshot_workspace(workspace.path()).unwrap();
            let (tool_calls, completion) = expected_capture(scenario.id);
            materialize_expected(scenario.id, workspace.path()).unwrap();
            let after = snapshot_workspace(workspace.path()).unwrap();
            let captured = Captured {
                tool_calls,
                completion: Some(completion.into()),
                ..Captured::default()
            };
            verify_scenario(scenario.id, &before, &after, &captured).unwrap();
        }
    }

    #[test]
    fn verifier_rejects_an_unexpected_workspace_change() {
        let workspace = TempWorkspace::new("read_facts").unwrap();
        setup_fixture("read_facts", workspace.path()).unwrap();
        let before = snapshot_workspace(workspace.path()).unwrap();
        write_fixture(workspace.path(), "extra.txt", "unexpected\n").unwrap();
        let after = snapshot_workspace(workspace.path()).unwrap();
        let captured = Captured {
            tool_calls: vec!["read".into(), "attempt_completion".into()],
            completion: Some("lines=2; first=alpha".into()),
            ..Captured::default()
        };
        let error = verify_scenario("read_facts", &before, &after, &captured).unwrap_err();
        assert!(error.to_string().contains("extra.txt"));
    }

    #[test]
    fn baseline_comparison_reports_regressions_and_metric_deltas() {
        let baseline = report(vec![
            result("read_facts", true, 100, 10),
            result("search_files", false, 200, 20),
        ]);
        let current = report(vec![
            result("read_facts", false, 150, 15),
            result("search_files", true, 250, 25),
        ]);
        let comparison = compare_reports(&current, &baseline, Path::new("baseline.json"));

        assert_eq!(comparison.regressions, vec!["read_facts"]);
        assert_eq!(comparison.improvements, vec!["search_files"]);
        assert_eq!(comparison.duration_ms_delta, 100);
        assert_eq!(comparison.input_tokens_delta, 10);
    }

    #[test]
    fn prompt_hash_is_stable_and_sensitive_to_prompt_text() {
        let catalog = scenarios();
        let hash = scenario_prompt_hash(&catalog);
        assert_eq!(hash, scenario_prompt_hash(&catalog));
        let mut changed = catalog;
        changed[0].prompt = "changed";
        assert_ne!(hash, scenario_prompt_hash(&changed));
    }

    fn expected_capture(id: &str) -> (Vec<String>, &'static str) {
        match id {
            "read_facts" => (
                vec!["read".into(), "attempt_completion".into()],
                "lines=2; first=alpha",
            ),
            "search_files" => (
                vec!["search".into(), "attempt_completion".into()],
                "src/analyze.py\ntools/export.py",
            ),
            "grep_marker" => (
                vec!["grep".into(), "attempt_completion".into()],
                "src/analysis.py:1: CALIBRATE",
            ),
            "targeted_edit" => (
                vec!["edit".into(), "attempt_completion".into()],
                "updated config",
            ),
            "derive_file" => (
                vec!["read".into(), "write".into(), "attempt_completion".into()],
                "wrote summary",
            ),
            "avoid_noop_edit" => (
                vec!["read".into(), "attempt_completion".into()],
                "enabled=true; limit=5",
            ),
            _ => panic!("unknown scenario"),
        }
    }

    fn materialize_expected(id: &str, root: &Path) -> Result<()> {
        match id {
            "targeted_edit" => {
                write_fixture(root, "config.toml", "iterations = 25\nmode = \"fast\"\n")
            }
            "derive_file" => write_fixture(root, "results/summary.txt", "count=3\nmean=2.0\n"),
            _ => Ok(()),
        }
    }

    fn result(id: &str, passed: bool, duration_ms: u64, input_tokens: u64) -> ScenarioResult {
        ScenarioResult {
            id: id.into(),
            description: String::new(),
            passed,
            failure: None,
            duration_ms,
            rounds: 1,
            tool_calls: vec!["attempt_completion".into()],
            tool_errors: 0,
            input_tokens,
            output_tokens: 1,
            reasoning_tokens: 0,
            cached_tokens: 0,
            completion: None,
        }
    }

    fn report(scenarios: Vec<ScenarioResult>) -> EvalReport {
        EvalReport {
            suite_version: SUITE_VERSION.into(),
            wisp_version: "test".into(),
            generated_at: String::new(),
            provider: "test".into(),
            model: "test".into(),
            max_context_tokens: MAX_CONTEXT_TOKENS,
            max_rounds: MAX_ROUNDS,
            tools: Vec::new(),
            scenario_prompt_hash: "prompts".into(),
            tool_schema_hash: "tools".into(),
            summary: summarize(&scenarios),
            scenarios,
            comparison: None,
        }
    }
}
