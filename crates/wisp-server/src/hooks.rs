//! Command hooks, as the WebView's Settings → Hooks keeps them: shell
//! commands that run at points of a turn with a JSON payload on stdin, as in
//! Claude Code and Codex. Yours are kept under `command_hooks`; a project's
//! `.wisp/hooks.json` runs only while the file has the exact content you
//! trusted (`project_hooks_trust`, the file's SHA-256 by project).
//!
//! - Exit 0: carry on. Stdout may be a JSON decision (`decision`, or
//!   `hookSpecificOutput.permissionDecision`: block, deny, ask or allow, with
//!   `reason` and `additionalContext`). Plain stdout of UserPromptSubmit is
//!   context for the turn.
//! - Exit 2, or a block decision: block, with stderr (or `reason`) as why. It
//!   refuses the message, blocks the tool call, is fed back after a tool, or
//!   keeps a stopping turn going once. Later hooks of the event do not run.
//! - Anything else, invalid JSON, a failure to start or a timeout is a
//!   failure the conversation shows. PreToolUse fails closed and blocks the
//!   call; the other events carry on.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use wisp_dto::{CommandHook, HookEvent, ProjectHooks};
use wisp_store::Store;

use crate::Server;

const SETTING: &str = "command_hooks";
const TRUST_SETTING: &str = "project_hooks_trust";
const PROJECT_FILE: &str = ".wisp/hooks.json";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_TEXT_CHARS: usize = 10_000;
/// How many hooks one list may hold.
const LIMIT: usize = 100;

/// Your hooks, as saved.
pub(crate) async fn load(store: &Store) -> Vec<CommandHook> {
    store
        .get_setting(SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
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

/// A project's `.wisp/hooks.json`, with whether it is trusted as it is now;
/// `None` when there is no file. Trust covers that file only: a script it
/// runs can change without being reviewed again, as in Claude Code.
pub(crate) async fn project_hooks(
    store: &Store,
    project: &str,
    root: &Path,
) -> Option<ProjectHooks> {
    let path = root.join(".wisp").join("hooks.json");
    let bytes = tokio::fs::read(&path).await.ok()?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let parsed = String::from_utf8(bytes)
        .map_err(|error| error.to_string())
        .and_then(|text| parse_project_file(&text));
    let trusted = trusted_hashes(store).await.get(project) == Some(&sha256);
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

/// Claude Code and Codex's `hooks.json`:
/// `{"hooks": {"PreToolUse": [{"matcher": "shell", "hooks": [{"type": "command", "command": "…", "timeout": 10}]}]}}`.
/// Events Wisp does not fire and handlers that are not commands are left out.
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

/// Trims, drops empty commands and zero timeouts, clears the matcher of an
/// event that is not about a tool, and refuses a matcher that is not a
/// regular expression.
pub(crate) fn normalize(hooks: Vec<CommandHook>) -> Result<Vec<CommandHook>, String> {
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

/// Empty or `*` matches every tool; anything else is an anchored regular
/// expression, so `write|edit` names tools and `mcp:.*` a family.
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

/// What the hooks of one event said.
#[derive(Debug, Default)]
pub(crate) struct HookOutcome {
    /// Why the first hook that blocked did.
    pub(crate) block: Option<String>,
    /// A PreToolUse hook asked for the user's approval.
    pub(crate) ask: bool,
    /// Every hook's `additionalContext`, and plain UserPromptSubmit stdout.
    pub(crate) context: String,
    /// Hooks that failed, each in a line.
    pub(crate) errors: Vec<String>,
}

impl HookOutcome {
    /// What tool hooks give back to the model: why it was blocked, and
    /// their context.
    pub(crate) fn feedback(&self) -> Option<String> {
        let text = [self.block.clone().unwrap_or_default(), self.context.clone()]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        (!text.is_empty()).then_some(text)
    }
}

/// One hook's answer.
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
                // A policy hook that crashes, hangs or answers nonsense does
                // not let the call through.
                if event == HookEvent::PreToolUse {
                    outcome.block = Some(format!("{error} (PreToolUse hooks fail closed)"));
                    break;
                }
            }
        }
    }
    outcome
}

