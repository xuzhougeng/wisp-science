//! Isolated verification for a Frozen Publication revision.
//!
//! The runner sees a fresh temporary directory populated only from exact
//! allowlisted snapshots in the frozen manifest. Verification records are
//! append-only side reports; frozen evidence and revision metadata are never
//! updated.
//!
//! Shared by the WebView host (`src-tauri`) and the embedded desktop host
//! (`wisp-server`); each passes its run runner and its freeze module's
//! security scanner.

use crate::publication_capsule::{
    frozen_reproduction_sources, validate_snapshot_file, BlobSource, SecurityScan,
};
use crate::run_context::{
    build_run_command, run_environment_snapshot, RunCommandRunner,
    PUBLICATION_REPRODUCTION_CONTEXT_ID,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use wisp_store::{
    canonical_json, canonical_json_sha256, PublicationCapabilityLevel, ReproductionComparatorKind,
    ReproductionResult, ReproductionRun, ReproductionRunCommit, ReproductionRunStart, Store,
};

const MAX_STRUCTURED_COMPARISON_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReproductionComparisonRequest {
    pub output_id: String,
    pub comparator: ReproductionComparatorKind,
    #[serde(default)]
    pub absolute_tolerance: Option<f64>,
    #[serde(default)]
    pub relative_tolerance: Option<f64>,
}

#[derive(Clone)]
struct ExpectedOutput {
    id: String,
    path: String,
    artifact_version_id: String,
    sha256: String,
    size_bytes: u64,
    blob: Option<BlobSource>,
}

struct IsolatedWorkspace(PathBuf);

impl IsolatedWorkspace {
    fn create() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "wisp-publication-reproduction-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&path)
            .map_err(|error| format!("Cannot create isolated verification workspace: {error}"))?;
        Ok(Self(path))
    }
}

impl Drop for IsolatedWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn safe_relative_path(value: &str) -> Result<String, String> {
    if value.trim().is_empty()
        || value.contains(['\\', '\0', '\n', '\r'])
        || Path::new(value).is_absolute()
    {
        return Err(format!(
            "Verification path is not portable and relative: {value}"
        ));
    }
    let components = Path::new(value)
        .components()
        .map(|component| match component {
            Component::Normal(component) => component.to_str().map(str::to_string),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| format!("Verification path contains an unsafe component: {value}"))?;
    if components.is_empty() {
        return Err("Verification path is empty".into());
    }
    Ok(components.join("/"))
}

fn validate_output_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = root.join(relative);
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(component) = component else {
            return Err("Verification output path contains an unsafe component".into());
        };
        current.push(component);
        if current.exists()
            && std::fs::symlink_metadata(&current)
                .map_err(|error| error.to_string())?
                .file_type()
                .is_symlink()
        {
            return Err("Verification outputs must not use symlinks".into());
        }
    }
    Ok(path)
}

fn copy_blob(source: &BlobSource, destination: &Path) -> Result<(), String> {
    validate_snapshot_file(source)?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut input = BufReader::new(File::open(&source.path).map_err(|error| error.to_string())?);
    let mut output = File::options()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| error.to_string())?;
        digest.update(&buffer[..read]);
        size = size.saturating_add(read as u64);
    }
    output.sync_all().map_err(|error| error.to_string())?;
    if size != source.expected_size || hex::encode(digest.finalize()) != source.expected_sha256 {
        let _ = std::fs::remove_file(destination);
        return Err("Frozen input changed while materializing verification workspace".into());
    }
    Ok(())
}

