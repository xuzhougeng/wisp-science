//! Project-list queries shared by desktop and future native hosts.

use std::collections::HashSet;

use anyhow::Result;
use wisp_dto::ProjectSummary;
use wisp_store::Store;

/// List projects in the store's order (starred first, then latest activity).
///
/// `running` and `awaiting` are snapshots of session IDs, not project IDs.
/// Hosts must release their runtime locks before awaiting this query. The
/// snapshots need not include idle sessions: unseen replies come from the store.
/// Database-list and star-query errors propagate; optional activity and sync
/// enrichment retains the desktop's best-effort fallback behavior.
pub async fn list_projects(
    store: &Store,
    running: &HashSet<String>,
    awaiting: &HashSet<String>,
) -> Result<Vec<ProjectSummary>> {
    let rows = store.list_projects().await?;
    let starred = store.starred_project_ids().await?;
    let mut projects = Vec::with_capacity(rows.len());
    for (
        id,
        name,
        workspace_dir,
        _created_at,
        updated_at,
        session_count,
        description,
        artifact_count,
    ) in rows
    {
        let (running_count, needs_you_count) =
            project_status_counts(store, &id, running, awaiting).await;
        let sync_state = store.get_project_sync_state(&id).await.ok().flatten();
        let folder_sync = store
            .folder_snapshot_status(&id)
            .await
            .ok()
            .flatten()
            .map(str::to_owned);
        let sync_configured = sync_state
            .as_ref()
            .is_some_and(|state| state.base_revision.is_some());
        projects.push(ProjectSummary {
            starred: starred.contains(&id),
            id,
            name,
            description,
            workspace_dir,
            session_count,
            artifact_count,
            updated_at,
            running_count,
            needs_you_count,
            sync_configured,
            last_synced_at: sync_state.and_then(|state| state.last_synced_at),
            folder_sync,
        });
    }
    Ok(projects)
}

/// Count running sessions and sessions needing attention in one project.
///
/// Pending approvals take precedence over running turns. Otherwise, an unseen
/// assistant/internal reply needs attention until viewed. Like the desktop's
/// existing query, unavailable session metadata falls back to zero counts.
pub async fn project_status_counts(
    store: &Store,
    project_id: &str,
    running: &HashSet<String>,
    awaiting: &HashSet<String>,
) -> (i64, i64) {
    let Ok(rows) = store.list_session_last_roles(project_id).await else {
        return (0, 0);
    };
    let mut running_count = 0;
    let mut needs_you_count = 0;
    for (id, role, unseen) in rows {
        if awaiting.contains(&id) {
            needs_you_count += 1;
        } else if running.contains(&id) {
            running_count += 1;
        } else if matches!(role.as_deref(), Some("assistant" | "internal")) && unseen {
            needs_you_count += 1;
        }
    }
    (running_count, needs_you_count)
}

/// The home page uses the same five recent sessions as the WebView. A project
/// scope returns its saved sidebar sessions, including named drafts.
/// This read-only snapshot deliberately does not claim live runtime activity.
pub async fn list_browser_sessions(
    store: &Store,
    project_id: Option<&str>,
) -> Result<Vec<wisp_dto::RecentSession>> {
    if let Some(project_id) = project_id {
        anyhow::ensure!(
            store
                .list_projects()
                .await?
                .iter()
                .any(|row| row.0 == project_id),
            "Project not found"
        );
        let roles = store.list_session_last_roles(project_id).await?;
        Ok(store
            .list_sessions(project_id)
            .await?
            .into_iter()
            .map(|(id, title, ts, folder_id, _)| {
                let needs_you = roles.iter().any(|(sid, role, unseen)| {
                    sid == &id
                        && *unseen
                        && matches!(role.as_deref(), Some("assistant" | "internal"))
                });
                wisp_dto::RecentSession {
                    id,
                    project_id: project_id.to_owned(),
                    title,
                    ts,
                    status: if needs_you { "needs_you" } else { "complete" }.into(),
                    folder_id,
                }
            })
            .collect())
    } else {
        Ok(store
            .list_recent_sessions_detail(5)
            .await?
            .into_iter()
            .map(|row| {
                let needs_you = row.unseen
                    && matches!(row.last_role.as_deref(), Some("assistant" | "internal"));
                wisp_dto::RecentSession {
                    id: row.id,
                    project_id: row.project_id,
                    title: row.title,
                    ts: row.created_at,
                    status: if needs_you { "needs_you" } else { "complete" }.into(),
                    folder_id: None,
                }
            })
            .collect())
    }
}

/// Verify project ownership before loading a bounded page. Reading never marks
/// the conversation seen or changes the WebView's active session.
pub async fn browser_transcript(
    store: &Store,
    project_id: &str,
    session_id: &str,
    before_seq: Option<i64>,
) -> Result<(Vec<wisp_dto::project_browser::BrowserMessage>, Option<i64>)> {
    anyhow::ensure!(
        list_browser_sessions(store, Some(project_id))
            .await?
            .iter()
            .any(|s| s.id == session_id),
        "Session not found in project"
    );
    let page = store
        .load_session_transcript_page(session_id, before_seq, 20)
        .await?;
    let messages = page
        .messages
        .into_iter()
        .filter_map(|(seq, message)| {
            let role = match message.role {
                wisp_llm::Role::System => return None,
                wisp_llm::Role::User => "user",
                wisp_llm::Role::Assistant => "assistant",
                wisp_llm::Role::Tool => "tool",
            };
            let mut text = message.content.as_text();
            for call in message.tool_calls {
                text.push_str(&format!(
                    "\n{}\n{}",
                    call.function.name, call.function.arguments
                ));
            }
            Some(wisp_dto::project_browser::BrowserMessage {
                seq,
                role: role.into(),
                text,
                tool_name: message.tool_name,
            })
        })
        .collect();
    Ok((messages, page.next_before_seq))
}

/// Explicit desired state makes retries safe after a lost response. Activity
/// timestamps, workspace files, and active session identities are untouched.
pub async fn set_project_starred(store: &Store, id: &str, starred: bool) -> Result<()> {
    store.set_project_starred(id, starred).await
}
