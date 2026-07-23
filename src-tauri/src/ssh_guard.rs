//! Process-wide SSH connectivity gate.
//!
//! After a hard connectivity/auth failure, further managed SSH attempts for the
//! same context fail immediately without contacting the server. This protects
//! remote hosts (and the caller's IP) from AI tool loops that look like SSH
//! brute-force probes. A successful connection or a user-driven probe clears
//! the block.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a host stays blocked after a connectivity failure.
const COOLDOWN: Duration = Duration::from_secs(15 * 60);

const BLOCKED_MARKER: &str = "SSH connectivity gate blocked";

#[derive(Debug, Clone)]
struct Block {
    until: Instant,
    reason: String,
}

fn blocks() -> &'static Mutex<HashMap<String, Block>> {
    static BLOCKS: OnceLock<Mutex<HashMap<String, Block>>> = OnceLock::new();
    BLOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalize_key(context_id: &str) -> String {
    let trimmed = context_id.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("ssh:") {
        trimmed.to_string()
    } else {
        format!("ssh:{trimmed}")
    }
}

/// True when the error indicates SSH transport/auth failure rather than a
/// remote command that simply returned non-zero after a successful login.
pub fn is_connectivity_failure(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "connection timed out",
        "connect timed out",
        "operation timed out",
        "connection refused",
        "connection reset",
        "connection closed by remote host",
        "no route to host",
        "network is unreachable",
        "could not resolve hostname",
        "could not resolve host",
        "name or service not known",
        "permission denied (publickey",
        "permission denied (keyboard-interactive",
        "permission denied (password",
        "too many authentication failures",
        "host key verification failed",
        "no such identity",
        "identity file",
        "not accessible: no such file or directory",
        "kex_exchange_identification",
        "banner exchange",
        "ssh: connect to host",
        "ssh_dispatch_run_fatal",
        "ssh connectivity gate blocked",
    ]
    .iter()
    .any(|needle| error.contains(needle))
}

pub fn blocked_message(context_id: &str, reason: &str) -> String {
    format!(
        "{BLOCKED_MARKER} for `{context_id}` after a previous failure to protect the server \
         from repeated login attempts (which remote hosts treat as SSH brute force). \
         Do NOT retry with shell `ssh`, alternate `-i` keys, StrictHostKeyChecking changes, \
         `python`/`r` on this context_id, or additional `run_in_context` calls. \
         Report the failure once and ask the user to fix connectivity (unlock IP, fix key, \
         re-probe the environment) then retry manually. Previous error: {reason}"
    )
}

/// Fail immediately when this context is still in the post-failure cooldown.
pub fn assert_allowed(context_id: &str) -> Result<(), String> {
    let key = normalize_key(context_id);
    if key.is_empty() {
        return Ok(());
    }
    let mut map = blocks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(block) = map.get(&key) {
        if Instant::now() < block.until {
            return Err(blocked_message(&key, &block.reason));
        }
        map.remove(&key);
    }
    Ok(())
}

pub fn record_failure(context_id: &str, error: &str) {
    let key = normalize_key(context_id);
    if key.is_empty() || !is_connectivity_failure(error) {
        return;
    }
    // Avoid nesting the gate message as the "previous" reason.
    let reason = error
        .lines()
        .find(|line| !line.contains(BLOCKED_MARKER))
        .unwrap_or(error)
        .trim()
        .chars()
        .take(500)
        .collect::<String>();
    let reason = if reason.is_empty() {
        error.chars().take(500).collect()
    } else {
        reason
    };
    let mut map = blocks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.insert(
        key,
        Block {
            until: Instant::now() + COOLDOWN,
            reason,
        },
    );
}

pub fn record_success(context_id: &str) {
    clear(context_id);
}

/// Clear the gate so a user-driven probe or successful connect can proceed.
pub fn clear(context_id: &str) {
    let key = normalize_key(context_id);
    if key.is_empty() {
        return;
    }
    let mut map = blocks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.remove(&key);
}

/// Snapshot for system-prompt injection: currently blocked contexts.
pub fn blocked_contexts() -> Vec<(String, String)> {
    let now = Instant::now();
    let mut map = blocks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.retain(|_, block| now < block.until);
    map.iter()
        .map(|(id, block)| (id.clone(), block.reason.clone()))
        .collect()
}

/// Whether a shell command looks like it will open an SSH/SCP session.
pub fn shell_looks_like_ssh(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    // Match ssh/scp as standalone commands, not substrings like "session".
    lower
        .split(|c: char| c.is_whitespace() || c == ';' || c == '|' || c == '&' || c == '`')
        .any(|token| {
            let token = token
                .trim_matches(|c| matches!(c, '\'' | '"' | '(' | ')' | '{' | '}'))
                .trim_start_matches("./");
            matches!(token, "ssh" | "scp" | "sftp")
                || token.ends_with("/ssh")
                || token.ends_with("/scp")
                || token.ends_with("/sftp")
                || token.ends_with("\\ssh")
                || token.ends_with("\\scp")
                || token.ends_with("\\sftp")
        })
}

fn push_host_target(targets: &mut Vec<String>, raw: &str) {
    let host = raw
        .split_once(':')
        .filter(|(h, _)| !h.is_empty() && !h.contains('/') && !h.contains('\\'))
        .map(|(h, _)| h)
        .unwrap_or(raw);
    let host = host.split('@').next_back().unwrap_or(host);
    if host.is_empty()
        || host.contains('/')
        || host.contains('\\')
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return;
    }
    if !targets.iter().any(|t| t == host) {
        targets.push(host.to_string());
    }
}