fn write_declared_text(path: &Path, text: &str, expected_sha256: &str) -> Result<(), String> {
    if hex::encode(Sha256::digest(text.as_bytes())) != expected_sha256 {
        return Err("Frozen code snapshot checksum is invalid".into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut output = File::options()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("Cannot materialize declared code: {error}"))?;
    output
        .write_all(text.as_bytes())
        .and_then(|_| output.sync_all())
        .map_err(|error| error.to_string())
}

/// Whether a frozen command may be rerun: nothing the host's scanner flags,
/// no path out of the workspace, no home folder and no network fetch.
pub fn command_is_isolated(scan: SecurityScan, command: &str) -> bool {
    if !scan(command, true).is_empty() {
        return false;
    }
    let lower = command.to_ascii_lowercase();
    if lower
        .split(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    ';' | '|' | '&' | '(' | ')' | '<' | '>' | '\'' | '"'
                )
        })
        .flat_map(|token| token.split(['/', '\\']))
        .any(|component| component == "..")
    {
        return false;
    }
    ![
        "../",
        "..\\",
        "$home",
        "${home}",
        "%userprofile%",
        "http://",
        "https://",
        " curl ",
        "wget ",
        " ssh ",
        "scp ",
        "sftp ",
        "invoke-webrequest",
        "requests.get",
        "urlopen",
    ]
    .iter()
    .any(|marker| {
        lower.contains(marker)
            || lower
                .trim_start()
                .strip_prefix(marker.trim())
                .is_some_and(|rest| rest.starts_with(char::is_whitespace))
    })
}

/// The environment a rerun in `context` records, as canonical JSON and its
/// SHA-256.
pub fn reproduction_environment(context: &wisp_store::ExecutionContext) -> (String, String) {
    canonical_json_sha256(&run_environment_snapshot(context))
}

fn expected_environment(manifest: &Value, run_id: &str) -> (Option<String>, bool) {
    let Some(environment) = manifest
        .get("environments")
        .and_then(Value::as_array)
        .and_then(|values| {
            values
                .iter()
                .find(|value| value.get("run_id").and_then(Value::as_str) == Some(run_id))
        })
    else {
        return (None, false);
    };
    let expected_hash = environment
        .get("hash")
        .and_then(Value::as_str)
        .map(str::to_string);
    let valid = expected_hash.as_deref().is_some_and(|expected| {
        environment
            .get("snapshot")
            .filter(|snapshot| !snapshot.is_null())
            .is_some_and(|snapshot| canonical_json_sha256(snapshot).1 == expected)
    });
    (expected_hash, valid)
}

fn allowed_process_environment(workspace: &Path) -> Result<Vec<(String, String)>, String> {
    let mut environment = [
        "PATH",
        "Path",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "LANG",
        "LC_ALL",
        "TZ",
    ]
    .into_iter()
    .filter_map(|name| std::env::var(name).ok().map(|value| (name.into(), value)))
    .collect::<Vec<_>>();
    let temporary = workspace.join(".tmp");
    std::fs::create_dir(&temporary)
        .map_err(|error| format!("Cannot create isolated temporary directory: {error}"))?;
    let temporary = temporary.to_string_lossy().into_owned();
    for name in ["TEMP", "TMP", "TMPDIR"] {
        environment.push((name.into(), temporary.clone()));
    }
    Ok(environment)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Comparison target must be a regular non-symlink file".into());
    }
    if metadata.len() > MAX_STRUCTURED_COMPARISON_BYTES {
        return Err(format!(
            "Structured comparison is limited to {} bytes; use SHA-256",
            MAX_STRUCTURED_COMPARISON_BYTES
        ));
    }
    std::fs::read(path).map_err(|error| error.to_string())
}

fn sha256_file(path: &Path) -> Result<(String, u64), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Comparison target must be a regular non-symlink file".into());
    }
    let mut input = BufReader::new(File::open(path).map_err(|error| error.to_string())?);
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        size = size.saturating_add(read as u64);
    }
    Ok((hex::encode(digest.finalize()), size))
}

fn expected_bytes(output: &ExpectedOutput) -> Result<Vec<u8>, String> {
    let source = output
        .blob
        .as_ref()
        .ok_or_else(|| "This comparator requires frozen reference bytes".to_string())?;
    validate_snapshot_file(source)?;
    read_bounded(&source.path)
}

fn normalized_text(bytes: &[u8]) -> Result<String, String> {
    String::from_utf8(bytes.to_vec())
        .map(|text| text.replace("\r\n", "\n"))
        .map_err(|_| "Text comparator requires UTF-8 files".into())
}

