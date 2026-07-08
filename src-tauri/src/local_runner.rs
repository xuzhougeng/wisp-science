use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use wisp_llm::{Message, Role};

pub const PROVIDER_CODEX_CLI: &str = "codex_cli";
pub const PROVIDER_CLAUDE_CODE: &str = "claude_code";

#[derive(Debug, Clone)]
pub struct LocalRunnerSettings {
    pub command: String,
    pub profile: String,
    pub sandbox: String,
    pub web_search: bool,
    pub model: String,
    pub claude_command: String,
    pub persistent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRunnerCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub prompt_cwd: String,
    pub image_args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpBridgeLaunch {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerRuntime {
    pub home_dir: PathBuf,
    pub config_path: PathBuf,
    pub env: Vec<(String, String)>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerEvent {
    Text(String),
    Reasoning(String),
    ToolCall {
        name: String,
        preview: String,
    },
    ToolResult {
        name: String,
        ok: bool,
        content: String,
    },
    Diff {
        path: String,
    },
    Usage {
        input: u64,
        output: u64,
    },
    Error(String),
}

pub fn is_codex_cli(provider: &str) -> bool {
    provider.trim() == PROVIDER_CODEX_CLI
}

pub fn is_claude_code(provider: &str) -> bool {
    provider.trim() == PROVIDER_CLAUDE_CODE
}

pub fn is_local_runner(provider: &str) -> bool {
    is_codex_cli(provider) || is_claude_code(provider)
}

pub fn default_runner_sandbox(raw: &str) -> String {
    match raw.trim() {
        "read-only" | "workspace-write" | "danger-full-access" => raw.trim().to_string(),
        _ => "danger-full-access".into(),
    }
}

pub fn build_codex_command(
    settings: &LocalRunnerSettings,
    project_root: &Path,
    attachments: &[String],
    session_id: Option<&str>,
) -> LocalRunnerCommand {
    let session_id = session_id.map(str::trim).filter(|s| !s.is_empty());
    let image_args = attachments
        .iter()
        .filter(|p| is_image_path(p))
        .cloned()
        .collect::<Vec<_>>();
    let use_wsl = should_use_wsl(project_root);
    let prompt_cwd = if use_wsl {
        to_wsl_path(project_root).unwrap_or_else(|| project_root.display().to_string())
    } else {
        project_root.display().to_string()
    };
    let cwd = if use_wsl {
        PathBuf::from(r"C:\Windows\System32")
    } else {
        project_root.to_path_buf()
    };
    let (program, mut args) = resolve_runner_program(settings, use_wsl);
    if settings.web_search {
        args.push("--search".into());
    }
    if !settings.profile.trim().is_empty() {
        args.extend(["--profile".into(), settings.profile.trim().into()]);
    }
    if let Some(session_id) = session_id {
        args.extend([
            "--cd".into(),
            prompt_cwd.clone(),
            "--sandbox".into(),
            default_runner_sandbox(&settings.sandbox),
            "-c".into(),
            "approval_policy=\"never\"".into(),
        ]);
        let model = settings.model.trim();
        if !model.is_empty()
            && !matches!(
                model,
                "inherit" | "default" | "codex-default" | "inherit_local_codex_default"
            )
        {
            args.extend(["--model".into(), model.into()]);
        }
        args.extend([
            "exec".into(),
            "resume".into(),
            "--json".into(),
            "--skip-git-repo-check".into(),
        ]);
        for image in &image_args {
            let image = if use_wsl {
                to_wsl_path(Path::new(image)).unwrap_or_else(|| image.clone())
            } else {
                image.clone()
            };
            args.extend(["--image".into(), image]);
        }
        args.push(session_id.into());
        args.push("-".into());
        return LocalRunnerCommand {
            program,
            args,
            cwd,
            prompt_cwd,
            image_args,
            env: vec![],
        };
    }
    args.extend([
        "exec".into(),
        "--json".into(),
        "--cd".into(),
        prompt_cwd.clone(),
        "--skip-git-repo-check".into(),
        "--sandbox".into(),
        default_runner_sandbox(&settings.sandbox),
        "-c".into(),
        "approval_policy=\"never\"".into(),
    ]);
    let model = settings.model.trim();
    if !model.is_empty()
        && !matches!(
            model,
            "inherit" | "default" | "codex-default" | "inherit_local_codex_default"
        )
    {
        args.extend(["--model".into(), model.into()]);
    }
    args.push("-".into());
    for image in &image_args {
        let image = if use_wsl {
            to_wsl_path(Path::new(image)).unwrap_or_else(|| image.clone())
        } else {
            image.clone()
        };
        args.extend(["--image".into(), image]);
    }
    LocalRunnerCommand {
        program,
        args,
        cwd,
        prompt_cwd,
        image_args,
        env: vec![],
    }
}

pub fn build_claude_code_command(
    settings: &LocalRunnerSettings,
    project_root: &Path,
    session_id: Option<&str>,
) -> LocalRunnerCommand {
    let use_wsl = should_use_wsl(project_root);
    let prompt_cwd = if use_wsl {
        to_wsl_path(project_root).unwrap_or_else(|| project_root.display().to_string())
    } else {
        project_root.display().to_string()
    };
    let cwd = if use_wsl {
        PathBuf::from(r"C:\Windows\System32")
    } else {
        project_root.to_path_buf()
    };
    let (program, mut args) = resolve_claude_program(settings, use_wsl);
    args.push("-p".into());
    args.extend(["--output-format".into(), "stream-json".into()]);
    args.push("--verbose".into());
    args.extend(["--permission-mode".into(), "bypassPermissions".into()]);
    let model = settings.model.trim();
    if !model.is_empty() && !matches!(model, "inherit" | "default" | "claude-default") {
        args.extend(["--model".into(), model.into()]);
    }
    if let Some(session_id) = session_id.map(str::trim).filter(|s| !s.is_empty()) {
        args.extend(["--session-id".into(), session_id.into()]);
    }
    LocalRunnerCommand {
        program,
        args,
        cwd,
        prompt_cwd,
        image_args: vec![],
        env: vec![],
    }
}

pub fn apply_runtime_env(cmd: &mut LocalRunnerCommand, runtime: &RunnerRuntime) {
    cmd.env.extend(runtime.env.clone());
}

pub fn prepare_codex_runtime(
    project_root: &Path,
    bridge: &McpBridgeLaunch,
) -> Result<RunnerRuntime, String> {
    let home_dir = project_root.join(".wisp").join("codex-home");
    prepare_runtime_dir(&home_dir)?;
    let source = user_home_dir().map(|h| h.join(".codex"));
    let mut diagnostics = Vec::new();
    match source.as_deref() {
        Some(src) if src.is_dir() => sync_cli_home(src, &home_dir)?,
        Some(src) => diagnostics.push(format!(
            "Local Codex config directory not found: {}. Wisp generated a minimal CODEX_HOME.",
            src.display()
        )),
        None => diagnostics
            .push("Cannot locate user home directory; Wisp generated a minimal CODEX_HOME.".into()),
    }
    let config_path = home_dir.join("config.toml");
    inject_codex_config_block(&config_path, bridge)?;
    let env_home = runner_env_path(project_root, &home_dir);
    Ok(RunnerRuntime {
        home_dir,
        config_path,
        env: vec![("CODEX_HOME".into(), env_home)],
        diagnostics,
    })
}

pub fn prepare_claude_runtime(
    project_root: &Path,
    bridge: &McpBridgeLaunch,
) -> Result<RunnerRuntime, String> {
    let home_dir = project_root.join(".wisp").join("claude-home");
    prepare_runtime_dir(&home_dir)?;
    let source = user_home_dir().map(|h| h.join(".claude"));
    let mut diagnostics = Vec::new();
    match source.as_deref() {
        Some(src) if src.is_dir() => sync_cli_home(src, &home_dir)?,
        Some(src) => diagnostics.push(format!(
            "Local Claude config directory not found: {}. Wisp generated a minimal CLAUDE_CONFIG_DIR.",
            src.display()
        )),
        None => diagnostics.push(
            "Cannot locate user home directory; Wisp generated a minimal CLAUDE_CONFIG_DIR.".into(),
        ),
    }
    let config_path = home_dir.join("mcp.json");
    write_claude_mcp_config(&config_path, bridge)?;
    let env_home = runner_env_path(project_root, &home_dir);
    Ok(RunnerRuntime {
        home_dir,
        config_path,
        env: vec![("CLAUDE_CONFIG_DIR".into(), env_home)],
        diagnostics,
    })
}

pub fn add_claude_mcp_config(
    cmd: &mut LocalRunnerCommand,
    config_path: &Path,
    project_root: &Path,
) {
    let path = if should_use_wsl(project_root) {
        to_wsl_path(config_path).unwrap_or_else(|| config_path.display().to_string())
    } else {
        config_path.display().to_string()
    };
    cmd.args.extend(["--mcp-config".into(), path]);
}

fn resolve_runner_program(settings: &LocalRunnerSettings, use_wsl: bool) -> (String, Vec<String>) {
    let command = settings.command.trim();
    if !command.is_empty() {
        let mut parts = split_command(command);
        if !parts.is_empty() {
            let program = parts.remove(0);
            return (program, parts);
        }
    }
    if use_wsl {
        ("wsl.exe".into(), vec!["-e".into(), "codex".into()])
    } else {
        (default_windows_codex_program(), vec![])
    }
}

fn resolve_claude_program(settings: &LocalRunnerSettings, use_wsl: bool) -> (String, Vec<String>) {
    let command = settings.claude_command.trim();
    if !command.is_empty() {
        let mut parts = split_command(command);
        if !parts.is_empty() {
            let program = parts.remove(0);
            return (program, parts);
        }
    }
    if use_wsl {
        ("wsl.exe".into(), vec!["-e".into(), "claude".into()])
    } else {
        ("claude".into(), vec![])
    }
}

#[cfg(windows)]
fn default_windows_codex_program() -> String {
    if let Some(path) = find_openai_codex_exe() {
        return path.display().to_string();
    }
    "codex".into()
}

#[cfg(not(windows))]
fn default_windows_codex_program() -> String {
    "codex".into()
}

#[cfg(windows)]
fn find_openai_codex_exe() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)?
        .join("OpenAI")
        .join("Codex")
        .join("bin");
    let entries = std::fs::read_dir(base).ok()?;
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("codex.exe"))
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(modified, _)| *modified);
    candidates.pop().map(|(_, path)| path)
}

