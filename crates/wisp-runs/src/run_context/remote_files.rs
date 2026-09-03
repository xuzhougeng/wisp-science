//! Remote staging ledger operations: list what this project placed on a
//! server, classify what is still referenced, and delete retracted/replaced
//! files. Only ledgered paths can ever be deleted — never arbitrary input.
//! Harvest-persisted outputs (too large to pull back) live here too, so a
//! discarded server can be audited and abandoned without leaving silent
//! `ssh://` references.

use super::{checked_output, ssh_script_command, RunCommandRunner, REMOTE_RPC_TIMEOUT};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteFileState {
    /// Still referenced: its run is active, or a staged input whose workdir
    /// has not been cleaned yet.
    Active,
    /// A newer upload ledgered the same remote path.
    Replaced,
    /// No live reference; safe to delete.
    Orphan,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteFileView {
    pub id: String,
    pub remote_path: String,
    pub source: String,
    pub run_id: Option<String>,
    pub run_status: Option<String>,
    pub size_bytes: Option<i64>,
    pub created_at: i64,
    pub state: RemoteFileState,
}

/// Build the `ssh://alias/path` URI used for External Artifact versions.
/// Absolute paths become `ssh://alias/home/...`; `~/...` stays `ssh://alias/~/...`.
pub fn ssh_uri_for_remote_path(alias: &str, remote_path: &str) -> String {
    if remote_path.starts_with('/') {
        format!("ssh://{alias}{remote_path}")
    } else {
        format!("ssh://{alias}/{remote_path}")
    }
}

pub fn ssh_uri_for_context_path(context_id: &str, remote_path: &str) -> String {
    let alias = context_id.strip_prefix("ssh:").unwrap_or(context_id);
    ssh_uri_for_remote_path(alias, remote_path)
}

/// Stable prefix so the UI can localize. Re-adding the same host alias must
/// not resurrect a reference the user already abandoned.
pub const SOURCE_DISCARDED_ERROR: &str = "source_discarded: This artifact's source server was discarded; the remote file is no longer available through Wisp.";

pub async fn refuse_if_source_discarded(
    store: &wisp_store::Store,
    uri: &str,
) -> Result<(), String> {
    if store
        .ssh_uri_source_discarded(uri)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err(SOURCE_DISCARDED_ERROR.into());
    }
    Ok(())
}

pub async fn refuse_if_context_path_discarded(
    store: &wisp_store::Store,
    context_id: &str,
    remote_path: &str,
) -> Result<(), String> {
    refuse_if_source_discarded(store, &ssh_uri_for_context_path(context_id, remote_path)).await
}

/// Drop the project's claim on a server: mark External artifacts discarded and
/// close the staging ledger. Remote bytes are not deleted — the machine is
/// being thrown away.
pub async fn abandon_context_sources(
    store: &wisp_store::Store,
    alias: &str,
) -> Result<u64, String> {
    let marked = store
        .mark_external_artifacts_source_discarded(&format!("ssh://{alias}/"))
        .await
        .map_err(|e| e.to_string())?;
    store
        .mark_remote_staging_removed_for_context(&format!("ssh:{alias}"))
        .await
        .map_err(|e| e.to_string())?;
    Ok(marked)
}