fn numeric_value(bytes: &[u8]) -> Result<f64, String> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| "Numeric comparator requires UTF-8".to_string())?
        .trim()
        .parse::<f64>()
        .map_err(|_| "Numeric comparator requires one finite number".to_string())?;
    if !value.is_finite() {
        return Err("Numeric comparator requires one finite number".into());
    }
    Ok(value)
}

fn comparison_result(
    run_id: &str,
    output: &ExpectedOutput,
    request: &ReproductionComparisonRequest,
    workspace: &Path,
) -> ReproductionResult {
    let actual_path = match validate_output_path(workspace, &output.path) {
        Ok(path) => path,
        Err(error) => {
            return failed_result(run_id, output, request, error);
        }
    };
    if !actual_path.exists() {
        return failed_result(
            run_id,
            output,
            request,
            "Declared output was not produced".into(),
        );
    }
    let tolerance = json!({
        "absolute": request.absolute_tolerance,
        "relative": request.relative_tolerance,
    });
    let compared = match request.comparator {
        ReproductionComparatorKind::Sha256 => {
            sha256_file(&actual_path).map(|(actual_sha256, actual_size)| {
                let passed = actual_sha256 == output.sha256 && actual_size == output.size_bytes;
                (
                    passed,
                    json!({"sha256": output.sha256, "size_bytes": output.size_bytes}),
                    json!({"sha256": actual_sha256, "size_bytes": actual_size}),
                    json!({"mode": "byte_exact"}),
                )
            })
        }
        ReproductionComparatorKind::Text => expected_bytes(output).and_then(|expected| {
            let actual = read_bounded(&actual_path)?;
            let expected = normalized_text(&expected)?;
            let actual = normalized_text(&actual)?;
            Ok((
                expected == actual,
                json!({"normalized_sha256": hex::encode(Sha256::digest(expected.as_bytes()))}),
                json!({"normalized_sha256": hex::encode(Sha256::digest(actual.as_bytes()))}),
                json!({"line_endings": "lf"}),
            ))
        }),
        ReproductionComparatorKind::Json => expected_bytes(output).and_then(|expected| {
            let actual = read_bounded(&actual_path)?;
            let expected: Value = serde_json::from_slice(&expected)
                .map_err(|error| format!("Frozen reference JSON is invalid: {error}"))?;
            let actual: Value = serde_json::from_slice(&actual)
                .map_err(|error| format!("Produced JSON is invalid: {error}"))?;
            Ok((
                expected == actual,
                json!({"canonical_sha256": canonical_json_sha256(&expected).1}),
                json!({"canonical_sha256": canonical_json_sha256(&actual).1}),
                json!({"mode": "semantic_json"}),
            ))
        }),
        ReproductionComparatorKind::Numeric => expected_bytes(output).and_then(|expected| {
            let actual = read_bounded(&actual_path)?;
            let expected = numeric_value(&expected)?;
            let actual = numeric_value(&actual)?;
            let absolute = request.absolute_tolerance.unwrap_or(0.0);
            let relative = request.relative_tolerance.unwrap_or(0.0);
            if !absolute.is_finite() || !relative.is_finite() || absolute < 0.0 || relative < 0.0 {
                return Err("Numeric tolerances must be finite and non-negative".into());
            }
            let delta = (actual - expected).abs();
            let allowed = absolute.max(relative * expected.abs());
            Ok((
                delta <= allowed,
                json!({"value": expected}),
                json!({"value": actual}),
                json!({"absolute_error": delta, "allowed_error": allowed}),
            ))
        }),
    };
    match compared {
        Ok((passed, expected, actual, report)) => ReproductionResult {
            id: uuid::Uuid::new_v4().to_string(),
            reproduction_run_id: run_id.into(),
            output_id: output.id.clone(),
            output_path: output.path.clone(),
            expected_artifact_version_id: output.artifact_version_id.clone(),
            comparator_kind: request.comparator,
            required: true,
            expected_json: canonical_json(&expected),
            actual_json: canonical_json(&actual),
            tolerance_json: canonical_json(&tolerance),
            passed,
            report_json: canonical_json(&report),
            created_at: 0,
        },
        Err(error) => failed_result(run_id, output, request, error),
    }
}