fn split_command(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut escape = false;
    for ch in command.chars() {
        if escape {
            cur.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                cur.push(ch);
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn should_use_wsl(project_root: &Path) -> bool {
    let s = project_root.display().to_string().replace('\\', "/");
    s.starts_with("//wsl.localhost/")
        || s.starts_with("//wsl$/")
        || s.starts_with("/home/")
        || s.starts_with("/mnt/")
}

pub fn runner_uses_wsl(project_root: &Path) -> bool {
    should_use_wsl(project_root)
}

fn to_wsl_path(path: &Path) -> Option<String> {
    let raw = path.display().to_string();
    let s = raw.replace('\\', "/");
    for prefix in ["//wsl.localhost/", "//wsl$/"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let mut parts = rest.splitn(2, '/');
            let _distro = parts.next()?;
            let inner = parts.next().unwrap_or("");
            return Some(format!("/{}", inner.trim_start_matches('/')));
        }
    }
    if s.starts_with("/home/") || s.starts_with("/mnt/") {
        return Some(s);
    }
    if raw.len() >= 3 && raw.as_bytes()[1] == b':' {
        let drive = raw.chars().next()?.to_ascii_lowercase();
        let rest = raw[2..].replace('\\', "/");
        return Some(format!("/mnt/{drive}{}", rest));
    }
    None
}

fn prepare_runtime_dir(home_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(home_dir).map_err(|e| {
        format!(
            "Failed to create local runner runtime '{}': {e}",
            home_dir.display()
        )
    })
}

fn user_home_dir() -> Option<PathBuf> {
    dirs::home_dir()
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            Some(PathBuf::from(format!(
                "{}{}",
                drive.to_string_lossy(),
                path.to_string_lossy()
            )))
        })
}

