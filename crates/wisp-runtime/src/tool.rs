//! Persistent `python` and `r` tools backed by `RuntimeManager`.

use crate::{
    KernelResp, RuntimeEvent, RuntimeExecutionOptions, RuntimeInfo, RuntimeKey, RuntimeManager,
    LOCAL_CONTEXT_ID, MAX_CODE_BYTES,
};
use async_trait::async_trait;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
};
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolEvent, ToolResult};

/// Normalize path separators for comparison. Only meaningful on Windows,
/// where `\` cannot appear in a filename; on Unix a literal `\` is a legal
/// filename character and replacing it could redirect the path to a
/// *different* existing file (`we\ird.txt` vs `we/ird.txt`), i.e. credit a
/// file the cell never wrote.
fn normalize_separators(path: &str) -> String {
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_string()
    }
}

/// Relativize worker-reported absolute writes to the project root.
/// Outside-root paths, the root itself, and empty remainders are dropped;
/// on Windows `\` is normalized to `/`; duplicates are collapsed; the
/// result is sorted.
fn project_relative_writes(root: &Path, reported: &[String]) -> Vec<String> {
    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut out = Vec::new();
    for raw in reported {
        // Normalize separators before canonicalize/strip so a Windows-style
        // `out\a.txt` still matches on hosts whose temp root is a symlink.
        let normalized = normalize_separators(raw);
        let path = Path::new(&normalized);
        let abs = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let Ok(relative) = abs.strip_prefix(&root) else {
            continue;
        };
        // When canonicalize failed above, `abs` is the raw reported path and
        // the remainder could still climb out of the root (`/root/../etc`).
        // Provenance records feed exports and undo — never let one name a
        // path outside the root.
        if relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            continue;
        }
        let relative = normalize_separators(&relative.to_string_lossy());
        if relative.is_empty() {
            continue;
        }
        out.push(relative);
    }
    out.sort();
    out.dedup();
    out
}

/// Validate paths already reported relative to a host-configured project
/// boundary. Joining and canonicalizing again keeps a compromised or stale
/// worker from naming a symlink target outside the project.
fn validated_project_writes(root: &Path, reported: &[String]) -> Vec<String> {
    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut out = Vec::new();
    for raw in reported {
        let normalized = normalize_separators(raw);
        let relative = Path::new(&normalized);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            continue;
        }
        let candidate = root.join(relative);
        let Ok(candidate) = dunce::canonicalize(candidate) else {
            continue;
        };
        if candidate.strip_prefix(&root).is_err() {
            continue;
        }
        if !normalized.is_empty() {
            out.push(normalized);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Hand a finished cell's self-reported writes to the running tool
/// environment. Legacy absolute reports are local-only. A host-configured
/// project-relative report is also safe for WSL because its REPL runs inside
/// the translated Windows project root; SSH remains a different filesystem.
/// An absent report means the worker could not observe, so the host keeps
/// inferring from its snapshot.
fn report_runtime_writes(env: &dyn ToolEnv, context_id: &str, response: &KernelResp) {
    let Some(reported) = &response.files_written else {
        return;
    };
    let paths = match (context_id, response.files_written_base.as_deref()) {
        (LOCAL_CONTEXT_ID, None) => project_relative_writes(env.project_root(), reported),
        (LOCAL_CONTEXT_ID, Some("project")) => {
            validated_project_writes(env.project_root(), reported)
        }
        (context, Some("project")) if context.starts_with("wsl:") => {
            validated_project_writes(env.project_root(), reported)
        }
        _ => Vec::new(),
    };
    if !paths.is_empty() {
        env.report_written_paths(&paths);
    }
}

pub struct ReplTool {
    manager: RuntimeManager,
    project_id: String,
    scope_key: String,
    session_id: String,
}

pub struct RTool {
    manager: RuntimeManager,
    project_id: String,
    scope_key: String,
    session_id: String,
}

const PYTHON_TOOL_DESCRIPTION: &str = "Execute inline Python code or a project-local .py script in the same persistent REPL. Variables, imports, and loaded data persist per conversation and execution context; parallel conversations never share interpreter state. Prefer script_path for reproducible analysis source that depends on already-loaded large objects; use required_objects to fail instead of silently starting an empty replacement runtime. Return values of expressions are printed. Local and WSL REPLs start in the project root; SSH REPLs use the execution context workdir and receive the script content. Use this for analysis, data loading, plotting, and computation when required packages already exist. Do not use this as a package installer; if dependencies are missing, set up a project-local pixi environment or use local-env-setup first.";
const R_TOOL_DESCRIPTION: &str = "Execute inline R code or a project-local .R script in the same persistent REPL. Variables, libraries, and loaded data persist per conversation and execution context; parallel conversations never share interpreter state. Prefer script_path for reproducible analysis source that depends on already-loaded large objects; use required_objects to fail instead of silently starting an empty replacement runtime. The final visible value is printed. Local and WSL REPLs start in the project root; SSH REPLs use the execution context workdir and receive the script content. Write plots explicitly with png(), pdf(), ggsave(), or another file device. Rscript and the jsonlite package must already exist in that context; this tool does not install packages.";

impl ReplTool {
    pub fn new(manager: RuntimeManager, project_id: impl Into<String>) -> Self {
        Self {
            manager,
            project_id: project_id.into(),
            scope_key: crate::MAINLINE_RUNTIME_SCOPE.into(),
            session_id: String::new(),
        }
    }

    pub fn new_in_session(
        manager: RuntimeManager,
        project_id: impl Into<String>,
        scope_key: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            manager,
            project_id: project_id.into(),
            scope_key: scope_key.into(),
            session_id: session_id.into(),
        }
    }
}

impl RTool {
    pub fn new(manager: RuntimeManager, project_id: impl Into<String>) -> Self {
        Self {
            manager,
            project_id: project_id.into(),
            scope_key: crate::MAINLINE_RUNTIME_SCOPE.into(),
            session_id: String::new(),
        }
    }

    pub fn new_in_session(
        manager: RuntimeManager,
        project_id: impl Into<String>,
        scope_key: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            manager,
            project_id: project_id.into(),
            scope_key: scope_key.into(),
            session_id: session_id.into(),
        }
    }
}