fn failed_result(
    run_id: &str,
    output: &ExpectedOutput,
    request: &ReproductionComparisonRequest,
    error: String,
) -> ReproductionResult {
    ReproductionResult {
        id: uuid::Uuid::new_v4().to_string(),
        reproduction_run_id: run_id.into(),
        output_id: output.id.clone(),
        output_path: output.path.clone(),
        expected_artifact_version_id: output.artifact_version_id.clone(),
        comparator_kind: request.comparator,
        required: true,
        expected_json: canonical_json(&json!({
            "sha256": output.sha256,
            "size_bytes": output.size_bytes,
        })),
        actual_json: canonical_json(&json!({"unavailable": true})),
        tolerance_json: canonical_json(&json!({
            "absolute": request.absolute_tolerance,
            "relative": request.relative_tolerance,
        })),
        passed: false,
        report_json: canonical_json(&json!({"error": error})),
        created_at: 0,
    }
}

fn output_expectations(
    manifest: &Value,
    blobs: &BTreeMap<String, BlobSource>,
    source_run_id: &str,
) -> Result<Vec<ExpectedOutput>, String> {
    let outputs = manifest
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| "Frozen manifest has no outputs array".to_string())?;
    let mut expected = Vec::new();
    for output in outputs
        .iter()
        .filter(|output| output.get("run_id").and_then(Value::as_str) == Some(source_run_id))
    {
        let artifact = output
            .get("artifact")
            .filter(|value| value.is_object())
            .ok_or_else(|| "Run output has no exact ArtifactVersion".to_string())?;
        let artifact_version_id = artifact
            .get("source_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Run output ArtifactVersion has no identity".to_string())?;
        let sha256 = artifact
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|value| value.len() == 64)
            .ok_or_else(|| "Run output ArtifactVersion has no SHA-256".to_string())?;
        let size_bytes = artifact
            .get("size_bytes")
            .and_then(Value::as_i64)
            .and_then(|size| u64::try_from(size).ok())
            .ok_or_else(|| "Run output ArtifactVersion has no size".to_string())?;
        expected.push(ExpectedOutput {
            id: output
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "Run output has no identity".to_string())?
                .into(),
            path: safe_relative_path(
                output
                    .get("source_path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Run output has no safe source path".to_string())?,
            )?,
            artifact_version_id: artifact_version_id.into(),
            sha256: sha256.into(),
            size_bytes,
            blob: blobs.get(artifact_version_id).cloned(),
        });
    }
    if expected.is_empty() {
        return Err("Source Run has no exact declared outputs to compare".into());
    }
    expected.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(expected)
}

fn comparison_requests(
    outputs: &[ExpectedOutput],
    requests: &[ReproductionComparisonRequest],
) -> Result<BTreeMap<String, ReproductionComparisonRequest>, String> {
    let known = outputs
        .iter()
        .map(|output| output.id.as_str())
        .collect::<HashSet<_>>();
    let mut comparisons = BTreeMap::new();
    for request in requests {
        if !known.contains(request.output_id.as_str())
            || comparisons
                .insert(request.output_id.clone(), request.clone())
                .is_some()
        {
            return Err("Comparator refers to an unknown or duplicate Run output".into());
        }
    }
    for output in outputs {
        comparisons
            .entry(output.id.clone())
            .or_insert_with(|| ReproductionComparisonRequest {
                output_id: output.id.clone(),
                comparator: ReproductionComparatorKind::Sha256,
                absolute_tolerance: None,
                relative_tolerance: None,
            });
    }
    Ok(comparisons)
}