fn runner_env_path(project_root: &Path, path: &Path) -> String {
    if should_use_wsl(project_root) {
        to_wsl_path(path).unwrap_or_else(|| path.display().to_string())
    } else {
        path.display().to_string()
    }
}

fn sync_cli_home(source: &Path, target: &Path) -> Result<(), String> {
    let skip = [
        ".wisp", "cache", ".cache", "logs", "log", "tmp", "temp", "sessions", "history",
    ]
    .into_iter()
    .collect::<HashSet<_>>();
    let entries = fs::read_dir(source).map_err(|e| {
        format!(
            "Failed to read local CLI config directory '{}': {e}",
            source.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{e}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if skip.contains(name_s.as_ref()) || name_s.ends_with(".lock") || name_s.ends_with(".log") {
            continue;
        }
        let dest = target.join(&name);
        let meta = entry.metadata().map_err(|e| format!("{e}"))?;
        if meta.is_dir() {
            if dest.exists() {
                fs::remove_dir_all(&dest).map_err(|e| {
                    format!(
                        "Failed to refresh inherited config dir '{}': {e}",
                        dest.display()
                    )
                })?;
            }
            copy_dir_recursive(&path, &dest)?;
        } else if meta.is_file() {
            fs::copy(&path, &dest).map_err(|e| {
                format!(
                    "Failed to copy inherited config '{}' to '{}': {e}",
                    path.display(),
                    dest.display()
                )
            })?;
        }
    }
    Ok(())
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| {
        format!(
            "Failed to create inherited config dir '{}': {e}",
            target.display()
        )
    })?;
    for entry in fs::read_dir(source).map_err(|e| format!("{e}"))? {
        let entry = entry.map_err(|e| format!("{e}"))?;
        let src = entry.path();
        let dst = target.join(entry.file_name());
        let meta = entry.metadata().map_err(|e| format!("{e}"))?;
        if meta.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else if meta.is_file() {
            fs::copy(&src, &dst).map_err(|e| {
                format!(
                    "Failed to copy inherited config '{}' to '{}': {e}",
                    src.display(),
                    dst.display()
                )
            })?;
        }
    }
    Ok(())
}

const WISP_BLOCK_BEGIN: &str = "# BEGIN WISP BUILTINS";
const WISP_BLOCK_END: &str = "# END WISP BUILTINS";

fn inject_codex_config_block(config_path: &Path, bridge: &McpBridgeLaunch) -> Result<(), String> {
    let existing = fs::read_to_string(config_path).unwrap_or_default();
    let block = codex_config_block(bridge);
    let updated = replace_marked_block(&existing, &block);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
    }
    let mut f = fs::File::create(config_path).map_err(|e| {
        format!(
            "Failed to write Codex runtime config '{}': {e}",
            config_path.display()
        )
    })?;
    f.write_all(updated.as_bytes()).map_err(|e| format!("{e}"))
}

