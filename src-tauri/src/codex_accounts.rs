//! ChatGPT account pool, modeled on CLIProxyAPI's auth-file pool: several
//! signed-in accounts, each with its own keyring entry and quota. Instead of
//! round-robin, exactly one account is active; switching copies its tokens to
//! every ChatGPT model. Quota comes from the same `wham/usage` endpoint the
//! CLIProxyAPI management panel reads. Local sign-ins from Codex CLI and
//! CLIProxyAPI can be imported.

use crate::models;
use std::path::{Path, PathBuf};
use tauri::State;
use wisp_dto::codex_login::{CodexAccount, CodexAccountUsage, CodexImportResult, CodexUsageWindow};
use wisp_llm::codex_auth::{self, CodexCredentials, CodexUsage, UsageWindow};

fn account_view(creds: &CodexCredentials, active_id: &str) -> CodexAccount {
    CodexAccount {
        account_id: creds.account_id.clone(),
        email: codex_auth::account_email_from_access_token(&creds.access_token).unwrap_or_default(),
        plan_type: codex_auth::plan_type_from_access_token(&creds.access_token).unwrap_or_default(),
        active: creds.account_id == active_id,
    }
}

fn active_id() -> String {
    models::load_global_codex()
        .map(|creds| creds.account_id)
        .unwrap_or_default()
}

fn accounts() -> Vec<CodexAccount> {
    // Adopt a sign-in saved before the pool existed, or by `wisp-science login codex`.
    if let Some(active) = models::load_global_codex() {
        if !models::codex_account_ids().contains(&active.account_id) {
            if let Err(error) = models::store_codex_account(&active) {
                tracing::warn!(target: "wisp", %error, "could not add the saved ChatGPT account to the pool");
            }
        }
    }
    let active = active_id();
    models::codex_account_ids()
        .iter()
        .filter_map(|id| models::load_codex_account(id))
        .map(|creds| account_view(&creds, &active))
        .collect()
}

/// Refreshed tokens of the active account also go to its models.
async fn persist(store: &wisp_store::Store, creds: &CodexCredentials) -> Result<(), String> {
    if active_id() == creds.account_id {
        crate::codex_login::activate_codex_account(store, creds).await
    } else {
        models::store_codex_account(creds)
    }
}

#[tauri::command]
pub async fn list_codex_accounts() -> Result<Vec<CodexAccount>, String> {
    Ok(accounts())
}

#[tauri::command]
pub async fn switch_codex_account(
    state: State<'_, crate::AppState>,
    account_id: String,
) -> Result<Vec<CodexAccount>, String> {
    let creds = models::load_codex_account(&account_id)
        .ok_or_else(|| "That ChatGPT account is no longer saved. Sign in again.".to_string())?;
    crate::codex_login::activate_codex_account(&state.store, &creds).await?;
    crate::clear_idle_agents(&state).await;
    Ok(accounts())
}

#[tauri::command]
pub async fn remove_codex_account(account_id: String) -> Result<Vec<CodexAccount>, String> {
    if active_id() == account_id {
        return Err("Switch to another ChatGPT account before removing the active one.".into());
    }
    models::forget_codex_account(&account_id)?;
    Ok(accounts())
}

#[tauri::command]
pub async fn codex_account_usage(
    state: State<'_, crate::AppState>,
    account_id: String,
) -> Result<CodexAccountUsage, String> {
    let saved = models::load_codex_account(&account_id)
        .ok_or_else(|| "That ChatGPT account is no longer saved. Sign in again.".to_string())?;
    let client = crate::network::subscription_http_client();
    let creds = codex_auth::refresh_if_due(&client, saved.clone(), codex_auth::now_ms()).await?;
    if creds != saved {
        persist(&state.store, &creds).await?;
    }
    let usage = codex_auth::fetch_usage(&client, &creds).await?;
    Ok(usage_view(account_id, usage))
}