fn materialize_workspace(
    manifest: &Value,
    blobs: &BTreeMap<String, BlobSource>,
    source_run_id: &str,
    workspace: &Path,
    outputs: &[ExpectedOutput],
) -> Result<String, String> {
    let mut materialized = BTreeMap::<String, Value>::new();
    let inputs = manifest
        .get("inputs")
        .and_then(Value::as_array)
        .ok_or_else(|| "Frozen manifest has no inputs array".to_string())?;
    for input in inputs
        .iter()
        .filter(|input| input.get("run_id").and_then(Value::as_str) == Some(source_run_id))
    {
        let required = input
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let Some(source_id) = input.pointer("/source/source_id").and_then(Value::as_str) else {
            if required {
                return Err("Required Run input has no exact materializable source".into());
            }
            continue;
        };
        let Some(blob) = blobs.get(source_id) else {
            if required {
                return Err(format!(
                    "Required input '{source_id}' is reference-only or omitted by visibility policy"
                ));
            }
            continue;
        };
        let relative = safe_relative_path(
            input
                .get("source_ref")
                .and_then(Value::as_str)
                .ok_or_else(|| "Run input has no source_ref".to_string())?,
        )?;
        if materialized.contains_key(&relative) {
            return Err(format!("Multiple declared inputs target '{relative}'"));
        }
        copy_blob(blob, &workspace.join(&relative))?;
        materialized.insert(
            relative.clone(),
            json!({
                "kind": "input",
                "path": relative,
                "sha256": blob.expected_sha256,
                "source_id": source_id,
            }),
        );
    }

    let code = manifest
        .get("code")
        .and_then(Value::as_array)
        .ok_or_else(|| "Frozen manifest has no code array".to_string())?;
    for snapshot in code
        .iter()
        .filter(|snapshot| snapshot.get("run_id").and_then(Value::as_str) == Some(source_run_id))
    {
        let (Some(path), Some(text), Some(checksum)) = (
            snapshot.get("source_path").and_then(Value::as_str),
            snapshot.get("source_text").and_then(Value::as_str),
            snapshot.get("checksum").and_then(Value::as_str),
        ) else {
            continue;
        };
        if snapshot.get("content_included").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let relative = safe_relative_path(path)?;
        if materialized.contains_key(&relative) {
            return Err(format!("Declared code collides with input '{relative}'"));
        }
        write_declared_text(&workspace.join(&relative), text, checksum)?;
        materialized.insert(
            relative.clone(),
            json!({
                "kind": "code",
                "path": relative,
                "sha256": checksum,
                "source_id": snapshot.get("id").and_then(Value::as_str),
            }),
        );
    }

    for output in outputs {
        if materialized.contains_key(&output.path) {
            return Err(format!(
                "Declared output '{}' collides with an immutable input",
                output.path
            ));
        }
    }
    Ok(canonical_json(&json!({
        "files": materialized.into_values().collect::<Vec<_>>(),
        "schema_version": 1,
        "source_run_id": source_run_id,
    })))
}