fn replace_marked_block(existing: &str, block: &str) -> String {
    let Some(start) = existing.find(WISP_BLOCK_BEGIN) else {
        let mut out = existing.trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(block);
        out.push('\n');
        return out;
    };
    let Some(rel_end) = existing[start..].find(WISP_BLOCK_END) else {
        let mut out = existing[..start].trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(block);
        out.push('\n');
        return out;
    };
    let end = start + rel_end + WISP_BLOCK_END.len();
    let mut out = String::new();
    out.push_str(existing[..start].trim_end());
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(block);
    out.push_str(existing[end..].trim_start_matches(['\r', '\n']));
    out
}

fn codex_config_block(bridge: &McpBridgeLaunch) -> String {
    format!(
        "{WISP_BLOCK_BEGIN}\n\
[mcp_servers.wisp_bridge]\n\
transport = \"stdio\"\n\
command = {}\n\
args = {}\n\
startup_timeout_sec = 120\n\
{WISP_BLOCK_END}",
        toml_string(&bridge.command),
        toml_string_array(&bridge.args)
    )
}

fn write_claude_mcp_config(config_path: &Path, bridge: &McpBridgeLaunch) -> Result<(), String> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
    }
    let body = serde_json::json!({
        "mcpServers": {
            "wisp_bridge": {
                "command": bridge.command,
                "args": bridge.args
            }
        }
    });
    let data = serde_json::to_vec_pretty(&body).map_err(|e| format!("{e}"))?;
    fs::write(config_path, data).map_err(|e| {
        format!(
            "Failed to write Claude MCP config '{}': {e}",
            config_path.display()
        )
    })
}