pub async fn list_remote_files(
    store: &wisp_store::Store,
    project_id: &str,
    context_id: &str,
) -> Result<Vec<RemoteFileView>, String> {
    let entries = store
        .list_remote_staging(project_id, context_id, false)
        .await
        .map_err(|e| e.to_string())?;
    let alias = context_id.strip_prefix("ssh:").unwrap_or(context_id);
    let live_uris: HashSet<String> = store
        .list_live_external_uris_on_context(project_id, &format!("ssh://{alias}/"))
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    // Latest ledger entry per remote path wins; older ones are "replaced".
    let mut latest: HashMap<&str, (i64, &str)> = HashMap::new();
    for entry in &entries {
        let candidate = (entry.created_at, entry.id.as_str());
        latest
            .entry(entry.remote_path.as_str())
            .and_modify(|current| {
                if candidate > *current {
                    *current = candidate;
                }
            })
            .or_insert(candidate);
    }
    let mut views = Vec::with_capacity(entries.len());
    for entry in &entries {
        let run = match entry.run_id.as_deref() {
            Some(run_id) => store.get_run(run_id).await.map_err(|e| e.to_string())?,
            None => None,
        };
        let replaced = latest
            .get(entry.remote_path.as_str())
            .is_some_and(|(_, id)| *id != entry.id);
        let persist_live = entry.source == "harvest_persist"
            && live_uris.contains(&ssh_uri_for_remote_path(alias, &entry.remote_path));
        // A current upload is the user's dataset on that server — it stays
        // Active after the transfer run succeeds. Only a failed/cancelled
        // attempt (partial bytes) or a harvest-persist with no live artifact
        // becomes an orphan. Replaced is decided first: the newer entry owns
        // the path, so the older ledger row must never trigger `rm`.
        let transfer_live = entry.source == "transfer"
            && run.as_ref().map_or(true, |run| {
                !run.status.is_terminal() || run.status == wisp_store::RunStatus::Succeeded
            });
        let input_live = entry.source == "run_input"
            && run
                .as_ref()
                .is_some_and(|run| !run.status.is_terminal() || run.cleaned_at.is_none());
        let active = persist_live || transfer_live || input_live;
        let state = if replaced {
            RemoteFileState::Replaced
        } else if active {
            RemoteFileState::Active
        } else {
            RemoteFileState::Orphan
        };
        views.push(RemoteFileView {
            id: entry.id.clone(),
            remote_path: entry.remote_path.clone(),
            source: entry.source.clone(),
            run_id: entry.run_id.clone(),
            run_status: run.map(|run| run.status.as_str().to_string()),
            size_bytes: entry.size_bytes,
            created_at: entry.created_at,
            state,
        });
    }
    Ok(views)
}

fn removal_payload(paths: &[(String, String)]) -> String {
    let mut payload = String::from("set -eu\n");
    for (id, path) in paths {
        payload.push_str(&super::remote_path_assignment(path));
        payload.push('\n');
        payload.push_str("rm -rf \"$path\"\n");
        payload.push_str(&format!("printf '__WISP_RM__:%s\\n' '{id}'\n"));
    }
    payload
}

/// Delete ledgered files from the server. Active entries require `force`
/// (explicit user confirmation). A path that no longer exists on the server
/// still counts as removed — ledger/reality drift resolves toward removal.
///
/// Replaced rows share a path with a newer entry that owns the bytes. They
/// are closed in-ledger only — `rm` would delete the current file.
pub async fn remove_remote_files(
    store: &wisp_store::Store,
    runner: &dyn RunCommandRunner,
    project_id: &str,
    context: &wisp_store::ExecutionContext,
    ids: &[String],
    force: bool,
) -> Result<u64, String> {
    if ids.is_empty() {
        return Err("remove_remote_files requires at least one ledger entry id".into());
    }
    let views = list_remote_files(store, project_id, &context.id).await?;
    let mut ledger_only = Vec::new();
    let mut targets = Vec::new();
    let mut seen_paths = HashSet::new();
    for id in ids {
        let Some(view) = views.iter().find(|view| &view.id == id) else {
            return Err(format!(
                "remote file entry {id} is not ledgered for this project and server"
            ));
        };
        if view.state == RemoteFileState::Active && !force {
            return Err(format!(
                "{} is still referenced by run {}; pass force only with explicit user \
                 confirmation",
                view.remote_path,
                view.run_id.as_deref().unwrap_or("unknown")
            ));
        }
        if view.state == RemoteFileState::Replaced {
            ledger_only.push(view.id.clone());
            continue;
        }
        if seen_paths.insert(view.remote_path.clone()) {
            targets.push((view.id.clone(), view.remote_path.clone()));
        } else {
            ledger_only.push(view.id.clone());
        }
    }
    let mut confirmed = ledger_only;
    if !targets.is_empty() {
        let connection = crate::ssh_hosts::SshConnection::from_execution_context(context)?;
        let output = checked_output(
            "remove remote files",
            runner
                .run(
                    ssh_script_command(
                        &connection,
                        "remove remote files",
                        removal_payload(&targets),
                    )?,
                    REMOTE_RPC_TIMEOUT,
                )
                .await,
        )?;
        let deleted: Vec<String> = output
            .stdout
            .lines()
            .filter_map(|line| line.strip_prefix("__WISP_RM__:"))
            .map(|id| id.trim().to_string())
            .collect();
        if deleted.is_empty() {
            return Err("remote file removal did not confirm any deletion".into());
        }
        confirmed.extend(deleted);
    } else if confirmed.is_empty() {
        return Err("remote file removal did not confirm any deletion".into());
    }
    store
        .mark_remote_staging_removed(&confirmed)
        .await
        .map_err(|e| e.to_string())?;
    let discarded_uris: Vec<String> = views
        .iter()
        .filter(|view| {
            confirmed.iter().any(|id| id == &view.id) && view.source == "harvest_persist"
        })
        .map(|view| ssh_uri_for_context_path(&context.id, &view.remote_path))
        .collect();
    store
        .mark_external_uris_source_discarded(&discarded_uris)
        .await
        .map_err(|e| e.to_string())?;
    Ok(confirmed.len() as u64)
}

