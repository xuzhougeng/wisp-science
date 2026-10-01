//! User command hooks (Settings → Hooks): shell commands that run at turn
//! lifecycle events with a JSON payload on stdin, as in Claude Code / Codex.
//!
//! Hooks come from the app settings (user scope) and then the active
//! project's `.wisp/hooks.json`. Project hooks run only while the file has the
//! exact content the user trusted on the Hooks page.
//!
//! - Exit 0: success. stdout may be a JSON decision (`decision` or
//!   `hookSpecificOutput.permissionDecision`: block / deny / ask / allow, with
//!   `reason` and `additionalContext`). Plain UserPromptSubmit stdout becomes
//!   turn context.
//! - Exit 2 or a block decision: block, with stderr (or `reason`) as the
//!   reason. It refuses the prompt, blocks the tool call, feeds back after a
//!   tool, or keeps a stopping turn going. Later hooks for the same event do
//!   not run.
//! - Anything else, invalid JSON, a spawn failure or a timeout: reported as
//!   `HookFailed`. PreToolUse fails closed and blocks the call; other events
//!   carry on.
//!
//! The built-in hooks (automatic review, tool-failure analysis) live in
//! `turn_hooks`; the Hooks page shows both kinds together.

use super::*;
use sha2::{Digest, Sha256};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use wisp_dto::{CommandHook, HookEvent, ProjectHooks};

const SETTING: &str = "command_hooks";
/// `{project_id: sha256}` of each project's trusted `.wisp/hooks.json`.
const TRUST_SETTING: &str = "project_hooks_trust";
const PROJECT_FILE: &str = ".wisp/hooks.json";
/// `HookFailed::hook` for the untrusted-project notice.
pub(crate) const PROJECT_HOOKS: &str = "project_hooks";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_TEXT_CHARS: usize = 10_000;