fn context_id(args: &serde_json::Value) -> Result<&str, &'static str> {
    match args.get("context_id") {
        None => Ok(LOCAL_CONTEXT_ID),
        Some(value) => value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or("argument 'context_id' must be a non-empty string"),
    }
}

/// Which bytes ran, independent of the file's later contents. Recorded with the
/// tool call so a source edit cannot rewrite the history of what executed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptProvenance {
    pub path: String,
    pub sha256: String,
}

/// A project-local script read for one runtime execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectScript {
    pub code: String,
    pub provenance: ScriptProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeSource {
    code: String,
    script: Option<ScriptProvenance>,
}

fn code_arg(args: &serde_json::Value) -> Result<String, String> {
    let code = args
        .get("code")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "missing required argument 'code'".to_string())?;
    if code.len() > MAX_CODE_BYTES {
        return Err(format!(
            "argument 'code' exceeds {MAX_CODE_BYTES} byte limit"
        ));
    }
    Ok(code.to_string())
}

fn normalized_script_path(raw: &str) -> Result<(PathBuf, String), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("argument 'script_path' must be a non-empty project-relative path".into());
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err("argument 'script_path' must be project-relative".into());
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("argument 'script_path' must stay inside the project root".into())
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err("argument 'script_path' must name a file".into());
    }
    let display = relative.to_string_lossy().replace('\\', "/");
    Ok((relative, display))
}