fn toml_string_array(values: &[String]) -> String {
    let inner = values
        .iter()
        .map(|s| toml_string(s))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

fn toml_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

pub fn is_image_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    matches!(
        lower.rsplit('.').next(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

pub fn build_prompt(
    project_root: &Path,
    history: &[Message],
    user_message: &str,
    attachments: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("# Wisp local runner\n\n");
    out.push_str("You are running as a local agent for wisp-science. Complete the user's scientific analysis task using the local workspace and your configured tools.\n\n");
    out.push_str("Rules:\n");
    out.push_str("- Do not wait for interactive approval; make reasonable progress within the configured sandbox.\n");
    out.push_str("- Treat attached files as authoritative input data.\n");
    out.push_str("- Wisp skills and Wisp MCP tools are exposed through an MCP server named `wisp_bridge`; when a task needs Wisp capabilities, call `wisp_list_skills`, `wisp_use_skill`, or the bridged MCP tools instead of guessing whether tools exist.\n");
    out.push_str("- Save generated reports, tables, figures, or code artifacts under the project workspace when useful.\n");
    out.push_str("- In the final answer, summarize what you did and mention important output file paths.\n\n");
    out.push_str(&format!(
        "Project workspace: {}\n\n",
        project_root.display()
    ));
    if !attachments.is_empty() {
        out.push_str("Attached files:\n");
        for path in attachments {
            let kind = if is_image_path(path) {
                "image passed via --image"
            } else {
                "file path"
            };
            out.push_str(&format!("- {path} ({kind})\n"));
        }
        out.push('\n');
    }
    let turns = compact_history(history);
    if !turns.is_empty() {
        out.push_str("Recent conversation context:\n\n");
        out.push_str(&turns);
        out.push('\n');
    }
    out.push_str("Current user request:\n\n");
    out.push_str(user_message.trim());
    out.push('\n');
    out
}

fn compact_history(history: &[Message]) -> String {
    let mut lines = Vec::new();
    let keep = history.iter().rev().take(24).cloned().collect::<Vec<_>>();
    for msg in keep.into_iter().rev() {
        match msg.role {
            Role::System => {}
            Role::User => push_history(&mut lines, "User", &msg.content.as_text()),
            Role::Assistant => push_history(&mut lines, "Assistant", &msg.content.as_text()),
            Role::Tool => {
                let name = msg.tool_name.as_deref().unwrap_or("tool");
                push_history(&mut lines, &format!("Tool {name}"), &msg.content.as_text());
            }
        }
    }
    lines.join("\n\n")
}

fn push_history(lines: &mut Vec<String>, role: &str, text: &str) {
    let t = text.trim();
    if t.is_empty() {
        return;
    }
    let t = truncate(t, 4_000);
    lines.push(format!("## {role}\n{t}"));
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let head = limit.saturating_sub(160);
    format!(
        "{}\n...[truncated]...\n{}",
        &text[..floor_boundary(text, head)],
        &text[floor_boundary(text, text.len().saturating_sub(120))..]
    )
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

pub fn parse_codex_jsonl(line: &str) -> Vec<RunnerEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    let mut events = Vec::new();
    let typ = v.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if typ == "error" {
        if let Some(msg) = v.get("message").and_then(|v| v.as_str()) {
            events.push(RunnerEvent::Reasoning(msg.to_string()));
        }
        return events;
    }
    if typ == "turn.completed" {
        if let Some((input, output)) = usage_from(&v) {
            events.push(RunnerEvent::Usage { input, output });
        }
    }
    if typ == "turn.failed" {
        let msg = v
            .get("error")
            .or_else(|| v.get("message"))
            .map(value_preview)
            .unwrap_or_else(|| "Codex turn failed".into());
        events.push(RunnerEvent::Error(msg));
    }
    let item = v.get("item").unwrap_or(&v);
    parse_item(item, &mut events);
    events
}

pub fn codex_session_id_from_jsonl(line: &str) -> Option<String> {
    let v = serde_json::from_str::<Value>(line).ok()?;
    find_codex_session_id(&v)
}

fn find_codex_session_id(v: &Value) -> Option<String> {
    match v {
        Value::Object(map) => {
            for key in [
                "session_id",
                "sessionId",
                "session",
                "conversation_id",
                "conversationId",
                "thread_id",
                "threadId",
            ] {
                if let Some(id) = map
                    .get(key)
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    return Some(id.to_string());
                }
            }
            map.values().find_map(find_codex_session_id)
        }
        Value::Array(items) => items.iter().find_map(find_codex_session_id),
        _ => None,
    }
}

pub fn parse_claude_jsonl(line: &str) -> Vec<RunnerEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    let mut events = Vec::new();
    let typ = v.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match typ {
        "assistant" => {
            if let Some(message) = v.get("message") {
                parse_claude_message(message, &mut events);
            }
        }
        "user" => {
            if let Some(message) = v.get("message") {
                parse_claude_tool_results(message, &mut events);
            }
        }
        "result" => {
            if let Some((input, output)) = usage_from(&v) {
                events.push(RunnerEvent::Usage { input, output });
            }
            let subtype = v.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
            if subtype.starts_with("error") {
                let msg = v
                    .get("error")
                    .or_else(|| v.get("result"))
                    .map(value_preview)
                    .unwrap_or_else(|| "Claude Code runner failed".into());
                events.push(RunnerEvent::Error(msg));
            }
        }
        "error" => {
            let msg = v
                .get("message")
                .or_else(|| v.get("error"))
                .map(value_preview)
                .unwrap_or_else(|| "Claude Code runner failed".into());
            events.push(RunnerEvent::Error(msg));
        }
        _ => {}
    }
    events
}

fn parse_claude_message(message: &Value, events: &mut Vec<RunnerEvent>) {
    if let Some((input, output)) = usage_from(message) {
        events.push(RunnerEvent::Usage { input, output });
    }
    let Some(content) = message.get("content").and_then(|v| v.as_array()) else {
        return;
    };
    for part in content {
        match part.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "text" => {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    if !text.trim().is_empty() {
                        events.push(RunnerEvent::Text(text.to_string()));
                    }
                }
            }
            "thinking" => {
                if let Some(text) = part.get("thinking").and_then(|v| v.as_str()) {
                    if !text.trim().is_empty() {
                        events.push(RunnerEvent::Reasoning(text.to_string()));
                    }
                }
            }
            "tool_use" => {
                let name = part
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool")
                    .to_string();
                let preview = part.get("input").map(value_preview).unwrap_or_default();
                events.push(RunnerEvent::ToolCall {
                    name: format!("claude.{name}"),
                    preview,
                });
            }
            _ => {}
        }
    }
}