/// Disposal audit before dropping a server: what would be abandoned.
/// Counts are **across every project** — `abandon_context_sources` is
/// alias-global, so a report of only the active project would hide sole copies.
#[derive(Debug, Clone, Serialize)]
pub struct ContextDisposalReport {
    pub context_id: String,
    /// Registered artifact references (`ssh://alias/…`) that still live only
    /// on this server. These are the only Wisp copy of those bytes.
    pub external_references: i64,
    /// Ledgered files not yet removed from the server.
    pub staged_files: i64,
    /// Runs still submitted/running/cancelling on this context.
    pub active_runs: i64,
    /// Same as `external_references`: External refs are the project's only copy.
    pub sole_remote_copies: i64,
}

pub async fn context_disposal_report(
    store: &wisp_store::Store,
    _project_id: &str,
    context: &wisp_store::ExecutionContext,
) -> Result<ContextDisposalReport, String> {
    let alias = context.id.strip_prefix("ssh:").unwrap_or(&context.id);
    let external_references = store
        .count_external_references_on_context_all(&format!("ssh://{alias}/"))
        .await
        .map_err(|e| e.to_string())?;
    let staged_files = store
        .count_remote_staging_on_context(&context.id)
        .await
        .map_err(|e| e.to_string())?;
    let active_runs = store
        .count_active_runs_on_context(&context.id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ContextDisposalReport {
        context_id: context.id.clone(),
        external_references,
        staged_files,
        active_runs,
        sole_remote_copies: external_references,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_payload_deletes_only_quoted_ledgered_paths() {
        let payload = removal_payload(&[
            ("id-1".into(), "~/wisp/proj/data/input.fasta".into()),
            ("id-2".into(), "/scratch/proj/big file.bam".into()),
        ]);
        assert!(payload.contains("path=\"$HOME\"/'wisp/proj/data/input.fasta'"));
        assert!(payload.contains("path='/scratch/proj/big file.bam'"));
        assert_eq!(payload.matches("rm -rf \"$path\"").count(), 2);
        assert!(payload.contains("__WISP_RM__:%s\\n' 'id-1'"));
        assert!(payload.contains("__WISP_RM__:%s\\n' 'id-2'"));
    }

    #[test]
    fn ssh_uri_keeps_absolute_and_home_relative_paths() {
        assert_eq!(
            ssh_uri_for_remote_path("gpu", "/home/alice/data/x.bam"),
            "ssh://gpu/home/alice/data/x.bam"
        );
        assert_eq!(
            ssh_uri_for_remote_path("gpu", "~/wisp/proj/data/x.bam"),
            "ssh://gpu/~/wisp/proj/data/x.bam"
        );
        assert_eq!(
            ssh_uri_for_context_path("ssh:gpu", "/scratch/out.tsv"),
            "ssh://gpu/scratch/out.tsv"
        );
    }
}
