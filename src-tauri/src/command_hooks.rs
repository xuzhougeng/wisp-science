//! User command hooks (Settings → Hooks): shell commands that run at turn
//! lifecycle events with a JSON payload on stdin, as in Claude Code / Codex.
//!
//! - Exit 0: success. UserPromptSubmit stdout becomes turn context.
//! - Exit 2: block, with stderr as the reason. It refuses the prompt, blocks
//!   the tool call, feeds back after a tool, or keeps a stopping turn going
//!   once. Later hooks for the same event do not run.
//! - Anything else, a spawn failure or a timeout: reported as `HookFailed`;
//!   the turn carries on.
//!
//! The built-in hooks (automatic review, tool-failure analysis) live in
//! `turn_hooks`; the Hooks page shows both kinds together.

use super::*;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use wisp_dto::{CommandHook, HookEvent};

const SETTING: &str = "command_hooks";
// ponytail: one fixed timeout (Claude Code's default); per-hook timeouts when a hook needs longer.
const TIMEOUT: Duration = Duration::from_secs(60);
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

/// Trim, drop empty commands, clear matchers on non-tool events and reject
/// a matcher that is not a valid regex.
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
    /// Exit-2 stderr of the first blocking hook.
    pub(crate) block: Option<String>,
    /// Exit-0 stdout of every hook that printed something.
    pub(crate) context: String,
    errors: Vec<String>,
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
    let hooks = load(&app.state::<AppState>().store).await;
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
        match exec(command, cwd, input).await {
            Ok(output) => match output.status.code() {
                Some(0) => {
                    let stdout = clip(&output.stdout);
                    if !stdout.is_empty() {
                        if !outcome.context.is_empty() {
                            outcome.context.push('\n');
                        }
                        outcome.context.push_str(&stdout);
                    }
                }
                Some(2) => {
                    let reason = clip(&output.stderr);
                    outcome.block = Some(if reason.is_empty() {
                        format!("`{command}` exited with code 2")
                    } else {
                        reason
                    });
                    break;
                }
                code => outcome.errors.push(format!(
                    "`{command}` exited with {}: {}",
                    code.map_or("a signal".to_string(), |code| format!("code {code}")),
                    clip(&output.stderr)
                )),
            },
            Err(error) => outcome.errors.push(format!("`{command}` {error}")),
        }
    }
    outcome
}

/// Same shell as the `shell` tool: PowerShell on Windows, `sh` elsewhere.
async fn exec(command: &str, cwd: &Path, input: &str) -> Result<std::process::Output, String> {
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
    tokio::time::timeout(TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("timed out after {}s", TIMEOUT.as_secs()))?
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
        let saved = normalize(vec![
            hook(HookEvent::PreToolUse, " shell ", "  ./check.sh  "),
            hook(HookEvent::Stop, "shell", "./stop.sh"),
            hook(HookEvent::Stop, "", "   "),
        ])
        .unwrap();
        assert_eq!(
            saved,
            vec![
                hook(HookEvent::PreToolUse, "shell", "./check.sh"),
                hook(HookEvent::Stop, "", "./stop.sh"),
            ]
        );
        assert!(normalize(vec![hook(HookEvent::PreToolUse, "(", "x")])
            .unwrap_err()
            .contains("Invalid tool matcher"));
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