/// Reruns `source_run_id` of a Frozen or Published revision in a fresh
/// temporary directory holding only its frozen inputs and code, compares
/// every declared output with the frozen one, and records the report. The
/// revision itself is never changed. Only local runs can be rerun.
pub async fn verify_publication_revision_with_runner(
    store: &Store,
    revision_id: &str,
    source_run_id: &str,
    requests: &[ReproductionComparisonRequest],
    runner: &dyn RunCommandRunner,
    scan: SecurityScan,
) -> Result<ReproductionRun, String> {
    let (manifest, blobs) = frozen_reproduction_sources(store, revision_id, scan).await?;
    let manifest_run = manifest
        .get("runs")
        .and_then(Value::as_array)
        .and_then(|runs| {
            runs.iter()
                .find(|run| run.get("id").and_then(Value::as_str) == Some(source_run_id))
        })
        .ok_or_else(|| "Source Run is not part of the Frozen revision".to_string())?;
    let run = store
        .get_run(source_run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Source Run no longer exists".to_string())?;
    let command = run
        .command
        .as_deref()
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| "Source Run has no command".to_string())?;
    let command_sha256 = hex::encode(Sha256::digest(command.as_bytes()));
    if manifest_run.get("command_sha256").and_then(Value::as_str) != Some(command_sha256.as_str()) {
        return Err("Source Run command no longer matches the Frozen manifest".into());
    }
    if !command_is_isolated(scan, command) {
        return Err(
            "Frozen command contains a machine path, path escape, credential, or network access"
                .into(),
        );
    }
    let context = store
        .get_execution_context(&run.context_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Source Run execution context no longer exists".to_string())?;
    if context.kind != wisp_store::ExecutionContextKind::Local {
        return Err("Clean verification currently supports only local Runs".into());
    }
    let outputs = output_expectations(&manifest, &blobs, source_run_id)?;
    let comparisons = comparison_requests(&outputs, requests)?;
    let workspace = IsolatedWorkspace::create()?;
    let workspace_manifest =
        materialize_workspace(&manifest, &blobs, source_run_id, &workspace.0, &outputs)?;
    let (actual_environment_json, actual_environment_hash) = reproduction_environment(&context);
    let (expected_environment_hash, expected_environment_valid) =
        expected_environment(&manifest, source_run_id);
    let environment_matched = expected_environment_valid
        && expected_environment_hash.as_deref() == Some(actual_environment_hash.as_str());
    let reproduction_id = uuid::Uuid::new_v4().to_string();
    store
        .start_reproduction_run(&ReproductionRunStart {
            id: reproduction_id.clone(),
            revision_id: revision_id.into(),
            source_run_id: source_run_id.into(),
            command_sha256,
            expected_environment_hash,
            actual_environment_json,
            actual_environment_hash,
            environment_matched,
            workspace_manifest_json: workspace_manifest,
        })
        .await
        .map_err(|error| error.to_string())?;

    let mut isolated_context = context;
    isolated_context.id = PUBLICATION_REPRODUCTION_CONTEXT_ID.into();
    let mut run_command = build_run_command(&isolated_context, command, Some(workspace.0.clone()));
    if run_command.program == "sh" && run_command.args.first().is_some_and(|flag| flag == "-lc") {
        run_command.args[0] = "-c".into();
    }
    run_command.envs = allowed_process_environment(&workspace.0)?;
    let timeout = Duration::from_secs(
        run.timeout_secs
            .and_then(|seconds| u64::try_from(seconds).ok())
            .unwrap_or(60)
            .clamp(1, 300),
    );
    let output = match runner.run(run_command, timeout).await {
        Ok(output) => output,
        Err(error) => {
            return store
                .fail_reproduction_run(&reproduction_id, &error)
                .await
                .map_err(|persistence| persistence.to_string());
        }
    };
    let results = outputs
        .iter()
        .map(|expected| {
            comparison_result(
                &reproduction_id,
                expected,
                &comparisons[&expected.id],
                &workspace.0,
            )
        })
        .collect::<Vec<_>>();
    store
        .complete_reproduction_run(&ReproductionRunCommit {
            run_id: reproduction_id,
            stdout_tail: output.stdout,
            stderr_tail: output.stderr,
            exit_code: output.exit_code,
            results,
        })
        .await
        .map_err(|error| error.to_string())
}

/// What a frozen revision may claim once its rerun reports are counted:
/// `reproduced` only when every run its manifest names was rerun with a
/// matching environment and every output compared equal.
pub fn effective_capability(
    frozen: PublicationCapabilityLevel,
    manifest_json: Option<&str>,
    runs: &[ReproductionRun],
) -> PublicationCapabilityLevel {
    if frozen == PublicationCapabilityLevel::Reproduced {
        return frozen;
    }
    if frozen != PublicationCapabilityLevel::ReExecutable {
        return frozen;
    }
    let Some(manifest) = manifest_json.and_then(|json| serde_json::from_str::<Value>(json).ok())
    else {
        return frozen;
    };
    let required = manifest
        .get("runs")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.get("id").and_then(Value::as_str))
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    if required.is_empty() {
        return frozen;
    }
    let reproduced = required.iter().all(|run_id| {
        runs.iter().any(|run| {
            run.source_run_id == *run_id
                && run.status == "completed"
                && run.capability_level == PublicationCapabilityLevel::Reproduced
        })
    });
    if reproduced {
        PublicationCapabilityLevel::Reproduced
    } else {
        frozen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_tolerant_comparators_report_pass_and_fail() {
        let root = std::env::temp_dir().join(format!(
            "wisp-reproduction-comparator-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let expected_path = root.join("expected.txt");
        std::fs::write(&expected_path, b"1.0\n").unwrap();
        let blob = BlobSource {
            path: expected_path.clone(),
            project_root: root.clone(),
            expected_sha256: hex::encode(Sha256::digest(b"1.0\n")),
            expected_size: 4,
            scan_as_text: true,
            public: true,
        };
        let output = ExpectedOutput {
            id: "output".into(),
            path: "result.txt".into(),
            artifact_version_id: "version".into(),
            sha256: blob.expected_sha256.clone(),
            size_bytes: 4,
            blob: Some(blob),
        };
        std::fs::write(root.join("result.txt"), b"1.01\n").unwrap();
        let tolerant = comparison_result(
            "reproduction",
            &output,
            &ReproductionComparisonRequest {
                output_id: "output".into(),
                comparator: ReproductionComparatorKind::Numeric,
                absolute_tolerance: Some(0.02),
                relative_tolerance: None,
            },
            &root,
        );
        assert!(tolerant.passed);
        let exact = comparison_result(
            "reproduction",
            &output,
            &ReproductionComparisonRequest {
                output_id: "output".into(),
                comparator: ReproductionComparatorKind::Sha256,
                absolute_tolerance: None,
                relative_tolerance: None,
            },
            &root,
        );
        assert!(!exact.passed);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn environment_mismatch_cannot_promote_revision() {
        let run = ReproductionRun {
            id: "verification".into(),
            revision_id: "revision".into(),
            source_run_id: "run".into(),
            status: "completed".into(),
            capability_level: PublicationCapabilityLevel::ReExecutable,
            command_sha256: "0".repeat(64),
            expected_environment_hash: Some("1".repeat(64)),
            actual_environment_json: "{}".into(),
            actual_environment_hash: "2".repeat(64),
            environment_matched: false,
            workspace_manifest_json: "{}".into(),
            stdout_tail: None,
            stderr_tail: None,
            exit_code: Some(0),
            error: None,
            created_at: 0,
            started_at: 0,
            completed_at: Some(0),
        };
        assert_eq!(
            effective_capability(
                PublicationCapabilityLevel::ReExecutable,
                Some(r#"{"runs":[{"id":"run"}]}"#),
                &[run],
            ),
            PublicationCapabilityLevel::ReExecutable
        );
    }

    #[test]
    fn reproduction_report_cannot_hide_a_weaker_frozen_capability() {
        let run = ReproductionRun {
            id: "verification".into(),
            revision_id: "revision".into(),
            source_run_id: "run".into(),
            status: "completed".into(),
            capability_level: PublicationCapabilityLevel::Reproduced,
            command_sha256: "0".repeat(64),
            expected_environment_hash: Some("1".repeat(64)),
            actual_environment_json: "{}".into(),
            actual_environment_hash: "1".repeat(64),
            environment_matched: true,
            workspace_manifest_json: "{}".into(),
            stdout_tail: None,
            stderr_tail: None,
            exit_code: Some(0),
            error: None,
            created_at: 0,
            started_at: 0,
            completed_at: Some(0),
        };
        assert_eq!(
            effective_capability(
                PublicationCapabilityLevel::Archived,
                Some(r#"{"runs":[{"id":"run"}]}"#),
                &[run],
            ),
            PublicationCapabilityLevel::Archived
        );
    }

    #[test]
    fn isolated_command_rejects_paths_and_network() {
        fn clean(_: &str, _: bool) -> Vec<String> {
            Vec::new()
        }
        fn flags_keys(text: &str, _: bool) -> Vec<String> {
            if text.contains("api_key=") {
                vec!["credential_pattern".into()]
            } else {
                Vec::new()
            }
        }
        assert!(command_is_isolated(clean, "python analysis.py"));
        assert!(!command_is_isolated(clean, "python ../analysis.py"));
        assert!(!command_is_isolated(clean, "cd .. && python analysis.py"));
        assert!(!command_is_isolated(clean, "curl https://example.com/data"));
        // What the host's scanner flags is refused too.
        assert!(!command_is_isolated(
            flags_keys,
            "python analysis.py --api_key=secretvalue"
        ));
    }
}