/// Read a project-local script exactly as it sits on disk, for execution inside
/// an already-running runtime. Shared by the agent `python`/`r` tools and the
/// editor's whole-script run so both report the same path and hash.
pub fn read_project_script(
    project_root: &Path,
    raw_path: &str,
    expected_extension: &str,
) -> Result<ProjectScript, String> {
    let (relative, display) = normalized_script_path(raw_path)?;
    let extension = relative
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case(expected_extension) {
        return Err(format!(
            "argument 'script_path' must name a .{expected_extension} file"
        ));
    }
    let path = wisp_tools::safety::validate_file_path(project_root, &display)?;
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("read runtime script '{display}' error: {error}"))?;
    if !metadata.is_file() {
        return Err(format!(
            "read runtime script '{display}' error: not a regular file"
        ));
    }
    if metadata.len() > MAX_CODE_BYTES as u64 {
        return Err(format!(
            "runtime script '{display}' exceeds {MAX_CODE_BYTES} byte limit"
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::fs::File::open(&path)
        .and_then(|file| file.take(MAX_CODE_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("read runtime script '{display}' error: {error}"))?;
    if bytes.len() > MAX_CODE_BYTES {
        return Err(format!(
            "runtime script '{display}' grew beyond {MAX_CODE_BYTES} byte limit while reading"
        ));
    }
    let code = String::from_utf8(bytes.clone())
        .map_err(|_| format!("runtime script '{display}' must be UTF-8 text"))?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    Ok(ProjectScript {
        code,
        provenance: ScriptProvenance {
            path: display,
            sha256,
        },
    })
}

fn script_source(
    raw_path: &str,
    env: &dyn ToolEnv,
    expected_extension: &str,
) -> Result<RuntimeSource, String> {
    read_project_script(env.project_root(), raw_path, expected_extension).map(|script| {
        RuntimeSource {
            code: script.code,
            script: Some(script.provenance),
        }
    })
}

fn source_arg(
    args: &serde_json::Value,
    env: &dyn ToolEnv,
    expected_extension: &str,
) -> Result<RuntimeSource, String> {
    match (args.get("code"), args.get("script_path")) {
        (Some(_), Some(_)) => {
            Err("arguments 'code' and 'script_path' are mutually exclusive".into())
        }
        (Some(_), None) => code_arg(args).map(|code| RuntimeSource { code, script: None }),
        (None, Some(value)) => value
            .as_str()
            .ok_or_else(|| "argument 'script_path' must be a string".to_string())
            .and_then(|path| script_source(path, env, expected_extension)),
        (None, None) => Err("provide exactly one of 'code' or 'script_path'".into()),
    }
}

fn required_objects_arg(args: &serde_json::Value) -> Result<Vec<String>, String> {
    let Some(value) = args.get("required_objects") else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| "argument 'required_objects' must be an array of strings".to_string())?;
    if values.len() > 64 {
        return Err("argument 'required_objects' accepts at most 64 names".into());
    }
    let mut names = Vec::with_capacity(values.len());
    for value in values {
        let name = value
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                "argument 'required_objects' must contain non-empty strings".to_string()
            })?;
        if name.len() > 256 {
            return Err("a required runtime object name exceeds 256 bytes".into());
        }
        if !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

fn expected_generation_arg(args: &serde_json::Value) -> Result<Option<u64>, String> {
    let Some(value) = args.get("expected_runtime_generation") else {
        return Ok(None);
    };
    value
        .as_u64()
        .filter(|generation| *generation > 0)
        .map(Some)
        .ok_or_else(|| {
            "argument 'expected_runtime_generation' must be a positive integer".to_string()
        })
}

/// Render a kernel response the way the `python`/`r` tools do, so a user-driven
/// run from the UI reads identically to an agent-driven one.
pub fn format_response(resp: &KernelResp) -> String {
    let mut out = String::new();
    if !resp.stdout.is_empty() {
        out.push_str(&resp.stdout);
    }
    if !resp.stderr.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("[stderr] ");
        out.push_str(&resp.stderr);
    }
    if let Some(err) = &resp.error {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("[error] ");
        out.push_str(err);
    }
    if out.is_empty() {
        out = "(no output)".into();
    }
    out
}

/// Prefix a script run's output with the identity of the source and the process
/// that ran it. Public so an editor-driven whole-script run reads identically to
/// an agent-driven one, the way `format_response` already does for cells.
pub fn format_script_response(
    response: &KernelResp,
    script: Option<&ScriptProvenance>,
    runtime: &RuntimeInfo,
) -> String {
    let output = format_response(response);
    let Some(script) = script else {
        return output;
    };
    format!(
        "[runtime script] path={} sha256={} runtime_id={} generation={}\n{}",
        script.path, script.sha256, runtime.runtime_id, runtime.generation, output
    )
}

async fn run_runtime(
    manager: &RuntimeManager,
    key: RuntimeKey,
    source: RuntimeSource,
    required_objects: Vec<String>,
    expected_generation: Option<u64>,
    language: &'static str,
    env: &dyn ToolEnv,
) -> ToolResult {
    if key.context_id == LOCAL_CONTEXT_ID || key.context_id.starts_with("wsl:") {
        if let Err(error) = env.preflight_local_execution(&source.code).await {
            return ToolResult::fail(error).stop_batch();
        }
    }
    let options = RuntimeExecutionOptions {
        source_name: source.script.as_ref().map(|script| script.path.clone()),
        required_objects,
        expected_generation,
    };
    let mut execution = match manager
        .execute_with_options(&key, env.project_root(), source.code, options)
        .await
    {
        Ok(execution) => execution,
        Err(error) => return ToolResult::fail(format!("{language} error: {error}")),
    };
    let runtime = execution.info().clone();
    let mut cancel_poll = tokio::time::interval(std::time::Duration::from_millis(50));
    loop {
        tokio::select! {
            event = execution.recv() => match event {
                Some(RuntimeEvent::Stdout(chunk)) => {
                    env.emit(ToolEvent::Stdout { chunk }).await;
                }
                Some(RuntimeEvent::Finished(Ok(response))) => {
                    report_runtime_writes(env, &key.context_id, &response);
                    let success = response.error.is_none();
                    return ToolResult {
                        success,
                        content: format_script_response(
                            &response,
                            source.script.as_ref(),
                            &runtime,
                        ),
                        image: None,
                        control: wisp_tools::ToolControl::Continue,
                    };
                }
                Some(RuntimeEvent::Finished(Err(error))) => {
                    return ToolResult::fail(format!("{language} error: {error}"));
                }
                None => {
                    return ToolResult::fail(format!(
                        "{language} error: runtime ended before returning a result"
                    ));
                }
            },
            _ = cancel_poll.tick() => {
                if env.is_cancelled() {
                    // Dropping this receiver abandons only the caller. The
                    // manager-owned protocol task still drains the cell.
                    return ToolResult::fail(format!("{language} error: interrupted by user"));
                }
            }
        }
    }
}

#[async_trait]
impl Tool for ReplTool {
    fn name(&self) -> &str {
        "python"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "python",
            PYTHON_TOOL_DESCRIPTION,
            json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "Python code to execute (statements or a single expression)" },
                    "script_path": { "type": "string", "description": "Project-relative .py file whose exact UTF-8 content is executed in this persistent runtime; mutually exclusive with code" },
                    "required_objects": { "type": "array", "items": { "type": "string" }, "maxItems": 64, "description": "Bindings that must already exist in this runtime before execution; a missing/dead/restarted runtime fails instead of lazy-starting empty" },
                    "expected_runtime_generation": { "type": "integer", "minimum": 1, "description": "Optional generation guard from a previous runtime-script result" },
                    "context_id": { "type": "string", "description": "Execution context id; defaults to local (for example local, ssh:gpu, or wsl:Ubuntu)" }
                },
                "oneOf": [
                    { "required": ["code"] },
                    { "required": ["script_path"] }
                ]
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        let context = context_id(args).unwrap_or("invalid");
        let source = args
            .get("script_path")
            .and_then(|value| value.as_str())
            .map(|path| format!("script {path}"))
            .or_else(|| {
                args.get("code")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        format!("[python @ {context}] {source}")
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let source = match source_arg(args, env, "py") {
            Ok(source) => source,
            Err(error) => return ToolResult::fail(error),
        };
        let required_objects = match required_objects_arg(args) {
            Ok(required_objects) => required_objects,
            Err(error) => return ToolResult::fail(error),
        };
        let expected_generation = match expected_generation_arg(args) {
            Ok(expected_generation) => expected_generation,
            Err(error) => return ToolResult::fail(error),
        };
        let context_id = match context_id(args) {
            Ok(context_id) => context_id,
            Err(error) => return ToolResult::fail(error),
        };
        run_runtime(
            &self.manager,
            RuntimeKey::python_in_scope(&self.project_id, &self.scope_key, context_id)
                .with_session(&self.session_id),
            source,
            required_objects,
            expected_generation,
            "python",
            env,
        )
        .await
    }
}

#[async_trait]
impl Tool for RTool {
    fn name(&self) -> &str {
        "r"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "r",
            R_TOOL_DESCRIPTION,
            json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "R code to execute (one or more expressions)" },
                    "script_path": { "type": "string", "description": "Project-relative .R file whose exact UTF-8 content is executed in this persistent runtime; mutually exclusive with code" },
                    "required_objects": { "type": "array", "items": { "type": "string" }, "maxItems": 64, "description": "Bindings that must already exist in this runtime before execution; a missing/dead/restarted runtime fails instead of lazy-starting empty" },
                    "expected_runtime_generation": { "type": "integer", "minimum": 1, "description": "Optional generation guard from a previous runtime-script result" },
                    "context_id": { "type": "string", "description": "Execution context id; defaults to local (for example local, ssh:gpu, or wsl:Ubuntu)" }
                },
                "oneOf": [
                    { "required": ["code"] },
                    { "required": ["script_path"] }
                ]
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        let context = context_id(args).unwrap_or("invalid");
        let source = args
            .get("script_path")
            .and_then(|value| value.as_str())
            .map(|path| format!("script {path}"))
            .or_else(|| {
                args.get("code")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        format!("[r @ {context}] {source}")
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let source = match source_arg(args, env, "R") {
            Ok(source) => source,
            Err(error) => return ToolResult::fail(error),
        };
        let required_objects = match required_objects_arg(args) {
            Ok(required_objects) => required_objects,
            Err(error) => return ToolResult::fail(error),
        };
        let expected_generation = match expected_generation_arg(args) {
            Ok(expected_generation) => expected_generation,
            Err(error) => return ToolResult::fail(error),
        };
        let context_id = match context_id(args) {
            Ok(context_id) => context_id,
            Err(error) => return ToolResult::fail(error),
        };
        run_runtime(
            &self.manager,
            RuntimeKey::r_in_scope(&self.project_id, &self.scope_key, context_id)
                .with_session(&self.session_id),
            source,
            required_objects,
            expected_generation,
            "r",
            env,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        code_arg, context_id, expected_generation_arg, normalized_script_path,
        project_relative_writes, read_project_script, report_runtime_writes, required_objects_arg,
        script_source, source_arg, validated_project_writes, PYTHON_TOOL_DESCRIPTION,
        R_TOOL_DESCRIPTION,
    };
    use crate::{
        KernelResp, LaunchedRuntime, RTool, RuntimeKernel, RuntimeKey, RuntimeLauncher,
        RuntimeManager, RuntimeMetadata, RuntimeObjectList, RuntimeOutput, LOCAL_CONTEXT_ID,
        MAX_CODE_BYTES,
    };
    use anyhow::Result;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use wisp_tools::{Tool, ToolEnv, ToolEvent};

    #[test]
    fn python_description_keeps_package_setup_out_of_the_repl() {
        assert!(PYTHON_TOOL_DESCRIPTION.contains("Do not use this as a package installer"));
        assert!(PYTHON_TOOL_DESCRIPTION.contains("project-local pixi"));
        assert!(PYTHON_TOOL_DESCRIPTION.contains("local-env-setup"));
    }

    #[test]
    fn repl_descriptions_promise_per_conversation_state() {
        for description in [PYTHON_TOOL_DESCRIPTION, R_TOOL_DESCRIPTION] {
            assert!(description.contains("persist per conversation"));
            assert!(description.contains("parallel conversations never share interpreter state"));
            assert!(description.contains("Local and WSL REPLs start in the project root"));
            assert!(description.contains("SSH REPLs use the execution context workdir"));
            assert!(description.contains("script_path"));
            assert!(description.contains("required_objects"));
        }
    }

    #[test]
    fn r_description_requires_existing_runtime_dependencies_and_explicit_plots() {
        assert!(R_TOOL_DESCRIPTION.contains("Rscript"));
        assert!(R_TOOL_DESCRIPTION.contains("jsonlite"));
        assert!(R_TOOL_DESCRIPTION.contains("png()"));
        assert!(R_TOOL_DESCRIPTION.contains("does not install packages"));
    }

    #[test]
    fn context_defaults_to_local_and_rejects_blank_values() {
        assert_eq!(
            context_id(&serde_json::json!({"code": "1"})).unwrap(),
            "local"
        );
        assert!(context_id(&serde_json::json!({"context_id": "  "})).is_err());
        assert_eq!(
            context_id(&serde_json::json!({"context_id": " ssh:gpu "})).unwrap(),
            "ssh:gpu"
        );
    }

    #[test]
    fn code_size_is_rejected_before_runtime_dispatch() {
        let args = serde_json::json!({"code": "x".repeat(MAX_CODE_BYTES + 1)});
        assert!(code_arg(&args).unwrap_err().contains("byte limit"));
    }

    #[test]
    fn runtime_precondition_arguments_are_bounded_and_deduplicated() {
        assert_eq!(
            required_objects_arg(&serde_json::json!({
                "required_objects": ["sce", " sce ", "metadata"]
            }))
            .unwrap(),
            vec!["sce".to_string(), "metadata".to_string()]
        );
        assert!(required_objects_arg(&serde_json::json!({
            "required_objects": [""]
        }))
        .is_err());
        assert_eq!(
            expected_generation_arg(&serde_json::json!({"expected_runtime_generation": 3}))
                .unwrap(),
            Some(3)
        );
        assert!(
            expected_generation_arg(&serde_json::json!({"expected_runtime_generation": 0}))
                .is_err()
        );
    }

    #[test]
    fn runtime_script_paths_must_be_project_relative() {
        assert_eq!(
            normalized_script_path("./analysis/de.R").unwrap().1,
            "analysis/de.R"
        );
        assert!(normalized_script_path("../outside.R").is_err());
        assert!(
            normalized_script_path(&std::env::temp_dir().join("x.R").to_string_lossy()).is_err()
        );
    }

    fn unique_tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "wisp_rel_{tag}_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn project_relative_writes_drops_outside_root_and_normalizes() {
        let root = unique_tmp("writes");
        std::fs::create_dir_all(root.join("out")).unwrap();
        std::fs::write(root.join("out/a.txt"), b"a").unwrap();
        std::fs::write(root.join("out/c.txt"), b"c").unwrap();
        let outside = unique_tmp("writes_out");
        std::fs::write(outside.join("cache"), b"x").unwrap();

        let a = root.join("out/a.txt").to_string_lossy().into_owned();
        let c = root.join("out/c.txt").to_string_lossy().into_owned();
        let mut reported = vec![
            a.clone(),
            a.clone(),
            c,
            outside.join("cache").to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
        ];
        // Backslash spellings collapse into the same entries — but only on
        // Windows, where `\` is a separator rather than a filename character.
        if cfg!(windows) {
            reported.push(format!(
                "{}{}out\\a.txt",
                root.display(),
                std::path::MAIN_SEPARATOR
            ));
            reported.push(format!(
                "{}{}out\\c.txt",
                root.display(),
                std::path::MAIN_SEPARATOR
            ));
        }
        let got = project_relative_writes(&root, &reported);
        assert_eq!(got, vec!["out/a.txt".to_string(), "out/c.txt".to_string()]);
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    /// On Unix a literal `\` is a legal filename character; the spelling
    /// must be preserved so the record names the file that was written,
    /// not a same-identity sibling under a `we/` directory.
    #[cfg(not(windows))]
    #[test]
    fn project_relative_writes_preserves_backslash_filenames_on_unix() {
        let root = unique_tmp("writes_bs");
        std::fs::write(root.join(r"we\ird.txt"), b"x").unwrap();
        let reported = vec![root.join(r"we\ird.txt").to_string_lossy().into_owned()];
        assert_eq!(
            project_relative_writes(&root, &reported),
            vec![r"we\ird.txt".to_string()]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn project_relative_writes_rejects_parent_traversal_in_fallback() {
        // A path that does not exist skips canonicalization, so the raw
        // remainder is stripped verbatim; `..` must never survive into the
        // provenance record, where undo would resolve it outside the root.
        let root = unique_tmp("writes_dotdot");
        let escape = format!(
            "{}{}..{}escaped.txt",
            root.display(),
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        );
        assert!(project_relative_writes(&root, &[escape]).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn configured_project_writes_reject_absolute_traversal_and_missing_paths() {
        let root = unique_tmp("configured_writes");
        std::fs::create_dir_all(root.join("out")).unwrap();
        std::fs::write(root.join("out/a.txt"), b"a").unwrap();
        let absolute = root.join("out/a.txt").to_string_lossy().into_owned();
        assert_eq!(
            validated_project_writes(
                &root,
                &[
                    "out/a.txt".into(),
                    "../escape.txt".into(),
                    absolute,
                    "out/missing.txt".into(),
                ],
            ),
            vec!["out/a.txt".to_string()]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// Captures what the finished-cell branch of `run_runtime` hands to the
    /// agent loop.
    struct RecordingEnv {
        root: PathBuf,
        reported: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl ToolEnv for RecordingEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        async fn emit(&self, _event: ToolEvent) {}
        fn report_written_paths(&self, paths: &[String]) {
            self.reported.lock().unwrap().push(paths.to_vec());
        }
    }

    fn recording_env(root: PathBuf) -> RecordingEnv {
        RecordingEnv {
            root,
            reported: Mutex::new(Vec::new()),
        }
    }

    type SeenExecution = (String, Option<String>, Vec<String>);

    #[derive(Clone, Default)]
    struct EchoLauncher {
        seen: Arc<Mutex<Vec<SeenExecution>>>,
    }

    struct EchoKernel {
        seen: Arc<Mutex<Vec<SeenExecution>>>,
    }

    #[async_trait::async_trait]
    impl RuntimeLauncher for EchoLauncher {
        async fn launch(&self, _key: &RuntimeKey, _cwd: &Path) -> Result<LaunchedRuntime> {
            Ok(LaunchedRuntime::new(
                Box::new(EchoKernel {
                    seen: self.seen.clone(),
                }),
                RuntimeMetadata::default(),
            ))
        }
    }

    #[async_trait::async_trait]
    impl RuntimeKernel for EchoKernel {
        async fn execute(
            &mut self,
            _id: &str,
            code: &str,
            source_name: Option<&str>,
            required_objects: &[String],
            _output: &RuntimeOutput,
        ) -> Result<KernelResp> {
            self.seen.lock().unwrap().push((
                code.to_string(),
                source_name.map(str::to_string),
                required_objects.to_vec(),
            ));
            let missing = required_objects
                .iter()
                .filter(|name| name.as_str() != "sce")
                .cloned()
                .collect::<Vec<_>>();
            Ok(KernelResp {
                stdout: "executed".into(),
                error: (!missing.is_empty()).then(|| {
                    format!(
                        "required runtime objects are missing: {}",
                        missing.join(", ")
                    )
                }),
                ..KernelResp::default()
            })
        }

        async fn inspect(&mut self, _id: &str) -> Result<RuntimeObjectList> {
            Ok(RuntimeObjectList::default())
        }

        async fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn project_script_source_is_hashed_and_code_is_mutually_exclusive() {
        let root = unique_tmp("runtime_script");
        std::fs::create_dir_all(root.join("analysis")).unwrap();
        std::fs::write(root.join("analysis/de.R"), b"answer <- 42\n").unwrap();
        let env = recording_env(root.clone());

        let source = script_source("analysis/de.R", &env, "R").unwrap();
        assert_eq!(source.code, "answer <- 42\n");
        let script = source.script.unwrap();
        assert_eq!(script.path, "analysis/de.R");
        assert_eq!(script.sha256.len(), 64);

        let both = source_arg(
            &serde_json::json!({
                "code": "1 + 1",
                "script_path": "analysis/de.R"
            }),
            &env,
            "R",
        )
        .unwrap_err();
        assert!(both.contains("mutually exclusive"));
        assert!(script_source("analysis/de.R", &env, "py").is_err());
        assert!(script_source("../outside.R", &env, "R").is_err());
        std::fs::write(root.join("analysis/de.r"), b"answer <- 1\n").unwrap();
        let lowercase = read_project_script(&root, "analysis/de.r", "R").unwrap();
        assert_eq!(lowercase.code, "answer <- 1\n");
        assert_eq!(lowercase.provenance.path, "analysis/de.r");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn saved_script_executes_in_the_existing_runtime_with_provenance() {
        let root = unique_tmp("runtime_script_execute");
        std::fs::create_dir_all(root.join("analysis")).unwrap();
        std::fs::write(root.join("analysis/de.R"), b"answer <- nrow(sce)\n").unwrap();
        let env = recording_env(root.clone());
        let launcher = EchoLauncher::default();
        let manager = RuntimeManager::new(Arc::new(launcher.clone()));
        let key = RuntimeKey::r("project-a", LOCAL_CONTEXT_ID);
        let runtime = manager.start(key, root.clone()).await.unwrap();
        let tool = RTool::new(manager.clone(), "project-a");

        let result = tool
            .run(
                &serde_json::json!({
                    "script_path": "analysis/de.R",
                    "required_objects": ["sce"],
                    "expected_runtime_generation": runtime.generation
                }),
                &env,
            )
            .await;

        assert!(result.success, "{}", result.content);
        assert!(result.content.contains("path=analysis/de.R"));
        assert!(result.content.contains("sha256="));
        assert!(result
            .content
            .contains(&format!("generation={}", runtime.generation)));
        assert!(result.content.ends_with("executed"));
        assert_eq!(
            *launcher.seen.lock().unwrap(),
            vec![(
                "answer <- nrow(sce)\n".into(),
                Some("analysis/de.R".into()),
                vec!["sce".into()]
            )]
        );
        manager.shutdown_all().await;
        std::fs::remove_dir_all(&root).ok();
    }

    fn response_with(files_written: Option<Vec<String>>) -> KernelResp {
        KernelResp {
            files_written,
            ..KernelResp::default()
        }
    }

    #[test]
    fn local_kernel_report_reaches_the_tool_environment() {
        let root = unique_tmp("report_local");
        std::fs::write(root.join("fig_1.png"), b"x").unwrap();
        let env = recording_env(root.clone());
        let reported = vec![root.join("fig_1.png").to_string_lossy().into_owned()];

        report_runtime_writes(&env, LOCAL_CONTEXT_ID, &response_with(Some(reported)));

        assert_eq!(
            *env.reported.lock().unwrap(),
            vec![vec!["fig_1.png".to_string()]]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn configured_local_report_reaches_the_tool_environment() {
        let root = unique_tmp("report_configured_local");
        std::fs::write(root.join("fig_1.png"), b"x").unwrap();
        let env = recording_env(root.clone());
        let response = KernelResp {
            files_written: Some(vec!["fig_1.png".into()]),
            files_written_base: Some("project".into()),
            ..KernelResp::default()
        };

        report_runtime_writes(&env, LOCAL_CONTEXT_ID, &response);

        assert_eq!(
            *env.reported.lock().unwrap(),
            vec![vec!["fig_1.png".to_string()]]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn configured_wsl_report_is_forwarded_but_ssh_stays_remote() {
        let root = unique_tmp("report_configured_wsl");
        std::fs::write(root.join("fig_1.png"), b"x").unwrap();
        let response = KernelResp {
            files_written: Some(vec!["fig_1.png".into()]),
            files_written_base: Some("project".into()),
            ..KernelResp::default()
        };
        let wsl = recording_env(root.clone());
        report_runtime_writes(&wsl, "wsl:Ubuntu", &response);
        assert_eq!(
            *wsl.reported.lock().unwrap(),
            vec![vec!["fig_1.png".to_string()]]
        );

        let ssh = recording_env(root.clone());
        report_runtime_writes(&ssh, "ssh:gpu-box", &response);
        assert!(ssh.reported.lock().unwrap().is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    /// A remote or WSL worker's legacy absolute paths describe another
    /// filesystem, and an absent report means "host, keep inferring". Neither
    /// may reach the record.
    #[test]
    fn only_local_legacy_absolute_reports_are_forwarded() {
        let root = unique_tmp("report_gates");
        std::fs::write(root.join("fig_1.png"), b"x").unwrap();
        let inside = vec![root.join("fig_1.png").to_string_lossy().into_owned()];
        let env = recording_env(root.clone());

        report_runtime_writes(&env, "ssh:gpu-box", &response_with(Some(inside.clone())));
        report_runtime_writes(&env, "wsl:Ubuntu", &response_with(Some(inside)));
        report_runtime_writes(&env, LOCAL_CONTEXT_ID, &response_with(None));
        report_runtime_writes(&env, LOCAL_CONTEXT_ID, &response_with(Some(Vec::new())));

        assert!(env.reported.lock().unwrap().is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