pub(crate) async fn load(store: &Store) -> Vec<CommandHook> {
    store
        .get_setting(SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub(crate) async fn get_command_hooks(
    state: State<'_, AppState>,
) -> Result<Vec<CommandHook>, String> {
    Ok(load(&state.store).await)
}

#[tauri::command]
pub(crate) async fn set_command_hooks(
    state: State<'_, AppState>,
    hooks: Vec<CommandHook>,
) -> Result<Vec<CommandHook>, String> {
    let hooks = normalize(hooks)?;
    let json = serde_json::to_string(&hooks).map_err(|error| error.to_string())?;
    state
        .store
        .set_setting(SETTING, &json)
        .await
        .map_err(|error| error.to_string())?;
    Ok(hooks)
}

/// The active project's `.wisp/hooks.json`; `None` without a project or file.
#[tauri::command]
pub(crate) async fn get_project_hooks(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
) -> Result<Option<ProjectHooks>, String> {
    let Ok(project) = state.require_active(window.label()) else {
        return Ok(None);
    };
    Ok(project_hooks(&state.store, &project.id, &project.root).await)
}

/// Trust the reviewed `.wisp/hooks.json` (`sha256` from `get_project_hooks`)
/// or revoke trust with `None`. A file that changed since it was shown is
/// refused, so only commands the user has seen can run.
#[tauri::command]
pub(crate) async fn set_project_hooks_trust(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    sha256: Option<String>,
) -> Result<Option<ProjectHooks>, String> {
    let project = state.require_active(window.label())?;
    let current = project_hooks(&state.store, &project.id, &project.root).await;
    let mut trust = trusted_hashes(&state.store).await;
    match sha256 {
        Some(sha256) => {
            if !current
                .as_ref()
                .is_some_and(|file| file.sha256 == sha256 && file.error.is_none())
            {
                return Err(format!(
                    "{PROJECT_FILE} changed since it was shown. Review it again."
                ));
            }
            trust.insert(project.id.clone(), sha256);
        }
        None => {
            trust.remove(&project.id);
        }
    }
    let json = serde_json::to_string(&trust).map_err(|error| error.to_string())?;
    state
        .store
        .set_setting(TRUST_SETTING, &json)
        .await
        .map_err(|error| error.to_string())?;
    Ok(project_hooks(&state.store, &project.id, &project.root).await)
}

async fn trusted_hashes(store: &Store) -> BTreeMap<String, String> {
    store
        .get_setting(TRUST_SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

// ponytail: trust covers hooks.json only; a script it runs (e.g. `.wisp/hooks/check.py`)
// can change without re-review, as in Claude Code / Codex. Hash referenced files if that matters.
async fn project_hooks(store: &Store, project_id: &str, root: &Path) -> Option<ProjectHooks> {
    let path = root.join(".wisp").join("hooks.json");
    let bytes = tokio::fs::read(&path).await.ok()?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let parsed = String::from_utf8(bytes)
        .map_err(|error| error.to_string())
        .and_then(|text| parse_project_file(&text));
    let trusted = trusted_hashes(store).await.get(project_id) == Some(&sha256);
    let (hooks, error) = match parsed {
        Ok(hooks) => (hooks, None),
        Err(error) => (Vec::new(), Some(format!("Invalid {PROJECT_FILE}: {error}"))),
    };
    Some(ProjectHooks {
        path: path.display().to_string(),
        hooks,
        sha256,
        trusted,
        error,
    })
}

/// Claude Code / Codex `hooks.json`:
/// `{"hooks": {"PreToolUse": [{"matcher": "shell", "hooks": [{"type": "command", "command": "…", "timeout": 10}]}]}}`.
/// Events Wisp does not fire and non-command handlers are skipped.
fn parse_project_file(text: &str) -> Result<Vec<CommandHook>, String> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        hooks: BTreeMap<String, Vec<Group>>,
    }
    #[derive(Deserialize)]
    struct Group {
        #[serde(default)]
        matcher: String,
        #[serde(default)]
        hooks: Vec<Handler>,
    }
    #[derive(Deserialize)]
    struct Handler {
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        command: String,
        timeout: Option<u64>,
    }
    let file: File = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let mut hooks = Vec::new();
    for (event, groups) in file.hooks {
        let Some(event) = HookEvent::ALL
            .into_iter()
            .find(|known| known.as_str() == event)
        else {
            continue;
        };
        for group in groups {
            for handler in group.hooks.into_iter().filter(|h| h.kind == "command") {
                hooks.push(CommandHook {
                    event,
                    matcher: group.matcher.clone(),
                    command: handler.command,
                    enabled: true,
                    timeout: handler.timeout,
                });
            }
        }
    }
    normalize(hooks)
}

/// Trim, drop empty commands and zero timeouts, clear matchers on non-tool
/// events and reject a matcher that is not a valid regex.
fn normalize(hooks: Vec<CommandHook>) -> Result<Vec<CommandHook>, String> {
    hooks
        .into_iter()
        .filter(|hook| !hook.command.trim().is_empty())
        .map(|mut hook| {
            hook.command = hook.command.trim().to_string();
            hook.matcher = if hook.event.matches_tools() {
                hook.matcher.trim().to_string()
            } else {
                String::new()
            };
            hook.timeout = hook.timeout.filter(|secs| *secs > 0);
            matcher(&hook.matcher)
                .map(|_| hook.clone())
                .map_err(|error| format!("Invalid tool matcher '{}': {error}", hook.matcher))
        })
        .collect()
}

/// Empty or `*` matches every tool; otherwise an anchored regex, so
/// `write|edit` names exact tools and `mcp:.*` a family.
fn matcher(pattern: &str) -> Result<Option<regex::Regex>, regex::Error> {
    if pattern.is_empty() || pattern == "*" {
        return Ok(None);
    }
    regex::Regex::new(&format!("^(?:{pattern})$")).map(Some)
}

fn applies(hook: &CommandHook, event: HookEvent, tool: Option<&str>) -> bool {
    hook.enabled
        && hook.event == event
        && match (matcher(&hook.matcher), tool) {
            (Ok(None), _) => true,
            (Ok(Some(pattern)), Some(tool)) => pattern.is_match(tool),
            _ => false,
        }
}

#[derive(Debug, Default)]
pub(crate) struct HookOutcome {
    /// Reason of the first blocking hook.
    pub(crate) block: Option<String>,
    /// A PreToolUse hook asked for the user's approval.
    pub(crate) ask: bool,
    /// `additionalContext` of every hook, plus plain UserPromptSubmit stdout.
    pub(crate) context: String,
    errors: Vec<String>,
}

impl HookOutcome {
    /// What tool hooks send back to the agent: the block reason and context.
    pub(crate) fn feedback(self) -> Option<String> {
        let text = [self.block.unwrap_or_default(), self.context]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        (!text.is_empty()).then_some(text)
    }
}

/// Run the enabled hooks for `event` in `cwd`. Non-blocking failures are
/// logged and surfaced as `HookFailed`.
pub(crate) async fn fire(
    app: &AppHandle,
    frame_id: &str,
    project_id: &str,
    cwd: &Path,
    event: HookEvent,
    tool: Option<&str>,
    fields: serde_json::Value,
) -> HookOutcome {
    let state = app.state::<AppState>();
    let mut hooks = load(&state.store).await;
    match project_hooks(&state.store, project_id, cwd).await {
        Some(file) if file.trusted => hooks.extend(file.hooks),
        Some(file) => notify_untrusted(app, frame_id, project_id, &file),
        None => {}
    }
    if !hooks.iter().any(|hook| applies(hook, event, tool)) {
        return HookOutcome::default();
    }
    let mut input = serde_json::json!({
        "session_id": frame_id,
        "cwd": cwd,
        "hook_event_name": event.as_str(),
    });
    if let (Some(input), serde_json::Value::Object(fields)) = (input.as_object_mut(), fields) {
        input.extend(fields);
    }
    let outcome = run_hooks(&hooks, event, tool, cwd, &input.to_string()).await;
    for message in &outcome.errors {
        tracing::warn!("{} hook failed for {frame_id}: {message}", event.as_str());
        emit_agent_event_in(
            app,
            AgentEvent::HookFailed {
                frame_id: frame_id.to_string(),
                hook: event.as_str().into(),
                message: message.clone(),
            },
            Some(project_id),
        );
    }
    outcome
}

/// Once per project and file content per app run: an untrusted (or invalid)
/// `.wisp/hooks.json` was skipped. The UI shows its own localized notice.
fn notify_untrusted(app: &AppHandle, frame_id: &str, project_id: &str, file: &ProjectHooks) {
    static SHOWN: StdMutex<BTreeSet<String>> = StdMutex::new(BTreeSet::new());
    let key = format!("{project_id}:{}", file.sha256);
    if !SHOWN.lock().unwrap().insert(key) {
        return;
    }
    emit_agent_event_in(
        app,
        AgentEvent::HookFailed {
            frame_id: frame_id.to_string(),
            hook: PROJECT_HOOKS.into(),
            message: file.path.clone(),
        },
        Some(project_id),
    );
}

/// One hook's answer after it ran.
#[derive(Debug, PartialEq)]
enum Decision {
    Continue { ask: bool, context: String },
    Block(String),
}

async fn run_hooks(
    hooks: &[CommandHook],
    event: HookEvent,
    tool: Option<&str>,
    cwd: &Path,
    input: &str,
) -> HookOutcome {
    let mut outcome = HookOutcome::default();
    for hook in hooks.iter().filter(|hook| applies(hook, event, tool)) {
        let command = &hook.command;
        let timeout = Duration::from_secs(hook.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS));
        let decision = exec(command, cwd, input, timeout).await.and_then(|output| {
            match output.status.code() {
                Some(0) => decide(event, command, &clip(&output.stdout)),
                Some(2) => {
                    let reason = clip(&output.stderr);
                    Ok(Decision::Block(if reason.is_empty() {
                        format!("`{command}` exited with code 2")
                    } else {
                        reason
                    }))
                }
                code => Err(format!(
                    "exited with {}: {}",
                    code.map_or("a signal".to_string(), |code| format!("code {code}")),
                    clip(&output.stderr)
                )),
            }
        });
        match decision {
            Ok(Decision::Continue { ask, context }) => {
                outcome.ask |= ask;
                if !context.is_empty() {
                    if !outcome.context.is_empty() {
                        outcome.context.push('\n');
                    }
                    outcome.context.push_str(&context);
                }
            }
            Ok(Decision::Block(reason)) => {
                outcome.block = Some(reason);
                break;
            }
            Err(error) => {
                let error = format!("`{command}` {error}");
                outcome.errors.push(error.clone());
                // Fail closed: a policy hook that crashes, hangs or answers
                // nonsense must not let the call through.
                if event == HookEvent::PreToolUse {
                    outcome.block = Some(format!("{error} (PreToolUse hooks fail closed)"));
                    break;
                }
            }
        }
    }
    outcome
}

/// Read a clean exit's stdout: a JSON decision as in Claude Code / Codex, or
/// plain text, which only UserPromptSubmit keeps (as context).
fn decide(event: HookEvent, command: &str, stdout: &str) -> Result<Decision, String> {
    if !stdout.starts_with('{') {
        let context = if event == HookEvent::UserPromptSubmit {
            stdout.to_string()
        } else {
            String::new()
        };
        return Ok(Decision::Continue {
            ask: false,
            context,
        });
    }
    let json: serde_json::Value =
        serde_json::from_str(stdout).map_err(|error| format!("printed invalid JSON: {error}"))?;
    let specific = &json["hookSpecificOutput"];
    let text = |values: [&serde_json::Value; 2]| {
        values
            .into_iter()
            .filter_map(|value| value.as_str())
            .map(str::trim)
            .find(|text| !text.is_empty())
            .map(str::to_string)
    };
    let reason = text([&specific["permissionDecisionReason"], &json["reason"]]);
    let context =
        text([&specific["additionalContext"], &json["additionalContext"]]).unwrap_or_default();
    let decision = specific["permissionDecision"]
        .as_str()
        .or(json["decision"].as_str())
        .unwrap_or_default();
    match decision {
        "block" | "deny" => Ok(Decision::Block(
            reason.unwrap_or_else(|| format!("`{command}` blocked it")),
        )),
        // `allow` never skips an approval prompt or lifts a Deny.
        "" | "allow" | "approve" => Ok(Decision::Continue {
            ask: false,
            context,
        }),
        "ask" => Ok(Decision::Continue {
            ask: event == HookEvent::PreToolUse,
            context,
        }),
        other => Err(format!("printed an unknown decision `{other}`")),
    }
}

/// Same shell as the `shell` tool: PowerShell on Windows, `sh` elsewhere.
// ponytail: the hook inherits the app's environment like the `shell` tool does; keyring
// secrets (API keys, SSH keys) are never in it. A strict allowlist breaks PowerShell.
async fn exec(
    command: &str,
    cwd: &Path,
    input: &str,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let mut cmd = if cfg!(target_os = "windows") {
        let mut cmd = tokio::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", command]);
        cmd
    } else {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", command]);
        cmd
    };
    cmd.current_dir(cwd)
        .envs(wisp_tools::network::command_proxy_env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    wisp_tools::process::hide_console_async(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|error| format!("failed to start: {error}"))?;
    // Written concurrently so a hook that never reads stdin cannot deadlock
    // on a full pipe; dropping the handle closes stdin.
    if let Some(mut stdin) = child.stdin.take() {
        let input = input.as_bytes().to_vec();
        tokio::spawn(async move {
            let _ = stdin.write_all(&input).await;
        });
    }
    tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| format!("timed out after {}s", timeout.as_secs()))?
        .map_err(|error| error.to_string())
}

fn clip(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim()
        .chars()
        .take(MAX_TEXT_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(event: HookEvent, matcher: &str, command: &str) -> CommandHook {
        CommandHook {
            event,
            matcher: matcher.into(),
            command: command.into(),
            enabled: true,
            timeout: None,
        }
    }

    #[test]
    fn matchers_name_exact_tools_or_a_regex_family() {
        let edits = hook(HookEvent::PreToolUse, "write|edit", "x");
        assert!(applies(&edits, HookEvent::PreToolUse, Some("edit")));
        assert!(!applies(
            &edits,
            HookEvent::PreToolUse,
            Some("edit_notebook")
        ));
        assert!(!applies(&edits, HookEvent::PostToolUse, Some("edit")));
        let mcp = hook(HookEvent::PostToolUse, "mcp:.*", "x");
        assert!(applies(
            &mcp,
            HookEvent::PostToolUse,
            Some("mcp:pubmed_search")
        ));
        assert!(!applies(&mcp, HookEvent::PostToolUse, Some("shell")));
        for every in ["", "*"] {
            assert!(applies(
                &hook(HookEvent::PreToolUse, every, "x"),
                HookEvent::PreToolUse,
                Some("shell")
            ));
        }
        assert!(applies(
            &hook(HookEvent::Stop, "", "x"),
            HookEvent::Stop,
            None
        ));
        let mut off = hook(HookEvent::Stop, "", "x");
        off.enabled = false;
        assert!(!applies(&off, HookEvent::Stop, None));
    }

    #[test]
    fn saving_trims_drops_empty_commands_and_rejects_bad_regex() {
        let mut zero = hook(HookEvent::Stop, "", "./zero.sh");
        zero.timeout = Some(0);
        let saved = normalize(vec![
            hook(HookEvent::PreToolUse, " shell ", "  ./check.sh  "),
            hook(HookEvent::Stop, "shell", "./stop.sh"),
            hook(HookEvent::Stop, "", "   "),
            zero,
        ])
        .unwrap();
        assert_eq!(
            saved,
            vec![
                hook(HookEvent::PreToolUse, "shell", "./check.sh"),
                hook(HookEvent::Stop, "", "./stop.sh"),
                hook(HookEvent::Stop, "", "./zero.sh"),
            ]
        );
        assert!(normalize(vec![hook(HookEvent::PreToolUse, "(", "x")])
            .unwrap_err()
            .contains("Invalid tool matcher"));
    }

    #[test]
    fn project_files_use_the_claude_code_layout() {
        let hooks = parse_project_file(
            r#"{"hooks": {
                "PreToolUse": [{"matcher": "shell", "hooks": [
                    {"type": "command", "command": "python3 .wisp/hooks/deny.py", "timeout": 10},
                    {"type": "prompt", "prompt": "skipped: not a command"}
                ]}],
                "Stop": [{"hooks": [{"type": "command", "command": "./verify.sh"}]}],
                "SessionStart": [{"hooks": [{"type": "command", "command": "skipped: not fired"}]}]
            }}"#,
        )
        .unwrap();
        let mut deny = hook(
            HookEvent::PreToolUse,
            "shell",
            "python3 .wisp/hooks/deny.py",
        );
        deny.timeout = Some(10);
        assert_eq!(hooks, vec![deny, hook(HookEvent::Stop, "", "./verify.sh")]);
        assert!(parse_project_file("{").is_err());
        assert!(parse_project_file(
            r#"{"hooks": {"PreToolUse": [{"matcher": "(", "hooks": [{"type": "command", "command": "x"}]}]}}"#
        )
        .unwrap_err()
        .contains("Invalid tool matcher"));
    }

    #[tokio::test]
    async fn project_hooks_load_only_while_the_trusted_content_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store.sqlite")).await.unwrap();
        let root = dir.path().join("project");
        assert_eq!(project_hooks(&store, "p", &root).await, None);

        std::fs::create_dir_all(root.join(".wisp")).unwrap();
        let file = root.join(".wisp").join("hooks.json");
        std::fs::write(
            &file,
            r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "./a.sh"}]}]}}"#,
        )
        .unwrap();
        let seen = project_hooks(&store, "p", &root).await.unwrap();
        assert!(!seen.trusted);
        assert_eq!(seen.hooks, vec![hook(HookEvent::Stop, "", "./a.sh")]);

        let trust = BTreeMap::from([("p".to_string(), seen.sha256.clone())]);
        store
            .set_setting(TRUST_SETTING, &serde_json::to_string(&trust).unwrap())
            .await
            .unwrap();
        assert!(project_hooks(&store, "p", &root).await.unwrap().trusted);
        // Another project with the same file is not trusted by this one.
        assert!(!project_hooks(&store, "q", &root).await.unwrap().trusted);

        std::fs::write(
            &file,
            r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "./b.sh"}]}]}}"#,
        )
        .unwrap();
        assert!(!project_hooks(&store, "p", &root).await.unwrap().trusted);

        std::fs::write(&file, "not json").unwrap();
        let broken = project_hooks(&store, "p", &root).await.unwrap();
        assert!(broken.hooks.is_empty());
        assert!(broken
            .error
            .unwrap()
            .starts_with("Invalid .wisp/hooks.json"));
    }

    #[test]
    fn json_stdout_decides_like_claude_code() {
        let pre = HookEvent::PreToolUse;
        let cont = |ask, context: &str| {
            Ok(Decision::Continue {
                ask,
                context: context.into(),
            })
        };
        assert_eq!(
            decide(
                pre,
                "x",
                r#"{"hookSpecificOutput":{"permissionDecision":"deny","permissionDecisionReason":"no rm"}}"#
            ),
            Ok(Decision::Block("no rm".into()))
        );
        assert_eq!(
            decide(pre, "x", r#"{"decision":"block"}"#),
            Ok(Decision::Block("`x` blocked it".into()))
        );
        assert_eq!(decide(pre, "x", r#"{"decision":"ask"}"#), cont(true, ""));
        assert_eq!(
            decide(HookEvent::Stop, "x", r#"{"decision":"ask"}"#),
            cont(false, "")
        );
        assert_eq!(
            decide(
                HookEvent::PostToolUse,
                "x",
                r#"{"decision":"allow","hookSpecificOutput":{"additionalContext":"ruff: 2 fixed"}}"#
            ),
            cont(false, "ruff: 2 fixed")
        );
        // Plain stdout is context only for UserPromptSubmit.
        assert_eq!(
            decide(HookEvent::PostToolUse, "x", "1 file reformatted"),
            cont(false, "")
        );
        assert_eq!(
            decide(HookEvent::UserPromptSubmit, "x", "queue: gpu"),
            cont(false, "queue: gpu")
        );
        assert!(decide(pre, "x", "{not json")
            .unwrap_err()
            .contains("invalid JSON"));
        assert!(decide(pre, "x", r#"{"decision":"maybe"}"#)
            .unwrap_err()
            .contains("unknown decision"));
    }

    #[tokio::test]
    async fn exit_codes_decide_context_block_and_errors() {
        // Both commands run unchanged under sh and PowerShell.
        let ok = hook(HookEvent::UserPromptSubmit, "", "echo context");
        let fail = hook(HookEvent::UserPromptSubmit, "", "exit 3");
        let block = hook(HookEvent::UserPromptSubmit, "", "exit 2");
        let never = hook(HookEvent::UserPromptSubmit, "", "echo after-block");
        let cwd = std::env::temp_dir();

        let outcome = run_hooks(
            &[ok, fail, block, never],
            HookEvent::UserPromptSubmit,
            None,
            &cwd,
            "{}",
        )
        .await;

        assert_eq!(outcome.context, "context");
        assert_eq!(outcome.errors.len(), 1, "{:?}", outcome.errors);
        assert!(outcome.errors[0].contains("code 3"));
        assert_eq!(
            outcome.block.as_deref(),
            Some("`exit 2` exited with code 2")
        );
    }

    #[tokio::test]
    async fn pre_tool_use_fails_closed_on_errors_and_timeouts() {
        let cwd = std::env::temp_dir();
        let crash = hook(HookEvent::PreToolUse, "", "exit 3");
        let outcome = run_hooks(&[crash], HookEvent::PreToolUse, Some("shell"), &cwd, "{}").await;
        assert!(outcome.block.unwrap().contains("fail closed"));
        assert_eq!(outcome.errors.len(), 1);

        // `sleep` is also a PowerShell alias of Start-Sleep.
        let mut hang = hook(HookEvent::PreToolUse, "", "sleep 5");
        hang.timeout = Some(1);
        let outcome = run_hooks(&[hang], HookEvent::PreToolUse, Some("shell"), &cwd, "{}").await;
        assert!(outcome.block.unwrap().contains("timed out after 1s"));

        // The same failure after a tool only reports.
        let crash = hook(HookEvent::PostToolUse, "", "exit 3");
        let outcome = run_hooks(&[crash], HookEvent::PostToolUse, Some("shell"), &cwd, "{}").await;
        assert_eq!(outcome.block, None);
        assert_eq!(outcome.errors.len(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_payload_arrives_on_stdin_and_stderr_is_the_block_reason() {
        let echo = hook(HookEvent::PreToolUse, "", "cat >&2; exit 2");
        let outcome = run_hooks(
            &[echo],
            HookEvent::PreToolUse,
            Some("shell"),
            &std::env::temp_dir(),
            r#"{"tool_name":"shell"}"#,
        )
        .await;
        assert_eq!(outcome.block.as_deref(), Some(r#"{"tool_name":"shell"}"#));
    }
}
