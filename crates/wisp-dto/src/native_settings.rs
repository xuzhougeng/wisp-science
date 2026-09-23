//! Native settings transport. Command payloads retain the existing desktop DTOs.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA: &str = "wisp.native-settings.v1";

#[derive(Clone, Serialize, Deserialize)]
pub struct HostDescriptor {
    pub schema: String,
    pub endpoint: String,
    pub token: String,
    pub database: String,
    pub pid: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: String,
    pub id: String,
    pub project_id: Option<String>,
    pub command: String,
    pub args: Value,
}

#[derive(Serialize, Deserialize)]
pub struct Response {
    pub schema: String,
    pub id: String,
    pub result: Option<Value>,
    pub error: Option<String>,
}

pub const COMMANDS: &[&str] = &[
    "authorize_http_connection",
    "cancel_oauth_authorization",
    "get_pet",
    "native_terminal_snapshot",
    "write_terminal",
    "close_terminal",
    "authenticate_acp_agent",
    "native_download_update",
    "install_update",
    "plan_skill_portfolio",
    "add_custom_credential",
    "add_mcp_connection",
    "add_ssh_host",
    "browser_extension_status",
    "channels_status",
    "check_for_updates",
    "clear_memory",
    "confirm_feishu_pending_owner",
    "context_disposal_report",
    "create_global_memory",
    "credential_status",
    "delete_global_memory",
    "delete_mcp_connection",
    "delete_memory_file",
    "detect_local_environment",
    "feishu_bind_cancel",
    "feishu_bind_poll",
    "feishu_bind_start",
    "feishu_unbind",
    "get_appearance_prefs",
    "get_auto_failure_analysis_settings",
    "get_auto_review_enabled",
    "get_bootstrap_status",
    "get_browser_auto_close_tabs",
    "get_browser_auto_launch",
    "get_browser_url_filters",
    "get_context_storage_prefs",
    "get_default_execution_context",
    "get_device_bridge_token",
    "get_memory_view",
    "get_network_settings",
    "get_pet_runtime_status",
    "get_project_run_retention",
    "get_project_settings",
    "get_session_token_usage",
    "get_settings",
    "get_storage_usage",
    "get_token_usage",
    "get_update_check_enabled",
    "import_ssh_config_hosts",
    "import_wsl_contexts",
    "install_github_skill",
    "install_plugin",
    "install_plugin_url",
    "install_skill",
    "join_synced_project",
    "list_acp_agents",
    "list_approval_grants",
    "list_community_skills",
    "list_connectors",
    "list_custom_credentials",
    "list_execution_contexts",
    "list_mcp_connections",
    "list_models",
    "list_plugins",
    "list_projects",
    "list_quick_actions",
    "list_skill_files",
    "list_skills",
    "list_specialists",
    "list_ssh_hosts",
    "list_ssh_trust_edges",
    "list_workflow_templates",
    "model_catalog_lookup",
    "open_browser_extension_page",
    "preview_github_skills",
    "probe_execution_context",
    "project_sync_code",
    "read_memory_file",
    "read_skill_file",
    "reject_feishu_pending_owner",
    "reload_skills",
    "remove_acp_agent",
    "remove_custom_credential",
    "remove_model",
    "remove_plugin",
    "remove_quick_action",
    "remove_skill",
    "remove_specialist",
    "remove_ssh_host",
    "remove_workflow_template",
    "reorder_models",
    "resolve_project_sync",
    "revoke_all_approval_grants",
    "revoke_approval_grant",
    "revoke_device_bridge_token",
    "revoke_ssh_trust_edge",
    "rotate_device_bridge_token",
    "save_acp_agent",
    "save_local_environment_paths",
    "save_model",
    "save_quick_action",
    "save_specialist_cmd",
    "save_workflow_template",
    "set_active_model",
    "set_appearance_prefs",
    "set_approval_scope",
    "set_auto_failure_analysis_settings",
    "set_auto_review_enabled",
    "set_browser_auto_close_tabs",
    "set_browser_auto_launch",
    "set_browser_url_filters",
    "set_connector_enabled",
    "set_connector_skip_approvals",
    "set_context_storage_prefs",
    "set_credential",
    "set_default_execution_context",
    "set_device_bridge",
    "set_feishu_channel",
    "set_feishu_owner",
    "set_mcp_connection_enabled",
    "set_memory_enabled",
    "set_network_settings",
    "set_plugin_enabled",
    "set_project_run_retention",
    "set_settings",
    "set_skill_enabled",
    "set_skill_tags",
    "set_tool_approval",
    "set_update_check_enabled",
    "set_weixin_channel",
    "sync_project",
    "test_acp_agent",
    "test_mcp_connection",
    "test_oauth_mcp_connection",
    "test_ssh_connection",
    "update_browser_extension",
    "update_execution_context_interpreters",
    "update_global_memory",
    "update_mcp_connection",
    "update_project",
    "validate_settings",
    "weixin_bind_poll",
    "weixin_bind_start",
    "weixin_unbind",
    "write_memory_file",
];

#[derive(Clone, Serialize, Deserialize)]
pub struct TerminalSnapshot {
    pub text: String,
    pub running: bool,
    pub exit_code: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_swift_windows_fixtures_preserve_scope_and_void_success() {
        let request: Request = serde_json::from_str(include_str!(
            "../../../contracts/native-settings/v1/request.json"
        ))
        .unwrap();
        assert_eq!(request.schema, SCHEMA);
        assert_eq!(request.project_id.as_deref(), Some("research-1"));
        assert!(COMMANDS.contains(&request.command.as_str()));
        let prefs: crate::AppearancePrefs =
            serde_json::from_value(request.args["prefs"].clone()).unwrap();
        assert_eq!(prefs.ui_font_size, 15);
        let void: Response = serde_json::from_str(include_str!(
            "../../../contracts/native-settings/v1/void.json"
        ))
        .unwrap();
        assert!(void.error.is_none());
        assert!(serde_json::to_value(void)
            .unwrap()
            .get("result")
            .unwrap()
            .is_null());
    }
    #[test]
    fn command_catalog_matches_allowlist_without_agent_execution() {
        let catalog: Value = serde_json::from_str(include_str!(
            "../../../contracts/native-settings/v1/commands.json"
        ))
        .unwrap();
        let names = catalog["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["command"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        let allowed = COMMANDS
            .iter()
            .copied()
            .chain(["native_settings_capabilities"])
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names, allowed);
        assert_eq!(COMMANDS.len() + 1, allowed.len());
        assert!(!allowed.contains("send_message"));
        assert!(!allowed.contains("open_terminal"));
        assert!(allowed.contains("import_wsl_contexts"));
        assert!(allowed.contains("authorize_http_connection"));
    }
}