/// Best-effort extraction of SSH targets from a free-form shell command.
/// Returns bare aliases/hostnames (without `ssh:` prefix).
pub fn shell_ssh_targets(cmd: &str) -> Vec<String> {
    if !shell_looks_like_ssh(cmd) {
        return Vec::new();
    }
    let mut targets = Vec::new();
    let mut tokens = cmd.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        let bare = token.trim_start_matches("./");
        let is_scp = matches!(bare, "scp" | "sftp")
            || bare.ends_with("/scp")
            || bare.ends_with("/sftp")
            || bare.ends_with("\\scp")
            || bare.ends_with("\\sftp");
        let is_ssh = matches!(bare, "ssh") || bare.ends_with("/ssh") || bare.ends_with("\\ssh");
        if !is_ssh && !is_scp {
            continue;
        }
        while let Some(next) = tokens.peek().copied() {
            if next == "-p" || next == "-P" || next == "-i" || next == "-F" || next == "-o" {
                tokens.next();
                tokens.next();
                continue;
            }
            if next.starts_with('-') {
                tokens.next();
                continue;
            }
            if is_scp {
                // Prefer remote "host:path" / "user@host:path"; skip bare local paths.
                if next.contains(':')
                    && !next.starts_with('/')
                    && !next.starts_with("./")
                    && !next.starts_with("~/")
                    && !(next.len() > 1 && next.as_bytes()[1] == b':')
                {
                    push_host_target(&mut targets, next);
                }
                tokens.next();
                continue;
            }
            push_host_target(&mut targets, next);
            break;
        }
    }
    targets
}

/// Free-form shell SSH/SCP is always blocked for the agent: remote work must
/// go through registered contexts so alias/user/port/identity match Settings.
pub fn preflight_shell(cmd: &str) -> Result<(), String> {
    if !shell_looks_like_ssh(cmd) {
        return Ok(());
    }
    Err(
        "Free-form shell SSH/SCP is disabled. Use `run_in_context`, `python`, or `r` with the \
         registered `context_id` (for example `ssh:my-host`) so Wisp connects with the configured \
         host settings only. If connectivity is unknown or failed, ask the user to Probe the \
         environment first — do not invent `ssh -i` / port / StrictHostKeyChecking options."
            .into(),
    )
}

/// After a shell command finishes, open the gate for targets it failed to reach.
pub fn note_shell_outcome(cmd: &str, success: bool, detail: &str) {
    if success || !shell_looks_like_ssh(cmd) || !is_connectivity_failure(detail) {
        return;
    }
    let targets = shell_ssh_targets(cmd);
    if targets.is_empty() {
        // Fall back: if we cannot parse a target, gate all known blocked keys only
        // when the command clearly timed out — skip inventing keys.
        return;
    }
    for target in targets {
        record_failure(&target, detail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_id(label: &str) -> String {
        format!("ssh:test-{label}-{}", uuid::Uuid::new_v4())
    }

    #[test]
    fn gate_blocks_after_connectivity_failure_and_clears_on_success() {
        let id = unique_id("block");
        assert!(assert_allowed(&id).is_ok());
        record_failure(
            &id,
            "ssh: connect to host example port 22: Connection timed out",
        );
        let err = assert_allowed(&id).unwrap_err();
        assert!(err.contains(BLOCKED_MARKER), "{err}");
        assert!(err.contains("Do NOT retry"), "{err}");
        record_success(&id);
        assert!(assert_allowed(&id).is_ok());
    }

    #[test]
    fn non_connectivity_errors_do_not_open_the_gate() {
        let id = unique_id("cmdfail");
        record_failure(
            &id,
            "ls: cannot access '/missing': No such file or directory",
        );
        assert!(assert_allowed(&id).is_ok());
    }

    #[test]
    fn shell_target_extraction_handles_options_and_user() {
        let targets = shell_ssh_targets(
            "ssh -o ConnectTimeout=15 -o StrictHostKeyChecking=no -i ~/.ssh/id_ed25519 -p 14536 user@xiyoucloud 'uptime'",
        );
        assert_eq!(targets, vec!["xiyoucloud".to_string()]);
        let scp = shell_ssh_targets("scp -P 22 ./a.txt alice@lab-gpu:/tmp/");
        assert_eq!(scp, vec!["lab-gpu".to_string()]);
    }

    #[test]
    fn preflight_shell_always_blocks_free_form_ssh() {
        let err = preflight_shell("ssh user@lab-gpu uptime").unwrap_err();
        assert!(err.contains("Free-form shell SSH/SCP is disabled"), "{err}");
        assert!(err.contains("run_in_context"), "{err}");
        assert!(preflight_shell(r#"rsync -a -e "ssh -p 2222" a/ host:b/"#).is_err());
        assert!(preflight_shell("'/usr/bin/scp' a host:b").is_err());
        assert!(preflight_shell("ls -la").is_ok());
    }

    #[test]
    fn note_shell_outcome_opens_gate_for_parsed_target() {
        let alias = format!("host-{}", uuid::Uuid::new_v4());
        let cmd = format!("ssh -o ConnectTimeout=10 {alias} true");
        note_shell_outcome(
            &cmd,
            false,
            &format!("ssh: connect to host {alias} port 22: Connection timed out"),
        );
        let err = assert_allowed(&alias).unwrap_err();
        assert!(err.contains(BLOCKED_MARKER), "{err}");
        clear(&alias);
    }
}
