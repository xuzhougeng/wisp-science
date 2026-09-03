use wisp_runs::{
    CancelRunTool, CleanupRunWorkspaceTool, ConfigureSshTrustTool, GetRunTool, HarvestRunTool,
    ListRemoteFilesTool, MonitorRunTool, RemoveRemoteFilesTool, RunInContextTool, RunManager,
    TransferBetweenContextsTool,
};
use wisp_store::Store;
use wisp_tools::Registry;

pub const CLI_PROJECT_ID: &str = "cli";

pub fn register_run_tools(
    registry: &mut Registry,
    store: Store,
    manager: RunManager,
    project_id: &str,
) {
    let scope = wisp_store::StateScope::mainline(project_id.to_string());
    registry.add(Box::new(RunInContextTool::new(
        store.clone(),
        manager.clone(),
        project_id.to_string(),
        None,
    )));
    registry.add(Box::new(ConfigureSshTrustTool::new(
        store.clone(),
        manager.clone(),
        None,
    )));
    registry.add(Box::new(TransferBetweenContextsTool::new(
        store.clone(),
        manager.clone(),
        project_id.to_string(),
        None,
    )));
    registry.add(Box::new(GetRunTool::new_in_scope(
        store.clone(),
        scope.clone(),
    )));
    registry.add(Box::new(MonitorRunTool::new_in_scope(
        store.clone(),
        scope.clone(),
    )));
    registry.add(Box::new(CancelRunTool::new_in_scope(
        store.clone(),
        manager.clone(),
        scope.clone(),
    )));
    registry.add(Box::new(HarvestRunTool::new_in_scope(
        store.clone(),
        manager.clone(),
        scope.clone(),
    )));
    registry.add(Box::new(CleanupRunWorkspaceTool::new_in_scope(
        store.clone(),
        manager.clone(),
        scope,
    )));
    registry.add(Box::new(ListRemoteFilesTool::new(
        store.clone(),
        project_id.to_string(),
        None,
    )));
    registry.add(Box::new(RemoveRemoteFilesTool::new(
        store,
        manager,
        project_id.to_string(),
        None,
    )));
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_runs::open_project_store;

    #[tokio::test]
    async fn register_run_tools_exposes_monitor_run() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp-cli-run-tools-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = open_project_store(&tmp, "eval", "t").await.unwrap();
        let mut registry = wisp_tools::Registry::builtins();
        register_run_tools(&mut registry, store, RunManager::new(), "eval");
        let names = registry.names();
        for expected in [
            "run_in_context",
            "monitor_run",
            "get_run",
            "cancel_run",
            "harvest_run",
            "cleanup_run_workspace",
            "transfer_between_contexts",
            "configure_ssh_trust",
            "list_remote_files",
            "remove_remote_files",
        ] {
            assert!(
                names.iter().any(|name| *name == expected),
                "missing {expected} in {names:?}"
            );
        }
        let _ = std::fs::remove_dir_all(tmp);
    }
}