fn parse_claude_tool_results(message: &Value, events: &mut Vec<RunnerEvent>) {
    let Some(content) = message.get("content").and_then(|v| v.as_array()) else {
        return;
    };
    for part in content {
        if part.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
            continue;
        }
        let ok = part
            .get("is_error")
            .and_then(|v| v.as_bool())
            .map(|is_error| !is_error)
            .unwrap_or(true);
        let content = part
            .get("content")
            .map(value_preview)
            .unwrap_or_else(|| "tool result".into());
        events.push(RunnerEvent::ToolResult {
            name: "claude.tool".into(),
            ok,
            content,
        });
    }
}

fn parse_item(item: &Value, events: &mut Vec<RunnerEvent>) {
    let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match item_type {
        "agent_message" | "message" => {
            if let Some(text) = item_text(item, &["text", "content"]) {
                events.push(RunnerEvent::Text(text));
            }
        }
        "reasoning" => {
            if let Some(text) = item_text(item, &["text", "summary", "content"]) {
                events.push(RunnerEvent::Reasoning(text));
            }
        }
        "command_execution" => parse_command_item(item, events),
        "mcp_tool_call" | "tool_call" => parse_tool_item(item, events),
        "file_change" | "file_changes" | "patch" => parse_file_item(item, events),
        _ => {}
    }
}