fn usage_view(account_id: String, usage: CodexUsage) -> CodexAccountUsage {
    let window = |window: UsageWindow| CodexUsageWindow {
        used_percent: window.used_percent,
        window_seconds: window.window_seconds,
        reset_at: window.reset_at,
    };
    CodexAccountUsage {
        account_id,
        plan_type: usage.plan_type,
        limit_reached: usage.limit_reached,
        primary: usage.primary.map(window),
        secondary: usage.secondary.map(window),
    }
}

/// Codex CLI keeps one sign-in in `$CODEX_HOME/auth.json`; CLIProxyAPI keeps
/// one `codex-*.json` per account in its auth directory.
fn local_auth_files(home: &Path, codex_home: Option<PathBuf>) -> Vec<PathBuf> {
    let codex_cli = codex_home
        .unwrap_or_else(|| home.join(".codex"))
        .join("auth.json");
    let mut cliproxy: Vec<PathBuf> = std::fs::read_dir(home.join(".cli-proxy-api"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("codex-") && name.ends_with(".json"))
        })
        .collect();
    cliproxy.sort();
    std::iter::once(codex_cli)
        .chain(cliproxy)
        .filter(|path| path.is_file())
        .collect()
}

/// One entry per account, keeping the copy refreshed last.
fn newest_per_account(found: Vec<CodexCredentials>) -> Vec<CodexCredentials> {
    found.into_iter().fold(Vec::new(), |mut kept, creds| {
        match kept
            .iter_mut()
            .find(|seen: &&mut CodexCredentials| seen.account_id == creds.account_id)
        {
            Some(seen) if creds.expires_at_ms > seen.expires_at_ms => *seen = creds,
            Some(_) => {}
            None => kept.push(creds),
        }
        kept
    })
}

fn read_local_accounts(files: &[PathBuf]) -> Result<Vec<CodexCredentials>, String> {
    if files.is_empty() {
        return Err("No local ChatGPT sign-in was found in ~/.codex/auth.json or ~/.cli-proxy-api. Run `codex login` first, or sign in here.".into());
    }
    let (found, errors): (Vec<_>, Vec<_>) = files
        .iter()
        .map(|path| {
            std::fs::read_to_string(path)
                .map_err(|_| "Could not read the local ChatGPT sign-in file".to_string())
                .and_then(|raw| codex_auth::credentials_from_local_auth(&raw))
        })
        .partition(Result::is_ok);
    let found = newest_per_account(found.into_iter().flatten().collect());
    match (found.is_empty(), errors.into_iter().find_map(Result::err)) {
        (true, Some(error)) => Err(error),
        (true, None) => Err("No importable ChatGPT sign-in was found.".into()),
        (false, _) => Ok(found),
    }
}

