//! Shared Run control plane used by the desktop shell and the headless CLI.
//!
//! Hosts register the same tools (`run_in_context`, `monitor_run`, `get_run`,
//! `cancel_run`, harvest/cleanup/transfer) against a SQLite [`wisp_store::Store`]
//! and a [`RunManager`]. Waiting is model-free: do not poll with `shell` sleep.

pub mod exploration_isolation;
pub mod harvest;
pub mod mime;
pub mod snapshot_store;
pub mod ssh_guard;
pub mod ssh_hosts;
pub mod ssh_master;
pub mod storage_prefs;

mod run_context;

pub use run_context::remote_files;
pub use run_context::*;

/// Always-on prompt for hosts that expose the Run tools.
pub fn runs_guidance() -> String {
    "## Runs\n\n\
Long-running commands use **run_in_context** (context_id `local`, or an SSH/WSL context when one is selected). \
After submission, call **monitor_run** with the returned run_id. Wisp waits without extra model calls. \
Do not use shell `sleep`, `Start-Sleep`, `ps`/`kill -0` polling, `nohup`, background `&`, or repeated **get_run**. \
The `shell` tool has a 60s timeout and is the wrong tool for waiting on jobs. \
If `monitor_run` returns `wait_interrupted`, answer the user, then call it again with the same id. Do not resubmit.\n"
        .into()
}

/// Combine the always-on Runs section with any registered remote contexts.
pub async fn cli_compute_section(store: &wisp_store::Store) -> String {
    let mut out = runs_guidance();
    let default = ssh_hosts::stored_default_execution_context(store).await;
    let default_ctx = match default.as_deref() {
        Some(id) => store.get_execution_context(id).await.ok().flatten(),
        None => None,
    };
    if let Ok(contexts) = store.list_execution_contexts().await {
        if let Some(section) = ssh_hosts::render_contexts_section(&contexts, default_ctx.as_ref()) {
            out.push('\n');
            out.push_str(&section);
        }
    }
    out
}

/// Open (or create) the project-local control-plane database and ensure a
/// project row exists so Runs can be submitted against `local`.
pub async fn open_project_store(
    root: &std::path::Path,
    project_id: &str,
    project_name: &str,
) -> anyhow::Result<wisp_store::Store> {
    let path = match std::env::var("WISP_STORE") {
        Ok(value) if !value.trim().is_empty() => std::path::PathBuf::from(value),
        _ => root.join(".wisp").join("wisp.sqlite"),
    };
    let store = wisp_store::Store::open(&path).await?;
    store
        .create_project(project_id, project_name, &root.to_string_lossy())
        .await?;
    Ok(store)
}
