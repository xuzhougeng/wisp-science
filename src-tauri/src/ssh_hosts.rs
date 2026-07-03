//! SSH host registry: model, pure transforms, and tauri commands. The agent
//! reaches these hosts with its existing `shell` tool (`ssh <alias> '<cmd>'`);
//! this module just tracks which hosts exist and tells the agent about them.

use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SshHost {
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

pub fn upsert_host(mut hosts: Vec<SshHost>, host: SshHost) -> Vec<SshHost> {
    if let Some(existing) = hosts.iter_mut().find(|h| h.alias == host.alias) {
        *existing = host;
    } else {
        hosts.push(host);
    }
    hosts
}

pub fn remove_host(mut hosts: Vec<SshHost>, alias: &str) -> Vec<SshHost> {
    hosts.retain(|h| h.alias != alias);
    hosts
}

/// Parse `Host` aliases from an ~/.ssh/config body. Skips wildcard patterns
/// (`*`, `?` — those are match rules, not connectable hosts) and dedupes,
/// preserving first-seen order.
pub fn parse_ssh_config_aliases(config: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let Some(kw) = parts.next() else { continue };
        if !kw.eq_ignore_ascii_case("host") {
            continue;
        }
        for alias in parts {
            if alias.contains('*') || alias.contains('?') {
                continue;
            }
            if !out.iter().any(|a| a == alias) {
                out.push(alias.to_string());
            }
        }
    }
    out
}

pub fn render_hosts_section(hosts: &[SshHost]) -> Option<String> {
    if hosts.is_empty() {
        return None;
    }
    let mut s = String::from(
        "## Compute hosts\n\n\
The user has these SSH hosts available. Run remote commands with the shell \
tool: `ssh <alias> '<cmd>'`. Prefer them for heavy jobs; remote paths live on \
the host, not on this machine.\n\n",
    );
    for h in hosts {
        let mut conn = String::new();
        if let Some(u) = &h.user {
            conn.push_str(u);
            conn.push('@');
        }
        conn.push_str(&h.alias);
        if let Some(p) = h.port {
            conn.push_str(&format!(":{p}"));
        }
        s.push_str(&format!("- {} — {}", h.alias, conn));
        if let Some(n) = h.notes.as_deref().filter(|n| !n.trim().is_empty()) {
            s.push_str(&format!(" — {n}"));
        }
        s.push('\n');
    }
    Some(s)
}

const KEY: &str = "ssh_hosts";

async fn load(store: &wisp_store::Store) -> Vec<SshHost> {
    store
        .get_setting(KEY)
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

async fn save(store: &wisp_store::Store, hosts: &[SshHost]) -> Result<(), String> {
    let json = serde_json::to_string(hosts).map_err(|e| e.to_string())?;
    store.set_setting(KEY, &json).await.map_err(|e| e.to_string())
}

/// Public: read the persisted hosts for system-prompt injection (Task 5).
pub async fn stored_hosts(store: &wisp_store::Store) -> Vec<SshHost> {
    load(store).await
}

#[tauri::command]
pub async fn list_ssh_hosts(state: State<'_, crate::AppState>) -> Result<Vec<SshHost>, String> {
    Ok(load(&state.store).await)
}

#[tauri::command]
pub async fn add_ssh_host(state: State<'_, crate::AppState>, host: SshHost) -> Result<Vec<SshHost>, String> {
    if host.alias.trim().is_empty() {
        return Err("Alias is required.".into());
    }
    let hosts = upsert_host(load(&state.store).await, host);
    save(&state.store, &hosts).await?;
    Ok(hosts)
}

#[tauri::command]
pub async fn remove_ssh_host(state: State<'_, crate::AppState>, alias: String) -> Result<Vec<SshHost>, String> {
    let hosts = remove_host(load(&state.store).await, &alias);
    save(&state.store, &hosts).await?;
    Ok(hosts)
}

#[tauri::command]
pub async fn list_ssh_config_aliases() -> Result<Vec<String>, String> {
    let text = dirs::home_dir()
        .map(|h| h.join(".ssh").join("config"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    Ok(parse_ssh_config_aliases(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(alias: &str, notes: Option<&str>) -> SshHost {
        SshHost { alias: alias.into(), user: None, port: None, identity_file: None, notes: notes.map(Into::into) }
    }

    #[test]
    fn upsert_adds_new_and_replaces_by_alias_in_place() {
        let list = vec![host("a", Some("first")), host("b", None)];
        let added = upsert_host(list, host("c", None));
        assert_eq!(added.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);

        let replaced = upsert_host(added, host("a", Some("second")));
        assert_eq!(replaced.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(replaced[0].notes.as_deref(), Some("second"));
    }

    #[test]
    fn remove_drops_matching_alias() {
        let list = vec![host("a", None), host("b", None)];
        let out = remove_host(list, "a");
        assert_eq!(out.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn parses_host_aliases_skips_wildcards_and_dedupes() {
        let cfg = "\
Host gpu-box lab-gpu
    HostName 10.0.0.5
    User alice

Host *
    ForwardAgent yes

Host biowulf
    HostName biowulf.nih.gov

Host gpu-box
    Port 2222
";
        assert_eq!(
            parse_ssh_config_aliases(cfg),
            vec!["gpu-box".to_string(), "lab-gpu".to_string(), "biowulf".to_string()]
        );
    }

    #[test]
    fn render_empty_is_none() {
        assert!(render_hosts_section(&[]).is_none());
    }

    #[test]
    fn render_lists_conn_and_notes() {
        let hosts = vec![
            SshHost { alias: "gpu".into(), user: Some("alice".into()), port: Some(2222), identity_file: None, notes: Some("slurm; sbatch".into()) },
            host("plain", None),
        ];
        let s = render_hosts_section(&hosts).unwrap();
        assert!(s.starts_with("## Compute hosts"), "{s}");
        assert!(s.contains("ssh <alias>"), "must teach the shell invocation:\n{s}");
        assert!(s.contains("alice@gpu:2222"), "conn missing:\n{s}");
        assert!(s.contains("slurm; sbatch"), "notes missing:\n{s}");
        assert!(s.contains("- plain"), "bare alias missing:\n{s}");
    }
}