#[tauri::command]
pub async fn import_local_codex_accounts(
    state: State<'_, crate::AppState>,
) -> Result<CodexImportResult, String> {
    let home = dirs::home_dir().ok_or_else(|| "Could not find the home folder".to_string())?;
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let found = read_local_accounts(&local_auth_files(&home, codex_home))?;
    for creds in found.iter().filter(|creds| {
        models::load_codex_account(&creds.account_id)
            .is_none_or(|saved| creds.expires_at_ms > saved.expires_at_ms)
    }) {
        persist(&state.store, creds).await?;
    }
    if models::load_global_codex().is_none() {
        crate::codex_login::activate_codex_account(&state.store, &found[0]).await?;
    }
    crate::clear_idle_agents(&state).await;
    Ok(CodexImportResult {
        imported: found.len(),
        accounts: accounts(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn access_token(account: &str, exp: i64, email: &str) -> String {
        let encode = |value: String| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
        let claims = serde_json::json!({
            "exp": exp,
            "https://api.openai.com/profile": {"email": email},
            "https://api.openai.com/auth": {"chatgpt_account_id": account, "chatgpt_plan_type": "plus"},
        });
        format!("{}.{}.sig", encode("{}".into()), encode(claims.to_string()))
    }

    fn codex_cli_file(account: &str, exp: i64) -> String {
        serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {"access_token": access_token(account, exp, "cli@example.test"), "refresh_token": format!("refresh-{exp}"), "id_token": "id"},
            "last_refresh": "2026-09-01T00:00:00Z",
        })
        .to_string()
    }

    #[test]
    fn discovers_codex_cli_and_cliproxy_files_only() {
        let home = tempfile::tempdir().unwrap();
        assert!(local_auth_files(home.path(), None).is_empty());
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(home.path().join(".codex/auth.json"), "{}").unwrap();
        let proxy = home.path().join(".cli-proxy-api");
        std::fs::create_dir_all(&proxy).unwrap();
        for name in [
            "codex-b@example.test.json",
            "codex-a@example.test.json",
            "claude-a.json",
            "codex-notes.txt",
        ] {
            std::fs::write(proxy.join(name), "{}").unwrap();
        }
        let names: Vec<String> = local_auth_files(home.path(), None)
            .iter()
            .map(|path| {
                path.strip_prefix(home.path())
                    .unwrap()
                    .display()
                    .to_string()
            })
            .map(|path| path.replace('\\', "/"))
            .collect();
        assert_eq!(
            names,
            vec![
                ".codex/auth.json",
                ".cli-proxy-api/codex-a@example.test.json",
                ".cli-proxy-api/codex-b@example.test.json",
            ]
        );
        let custom = tempfile::tempdir().unwrap();
        std::fs::write(custom.path().join("auth.json"), "{}").unwrap();
        assert_eq!(
            local_auth_files(home.path(), Some(custom.path().into()))[0],
            custom.path().join("auth.json")
        );
    }

    #[test]
    fn local_import_keeps_the_newest_copy_of_each_account() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, raw: String| {
            let path = dir.path().join(name);
            std::fs::write(&path, raw).unwrap();
            path
        };
        let files = [
            write("a-old.json", codex_cli_file("acct-a", 1_000)),
            write("b.json", codex_cli_file("acct-b", 5_000)),
            write("a-new.json", codex_cli_file("acct-a", 2_000)),
            write("api-key.json", r#"{"OPENAI_API_KEY":"sk-fixture"}"#.into()),
        ];
        let found = read_local_accounts(&files).unwrap();
        assert_eq!(
            found
                .iter()
                .map(|creds| (creds.account_id.as_str(), creds.refresh_token.as_str()))
                .collect::<Vec<_>>(),
            vec![("acct-a", "refresh-2000"), ("acct-b", "refresh-5000")]
        );
        assert_eq!(
            account_view(&found[0], "acct-a"),
            CodexAccount {
                account_id: "acct-a".into(),
                email: "cli@example.test".into(),
                plan_type: "plus".into(),
                active: true,
            }
        );
        assert!(!account_view(&found[1], "acct-a").active);
    }

    #[test]
    fn local_import_reports_why_nothing_was_imported_without_echoing_secrets() {
        assert!(read_local_accounts(&[])
            .unwrap_err()
            .contains("codex login"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, r#"{"OPENAI_API_KEY":"sk-fixture"}"#).unwrap();
        let error = read_local_accounts(&[path]).unwrap_err();
        assert!(error.contains("API-key"));
        assert!(!error.contains("sk-fixture"));
    }

    #[test]
    fn usage_view_carries_both_windows() {
        let usage = codex_auth::parse_usage(
            r#"{"plan_type":"pro","rate_limit":{"limit_reached":true,"primary_window":{"used_percent":100,"limit_window_seconds":18000,"reset_at":42}}}"#,
            0,
        )
        .unwrap();
        let view = usage_view("acct-a".into(), usage);
        assert_eq!(view.account_id, "acct-a");
        assert_eq!(view.plan_type, "pro");
        assert!(view.limit_reached);
        assert_eq!(
            view.primary,
            Some(CodexUsageWindow {
                used_percent: 100.0,
                window_seconds: 18000,
                reset_at: 42
            })
        );
        assert_eq!(view.secondary, None);
    }
}
