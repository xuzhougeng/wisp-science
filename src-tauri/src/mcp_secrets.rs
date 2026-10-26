//! Glue between MCP connection records and keyring-backed header/env slots.

use wisp_dto::McpSecretEntry;
use wisp_store::mcp_secrets::{
    apply_named_secrets, delete_named_secrets, hydrate_named_secrets, migrate_plaintext,
    persist_json_leaks_secret_values, refresh_has_value, IncomingSecret, McpSecretKind,
    RedactedSecret,
};

use super::{McpConnection, McpTransport};

fn incoming_from_entry(entry: &McpSecretEntry) -> IncomingSecret {
    IncomingSecret {
        name: entry.name.clone(),
        value: entry.value.clone(),
        clear: entry.clear,
    }
}

fn entry_from_redacted(slot: RedactedSecret) -> McpSecretEntry {
    McpSecretEntry::redacted(slot.name, slot.has_value)
}

fn incoming_list(entries: &[McpSecretEntry]) -> Vec<IncomingSecret> {
    entries.iter().map(incoming_from_entry).collect()
}

fn entry_names(entries: &[McpSecretEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| entry.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

fn replace_slots(entries: &mut Vec<McpSecretEntry>, slots: Vec<RedactedSecret>) {
    *entries = slots.into_iter().map(entry_from_redacted).collect();
}

fn migrate_slots(
    connection_id: &str,
    kind: McpSecretKind,
    entries: &mut Vec<McpSecretEntry>,
) -> Result<bool, String> {
    let mut incoming = incoming_list(entries);
    let had_plaintext = incoming.iter().any(|entry| {
        entry
            .value
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    });
    let (redacted, migrated) = migrate_plaintext(connection_id, kind, &mut incoming)?;
    replace_slots(entries, redacted);
    Ok(had_plaintext || migrated)
}

/// Move any leftover plaintext header/env values into the keyring and strip
/// them from the in-memory records. Returns whether settings JSON must be
/// rewritten.
pub(crate) fn migrate_loaded(conns: &mut [McpConnection]) -> Result<bool, String> {
    let mut changed = false;
    for conn in conns.iter_mut() {
        match &mut conn.transport {
            McpTransport::Stdio { env, .. } => {
                changed |= migrate_slots(&conn.id, McpSecretKind::Env, env)?;
            }
            McpTransport::Http { headers, .. } => {
                changed |= migrate_slots(&conn.id, McpSecretKind::Header, headers)?;
            }
        }
        strip_secret_values(conn);
    }
    Ok(changed)
}

pub(crate) fn refresh_listed_has_value(conns: &mut [McpConnection]) {
    for conn in conns.iter_mut() {
        match &mut conn.transport {
            McpTransport::Stdio { env, .. } => {
                let names = entry_names(env);
                replace_slots(env, refresh_has_value(&conn.id, McpSecretKind::Env, &names));
            }
            McpTransport::Http { headers, .. } => {
                let names = entry_names(headers);
                replace_slots(
                    headers,
                    refresh_has_value(&conn.id, McpSecretKind::Header, &names),
                );
            }
        }
    }
}

pub(crate) fn strip_secret_values(conn: &mut McpConnection) {
    match &mut conn.transport {
        McpTransport::Stdio { env, .. } => {
            for entry in env {
                entry.value = None;
                entry.clear = false;
            }
        }
        McpTransport::Http { headers, .. } => {
            for entry in headers {
                entry.value = None;
                entry.clear = false;
            }
        }
    }
}

pub(crate) fn persist_connection_secrets(
    conn: &mut McpConnection,
    previous: Option<&McpConnection>,
) -> Result<(), String> {
    match &mut conn.transport {
        McpTransport::Stdio { env, .. } => {
            let previous_env = match previous.map(|item| &item.transport) {
                Some(McpTransport::Stdio { env, .. }) => entry_names(env),
                Some(McpTransport::Http { headers, .. }) => {
                    delete_named_secrets(&conn.id, McpSecretKind::Header, &entry_names(headers));
                    Vec::new()
                }
                None => Vec::new(),
            };
            let incoming = incoming_list(env);
            let redacted =
                apply_named_secrets(&conn.id, McpSecretKind::Env, &incoming, &previous_env)?;
            replace_slots(env, redacted);
        }
        McpTransport::Http { headers, .. } => {
            let previous_headers = match previous.map(|item| &item.transport) {
                Some(McpTransport::Http { headers, .. }) => entry_names(headers),
                Some(McpTransport::Stdio { env, .. }) => {
                    delete_named_secrets(&conn.id, McpSecretKind::Env, &entry_names(env));
                    Vec::new()
                }
                None => Vec::new(),
            };
            let incoming = incoming_list(headers);
            let redacted = apply_named_secrets(
                &conn.id,
                McpSecretKind::Header,
                &incoming,
                &previous_headers,
            )?;
            replace_slots(headers, redacted);
        }
    }
    strip_secret_values(conn);
    Ok(())
}

pub(crate) fn hydrate_headers(conn: &McpConnection) -> Vec<(String, String)> {
    match &conn.transport {
        McpTransport::Http { headers, .. } => {
            hydrate_named_secrets(&conn.id, McpSecretKind::Header, &incoming_list(headers))
        }
        McpTransport::Stdio { .. } => Vec::new(),
    }
}

pub(crate) fn hydrate_env(conn: &McpConnection) -> Vec<(String, String)> {
    match &conn.transport {
        McpTransport::Stdio { env, .. } => {
            hydrate_named_secrets(&conn.id, McpSecretKind::Env, &incoming_list(env))
        }
        McpTransport::Http { .. } => Vec::new(),
    }
}

pub(crate) fn forget_connection_secrets(conn: &McpConnection) {
    match &conn.transport {
        McpTransport::Stdio { env, .. } => {
            delete_named_secrets(&conn.id, McpSecretKind::Env, &entry_names(env));
        }
        McpTransport::Http { headers, .. } => {
            delete_named_secrets(&conn.id, McpSecretKind::Header, &entry_names(headers));
        }
    }
}

pub(crate) fn hydrated_secret_digest(conn: &McpConnection) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for (name, value) in hydrate_headers(conn) {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    for (name, value) in hydrate_env(conn) {
        hasher.update(name.as_bytes());
        hasher.update([1]);
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    hex::encode(hasher.finalize())
}

pub(crate) fn stored_json_is_redacted(conns: &[McpConnection]) -> bool {
    serde_json::to_value(conns)
        .ok()
        .is_some_and(|value| !persist_json_leaks_secret_values(&value))
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;
    use crate::{McpHttpAuth, McpTransport};

    fn http_conn(id: &str, headers: Vec<McpSecretEntry>) -> McpConnection {
        McpConnection {
            id: id.into(),
            name: "remote".into(),
            enabled: true,
            transport: McpTransport::Http {
                url: "https://example.test/mcp".into(),
                headers,
                auth: McpHttpAuth::None,
            },
            proxy_url: String::new(),
        }
    }

    fn stdio_conn(id: &str, env: Vec<McpSecretEntry>) -> McpConnection {
        McpConnection {
            id: id.into(),
            name: "local".into(),
            enabled: true,
            transport: McpTransport::Stdio {
                command: "python".into(),
                args: vec!["s.py".into()],
                env,
                cwd: None,
            },
            proxy_url: String::new(),
        }
    }

    #[test]
    fn migrate_rewrites_legacy_http_headers() {
        let id = format!("tauri-{}", uuid::Uuid::new_v4());
        let mut conns = [http_conn(
            &id,
            vec![McpSecretEntry::plaintext("Authorization", "secret-value")],
        )];
        assert!(migrate_loaded(&mut conns).unwrap());
        match &conns[0].transport {
            McpTransport::Http { headers, .. } => {
                assert_eq!(headers.len(), 1);
                assert_eq!(headers[0].name, "Authorization");
                assert!(headers[0].has_value);
                assert_eq!(headers[0].value, None);
            }
            _ => panic!("expected http"),
        }
        assert!(stored_json_is_redacted(&conns));
        assert!(!serde_json::to_string(&conns)
            .unwrap()
            .contains("secret-value"));
        forget_connection_secrets(&conns[0]);
    }

    #[test]
    fn persist_keep_and_clear() {
        let id = format!("tauri-{}", uuid::Uuid::new_v4());
        let mut created = http_conn(
            &id,
            vec![McpSecretEntry::plaintext("Authorization", "secret-value")],
        );
        persist_connection_secrets(&mut created, None).unwrap();
        assert_eq!(
            hydrate_headers(&created),
            vec![("Authorization".into(), "secret-value".into())]
        );

        let mut keep = http_conn(&id, vec![McpSecretEntry::redacted("Authorization", true)]);
        persist_connection_secrets(&mut keep, Some(&created)).unwrap();
        assert_eq!(
            hydrate_headers(&keep),
            vec![("Authorization".into(), "secret-value".into())]
        );

        let mut clear = http_conn(&id, vec![]);
        persist_connection_secrets(&mut clear, Some(&keep)).unwrap();
        assert!(hydrate_headers(&clear).is_empty());
        forget_connection_secrets(&clear);
    }

    #[test]
    fn delete_forgets_env_secrets() {
        let id = format!("tauri-{}", uuid::Uuid::new_v4());
        let mut created = stdio_conn(
            &id,
            vec![McpSecretEntry::plaintext("TOKEN", "secret-value")],
        );
        persist_connection_secrets(&mut created, None).unwrap();
        assert_eq!(
            hydrate_env(&created),
            vec![("TOKEN".into(), "secret-value".into())]
        );
        forget_connection_secrets(&created);
        assert!(hydrate_env(&created).is_empty());
    }
}