fn item_text(item: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = item.get(*key) else {
            continue;
        };
        if let Some(text) = value_text(value) {
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn value_text(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    let arr = v.as_array()?;
    let text = arr
        .iter()
        .filter_map(|part| {
            part.get("text")
                .or_else(|| part.get("content"))
                .and_then(|v| v.as_str())
        })
        .collect::<Vec<_>>()
        .join("");
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn parse_command_item(item: &Value, events: &mut Vec<RunnerEvent>) {
    let command = item
        .get("command")
        .map(value_preview)
        .unwrap_or_else(|| "command".into());
    let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status == "in_progress" || status == "started" {
        events.push(RunnerEvent::ToolCall {
            name: "codex.command".into(),
            preview: command,
        });
        return;
    }
    let content = item
        .get("output")
        .or_else(|| item.get("stdout"))
        .or_else(|| item.get("stderr"))
        .map(value_preview)
        .unwrap_or_else(|| command.clone());
    events.push(RunnerEvent::ToolResult {
        name: "codex.command".into(),
        ok: status != "failed",
        content,
    });
}

fn parse_tool_item(item: &Value, events: &mut Vec<RunnerEvent>) {
    let name = item
        .get("name")
        .or_else(|| item.get("tool_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("codex.tool")
        .to_string();
    let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status == "in_progress" || status == "started" {
        events.push(RunnerEvent::ToolCall {
            name,
            preview: item.get("arguments").map(value_preview).unwrap_or_default(),
        });
    } else {
        events.push(RunnerEvent::ToolResult {
            name,
            ok: status != "failed",
            content: item
                .get("output")
                .or_else(|| item.get("result"))
                .map(value_preview)
                .unwrap_or_default(),
        });
    }
}

fn parse_file_item(item: &Value, events: &mut Vec<RunnerEvent>) {
    if let Some(path) = item
        .get("path")
        .or_else(|| item.get("file"))
        .and_then(|v| v.as_str())
    {
        events.push(RunnerEvent::Diff { path: path.into() });
    }
    if let Some(paths) = item.get("paths").and_then(|v| v.as_array()) {
        for path in paths.iter().filter_map(|v| v.as_str()) {
            events.push(RunnerEvent::Diff { path: path.into() });
        }
    }
}

fn usage_from(v: &Value) -> Option<(u64, u64)> {
    let usage = v.get("usage")?;
    let input = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    Some((input, output))
}

fn value_preview(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    serde_json::to_string(v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_wsl_codex_command_for_unc_path() {
        let settings = LocalRunnerSettings {
            command: String::new(),
            profile: "glm".into(),
            sandbox: String::new(),
            web_search: true,
            model: "inherit".into(),
            claude_command: String::new(),
            persistent: false,
        };
        let cmd = build_codex_command(
            &settings,
            Path::new(r"\\wsl.localhost\Ubuntu\home\ljx\proj"),
            &["/home/ljx/proj/a.png".into()],
            None,
        );
        assert_eq!(cmd.program, "wsl.exe");
        assert!(cmd.args.contains(&"--search".into()));
        assert!(cmd.args.contains(&"--profile".into()));
        assert!(cmd.args.contains(&"danger-full-access".into()));
        assert!(cmd.args.contains(&"--image".into()));
        assert!(
            cmd.args.iter().position(|a| a == "-").unwrap()
                < cmd.args.iter().position(|a| a == "--image").unwrap()
        );
        assert_eq!(cmd.prompt_cwd, "/home/ljx/proj");
    }

    #[test]
    fn explicit_command_is_respected() {
        let settings = LocalRunnerSettings {
            command: "wsl.exe -e codex".into(),
            profile: String::new(),
            sandbox: "workspace-write".into(),
            web_search: false,
            model: "gpt-5.4".into(),
            claude_command: String::new(),
            persistent: false,
        };
        let cmd = build_codex_command(&settings, Path::new("C:/repo"), &[], None);
        assert_eq!(cmd.program, "wsl.exe");
        assert_eq!(&cmd.args[..2], ["-e", "codex"]);
        assert!(cmd.args.contains(&"--model".into()));
        assert!(cmd.args.contains(&"gpt-5.4".into()));
        assert!(cmd.args.contains(&"workspace-write".into()));
    }

    #[test]
    fn codex_resume_uses_external_session_id() {
        let settings = LocalRunnerSettings {
            command: String::new(),
            profile: String::new(),
            sandbox: "workspace-write".into(),
            web_search: false,
            model: "gpt-5.4".into(),
            claude_command: String::new(),
            persistent: true,
        };
        let cmd = build_codex_command(
            &settings,
            Path::new("/repo"),
            &["fig.png".into()],
            Some("sid-1"),
        );
        assert!(cmd.args.windows(2).any(|w| w == ["exec", "resume"]));
        assert!(cmd.args.contains(&"sid-1".into()));
        assert!(cmd.args.contains(&"--image".into()));
        assert!(cmd.args.contains(&"workspace-write".into()));
    }

    #[test]
    fn prompt_includes_attachments_and_history() {
        let history = vec![
            Message::user("previous question"),
            Message::assistant("previous answer"),
        ];
        let prompt = build_prompt(
            Path::new("/tmp/proj"),
            &history,
            "analyze this",
            &["a.csv".into(), "b.png".into()],
        );
        assert!(prompt.contains("previous question"));
        assert!(prompt.contains("a.csv"));
        assert!(prompt.contains("image passed via --image"));
        assert!(prompt.contains("wisp_bridge"));
        assert!(prompt.contains("analyze this"));
    }

    #[test]
    fn parses_agent_message_and_usage() {
        let events = parse_codex_jsonl(
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"done"}}"#,
        );
        assert_eq!(events, vec![RunnerEvent::Text("done".into())]);
        let events = parse_codex_jsonl(
            r#"{"type":"turn.completed","usage":{"input_tokens":7,"output_tokens":3}}"#,
        );
        assert_eq!(
            events,
            vec![RunnerEvent::Usage {
                input: 7,
                output: 3
            }]
        );
        let events = parse_codex_jsonl(
            r#"{"type":"item.completed","item":{"type":"message","content":[{"type":"output_text","text":"hello"}]}}"#,
        );
        assert_eq!(events, vec![RunnerEvent::Text("hello".into())]);
        let events = parse_codex_jsonl(r#"{"type":"error","message":"Reconnecting..."}"#);
        assert_eq!(
            events,
            vec![RunnerEvent::Reasoning("Reconnecting...".into())]
        );
    }

    #[test]
    fn parses_command_and_diff() {
        let events = parse_codex_jsonl(
            r#"{"type":"item.started","item":{"type":"command_execution","command":"ls","status":"in_progress"}}"#,
        );
        assert_eq!(
            events,
            vec![RunnerEvent::ToolCall {
                name: "codex.command".into(),
                preview: "ls".into()
            }]
        );
        let events = parse_codex_jsonl(
            r#"{"type":"item.completed","item":{"type":"file_change","path":"out.md"}}"#,
        );
        assert_eq!(
            events,
            vec![RunnerEvent::Diff {
                path: "out.md".into()
            }]
        );
    }

    #[test]
    fn builds_and_parses_claude_code_runner() {
        let settings = LocalRunnerSettings {
            command: String::new(),
            profile: String::new(),
            sandbox: "danger-full-access".into(),
            web_search: false,
            model: "claude-sonnet-5".into(),
            claude_command: "claude.exe --dangerously-skip-permissions".into(),
            persistent: true,
        };
        let cmd = build_claude_code_command(
            &settings,
            Path::new("C:/repo"),
            Some("123e4567-e89b-12d3-a456-426614174000"),
        );
        assert_eq!(cmd.program, "claude.exe");
        assert!(cmd.args.contains(&"-p".into()));
        assert!(cmd.args.contains(&"stream-json".into()));
        assert!(cmd.args.contains(&"--model".into()));
        assert!(cmd.args.contains(&"--session-id".into()));
        assert!(cmd
            .args
            .contains(&"123e4567-e89b-12d3-a456-426614174000".into()));
        let events = parse_claude_jsonl(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"},{"type":"tool_use","name":"Bash","input":{"command":"pwd"}}],"usage":{"input_tokens":4,"output_tokens":2}}}"#,
        );
        assert_eq!(
            events,
            vec![
                RunnerEvent::Usage {
                    input: 4,
                    output: 2
                },
                RunnerEvent::Text("hi".into()),
                RunnerEvent::ToolCall {
                    name: "claude.Bash".into(),
                    preview: r#"{"command":"pwd"}"#.into()
                }
            ]
        );
    }

    #[test]
    fn adds_claude_mcp_config_without_dropping_session_args() {
        let settings = LocalRunnerSettings {
            command: String::new(),
            profile: String::new(),
            sandbox: String::new(),
            web_search: false,
            model: "inherit".into(),
            claude_command: "claude".into(),
            persistent: true,
        };
        let mut cmd = build_claude_code_command(&settings, Path::new("C:/repo"), Some("sid"));
        add_claude_mcp_config(
            &mut cmd,
            Path::new("C:/repo/.wisp/claude-home/mcp.json"),
            Path::new("C:/repo"),
        );
        assert!(cmd.args.contains(&"--session-id".into()));
        assert!(cmd.args.contains(&"sid".into()));
        assert!(cmd.args.contains(&"--mcp-config".into()));
        assert!(cmd
            .args
            .contains(&"C:/repo/.wisp/claude-home/mcp.json".into()));
    }

    #[test]
    fn extracts_codex_session_id_from_jsonl() {
        assert_eq!(
            codex_session_id_from_jsonl(r#"{"type":"session.created","session_id":"abc-123"}"#)
                .as_deref(),
            Some("abc-123")
        );
        assert_eq!(
            codex_session_id_from_jsonl(r#"{"type":"event","payload":{"threadId":"thread-7"}}"#)
                .as_deref(),
            Some("thread-7")
        );
    }

    #[test]
    fn codex_config_block_preserves_user_config_and_replaces_old_block() {
        let bridge = McpBridgeLaunch {
            command: r"C:\Wisp\wisp-tauri.exe".into(),
            args: vec![
                "--wisp-mcp-bridge".into(),
                "--project-root".into(),
                r"C:\repo".into(),
            ],
        };
        let original = r#"model = "gpt-5"

# BEGIN WISP BUILTINS
[mcp_servers.wisp_bridge]
command = "old"
# END WISP BUILTINS

[profiles.default]
model = "local"
"#;
        let updated = replace_marked_block(original, &codex_config_block(&bridge));
        assert!(updated.contains(r#"model = "gpt-5""#));
        assert!(updated.contains("[profiles.default]"));
        assert!(updated.contains("transport = \"stdio\""));
        assert!(updated.contains("startup_timeout_sec = 120"));
        assert!(!updated.contains("command = \"old\""));
        assert!(updated.contains("wisp-tauri.exe"));
    }

    #[test]
    fn sync_cli_home_skips_cache_and_copies_config_assets() {
        let base = std::env::temp_dir().join(format!(
            "wisp-runner-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let src = base.join("src");
        let dst = base.join("dst");
        std::fs::create_dir_all(src.join("skills").join("s")).unwrap();
        std::fs::create_dir_all(src.join("cache")).unwrap();
        std::fs::write(src.join("config.toml"), "model = 'x'").unwrap();
        std::fs::write(src.join("auth.json"), "{}").unwrap();
        std::fs::write(src.join("skills").join("s").join("SKILL.md"), "body").unwrap();
        std::fs::write(src.join("cache").join("stale"), "no").unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        sync_cli_home(&src, &dst).unwrap();
        assert!(dst.join("config.toml").is_file());
        assert!(dst.join("auth.json").is_file());
        assert!(dst.join("skills").join("s").join("SKILL.md").is_file());
        assert!(!dst.join("cache").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn runtime_env_is_added_to_command() {
        let mut cmd = build_codex_command(
            &LocalRunnerSettings {
                command: "codex".into(),
                profile: String::new(),
                sandbox: String::new(),
                web_search: false,
                model: "inherit".into(),
                claude_command: String::new(),
                persistent: false,
            },
            Path::new("C:/repo"),
            &[],
            None,
        );
        let rt = RunnerRuntime {
            home_dir: PathBuf::from("C:/repo/.wisp/codex-home"),
            config_path: PathBuf::from("C:/repo/.wisp/codex-home/config.toml"),
            env: vec![("CODEX_HOME".into(), "C:/repo/.wisp/codex-home".into())],
            diagnostics: vec![],
        };
        apply_runtime_env(&mut cmd, &rt);
        assert_eq!(
            cmd.env,
            vec![("CODEX_HOME".into(), "C:/repo/.wisp/codex-home".into())]
        );
    }
}
