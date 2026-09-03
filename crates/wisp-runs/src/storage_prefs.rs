//! Effective storage locations for a (project × execution context): stored
//! preferences when the user confirmed them, deterministic defaults otherwise.

use wisp_store::ContextStoragePrefs;

pub const DEFAULT_REMOTE_WORKDIR_ROOT: &str = ".wisp-science/runs";

pub fn slug(value: &str) -> String {
    let sanitized: String = value
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let collapsed = sanitized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        "project".into()
    } else {
        collapsed
    }
}

pub fn default_prefs(
    project_id: &str,
    project_name: &str,
    context_id: &str,
    context_label: &str,
) -> ContextStoragePrefs {
    let now = chrono::Utc::now().timestamp();
    ContextStoragePrefs {
        project_id: project_id.into(),
        context_id: context_id.into(),
        remote_data_root: format!("~/wisp/{}/data", slug(project_name)),
        remote_workdir_root: DEFAULT_REMOTE_WORKDIR_ROOT.into(),
        local_results_dir: format!("remote/{}", slug(context_label)),
        created_at: now,
        updated_at: now,
    }
}

/// Stored preferences when present, deterministic defaults otherwise. The
/// second value reports whether the user has confirmed (persisted) them.
pub async fn effective_prefs(
    store: &wisp_store::Store,
    project_id: &str,
    context_id: &str,
) -> Result<(ContextStoragePrefs, bool), String> {
    if let Some(prefs) = store
        .get_context_storage_prefs(project_id, context_id)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok((prefs, true));
    }
    let project_name = store
        .get_project(project_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|(name, _)| name)
        .unwrap_or_default();
    let context_label = store
        .get_execution_context(context_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|context| context.label)
        .unwrap_or_else(|| context_id.to_string());
    Ok((
        default_prefs(project_id, &project_name, context_id, &context_label),
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_collapse_to_safe_lowercase_components() {
        assert_eq!(slug("My T-cell Atlas!"), "my-t-cell-atlas");
        assert_eq!(slug("  "), "project");
        assert_eq!(slug("GPU box #2"), "gpu-box-2");
    }

    #[test]
    fn defaults_follow_project_and_context_names() {
        let prefs = default_prefs("p", "T-cell Atlas", "ssh:gpu", "GPU Box");
        assert_eq!(prefs.remote_data_root, "~/wisp/t-cell-atlas/data");
        assert_eq!(prefs.remote_workdir_root, DEFAULT_REMOTE_WORKDIR_ROOT);
        assert_eq!(prefs.local_results_dir, "remote/gpu-box");
        assert!(prefs.validate().is_ok());
    }
}