/// Reads a clean exit's stdout: a JSON decision as in Claude Code and
/// Codex, or plain text, which only UserPromptSubmit keeps, as context.
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
        // `allow` never skips an approval or lifts a denial.
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

/// The shell the `shell` tool uses: PowerShell on Windows, `sh` elsewhere.
/// The hook has the app's environment, as that tool does; keychain secrets
/// are never in it.
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
        .envs(wisp_tools::local_env::command_env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    wisp_tools::process::hide_console_async(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|error| format!("failed to start: {error}"))?;
    // Written alongside, so a hook that never reads stdin cannot hang on a
    // full pipe; dropping the handle closes it.
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

/// The hooks one turn runs: yours and a trusted project file's, read when
/// the turn starts.
#[derive(Default)]
pub(crate) struct TurnHooks {
    hooks: Vec<CommandHook>,
    cwd: PathBuf,
    frame: String,
}

impl TurnHooks {
    /// The hooks for a turn of `frame` in `project`. An untrusted or invalid
    /// project file is not run; the second element says so, once per file
    /// content while the app runs.
    pub(crate) async fn load(
        store: &Store,
        project: &str,
        root: &Path,
        frame: &str,
    ) -> (Self, Option<String>) {
        static SHOWN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
        let mut hooks = load(store).await;
        let mut notice = None;
        match project_hooks(store, project, root).await {
            Some(file) if file.trusted => hooks.extend(file.hooks),
            Some(file) => {
                if SHOWN
                    .lock()
                    .unwrap()
                    .insert(format!("{project}:{}", file.sha256))
                {
                    notice = Some(format!(
                        "{} was not run: review and trust it in Settings, Hooks",
                        file.path
                    ));
                }
            }
            None => {}
        }
        hooks.retain(|hook| hook.enabled);
        (
            Self {
                hooks,
                cwd: root.to_path_buf(),
                frame: frame.to_owned(),
            },
            notice,
        )
    }

    /// Runs the hooks of `event` (for `tool`, if it is about one) with
    /// `fields` added to the payload. Nothing runs when none applies.
    pub(crate) async fn fire(
        &self,
        event: HookEvent,
        tool: Option<&str>,
        fields: serde_json::Value,
    ) -> HookOutcome {
        if !self.hooks.iter().any(|hook| applies(hook, event, tool)) {
            return HookOutcome::default();
        }
        let mut input = serde_json::json!({
            "session_id": self.frame,
            "cwd": self.cwd,
            "hook_event_name": event.as_str(),
        });
        if let (Some(input), serde_json::Value::Object(fields)) = (input.as_object_mut(), fields) {
            input.extend(fields);
        }
        let outcome = run_hooks(&self.hooks, event, tool, &self.cwd, &input.to_string()).await;
        for error in &outcome.errors {
            tracing::warn!("{} hook failed for {}: {error}", event.as_str(), self.frame);
        }
        outcome
    }
}

impl Server {
    /// Your command hooks.
    pub async fn command_hooks(&self) -> Result<Vec<CommandHook>> {
        Ok(load(&self.0.store).await)
    }

    /// Saves your command hooks, trimmed; a matcher that is not a regular
    /// expression refuses the whole list. Turns that start afterwards run
    /// them.
    pub async fn set_command_hooks(&self, hooks: Vec<CommandHook>) -> Result<Vec<CommandHook>> {
        if hooks.len() > LIMIT {
            bail!("Keep to {LIMIT} hooks");
        }
        let hooks = normalize(hooks).map_err(anyhow::Error::msg)?;
        self.0
            .store
            .set_setting(SETTING, &serde_json::to_string(&hooks)?)
            .await?;
        Ok(hooks)
    }

    /// The project's `.wisp/hooks.json`, as it is now; `None` without one.
    pub async fn project_hooks(&self, project: &str) -> Result<Option<ProjectHooks>> {
        let root = self.project_root(project).await?;
        Ok(project_hooks(&self.0.store, project, Path::new(&root)).await)
    }

    /// Trusts the project's hooks file as it was shown (`sha256`), or with
    /// `None` takes trust back. A file that changed since is refused, so
    /// only commands that were seen can run.
    pub async fn set_project_hooks_trust(
        &self,
        project: &str,
        sha256: Option<String>,
    ) -> Result<Option<ProjectHooks>> {
        let root = self.project_root(project).await?;
        let store = &self.0.store;
        let current = project_hooks(store, project, Path::new(&root)).await;
        let mut trust = trusted_hashes(store).await;
        match sha256 {
            Some(sha256) => {
                if !current
                    .as_ref()
                    .is_some_and(|file| file.sha256 == sha256 && file.error.is_none())
                {
                    bail!("{PROJECT_FILE} changed since it was shown. Review it again.");
                }
                trust.insert(project.to_owned(), sha256);
            }
            None => {
                trust.remove(project);
            }
        }
        store
            .set_setting(TRUST_SETTING, &serde_json::to_string(&trust)?)
            .await?;
        Ok(project_hooks(store, project, Path::new(&root)).await)
    }
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
    fn matchers_name_exact_tools_or_a_family() {
        let pre = |matcher: &str| hook(HookEvent::PreToolUse, matcher, "x");
        assert!(applies(&pre(""), HookEvent::PreToolUse, Some("shell")));
        assert!(applies(&pre("*"), HookEvent::PreToolUse, Some("shell")));
        assert!(applies(
            &pre("write|edit"),
            HookEvent::PreToolUse,
            Some("edit")
        ));
        // Anchored: a longer name is not matched by a shorter one.
        assert!(!applies(
            &pre("write"),
            HookEvent::PreToolUse,
            Some("write_file")
        ));
        assert!(applies(
            &pre("mcp:.*"),
            HookEvent::PreToolUse,
            Some("mcp:lab")
        ));
        // Another event, or a hook that is off, does not run.
        assert!(!applies(&pre(""), HookEvent::PostToolUse, Some("shell")));
        let mut off = pre("");
        off.enabled = false;
        assert!(!applies(&off, HookEvent::PreToolUse, Some("shell")));
    }

    #[test]
    fn saving_trims_drops_empty_commands_and_refuses_a_bad_matcher() {
        let mut stop = hook(HookEvent::Stop, "shell", "  ./check.sh  ");
        stop.timeout = Some(0);
        let saved = normalize(vec![stop, hook(HookEvent::PreToolUse, "", "  ")]).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].command, "./check.sh");
        // Only tool events keep a matcher.
        assert_eq!(saved[0].matcher, "");
        assert_eq!(saved[0].timeout, None);
        assert!(normalize(vec![hook(HookEvent::PreToolUse, "(", "x")])
            .unwrap_err()
            .contains("Invalid tool matcher"));
    }

    #[test]
    fn project_files_use_the_claude_code_layout() {
        let hooks = parse_project_file(
            r#"{"hooks": {
                "PreToolUse": [{"matcher": "shell", "hooks": [
                    {"type": "command", "command": "./guard.sh", "timeout": 5},
                    {"type": "prompt", "prompt": "not a command"}
                ]}],
                "SessionStart": [{"hooks": [{"type": "command", "command": "ignored"}]}]
            }}"#,
        )
        .unwrap();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].event, HookEvent::PreToolUse);
        assert_eq!(hooks[0].matcher, "shell");
        assert_eq!(hooks[0].timeout, Some(5));
        assert!(parse_project_file("not json").is_err());
    }

    #[test]
    fn a_clean_exit_reads_as_a_decision_or_context() {
        let pre = HookEvent::PreToolUse;
        let cont = |ask: bool, context: &str| {
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
    async fn exit_codes_decide_context_block_and_failures() {
        // Each runs unchanged under sh and PowerShell.
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
    async fn pre_tool_use_fails_closed_on_failures_and_timeouts() {
        let cwd = std::env::temp_dir();
        let crash = hook(HookEvent::PreToolUse, "", "exit 3");
        let outcome = run_hooks(&[crash], HookEvent::PreToolUse, Some("shell"), &cwd, "{}").await;
        assert!(outcome.block.unwrap().contains("fail closed"));
        assert_eq!(outcome.errors.len(), 1);
        // `sleep` is also PowerShell's Start-Sleep.
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
    async fn the_payload_arrives_on_stdin_and_stderr_is_why() {
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
