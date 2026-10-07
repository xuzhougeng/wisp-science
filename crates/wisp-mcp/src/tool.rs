//! `McpTool` — wraps a remote MCP tool as a `wisp_tools::Tool`.

use crate::client::{McpCallResult, McpClient, RemoteTool};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{SystemTime, UNIX_EPOCH};
use wisp_llm::ToolSchema;
use wisp_tools::{Approval, McpAppServer, Tool, ToolEnv, ToolEvent, ToolResult};

const MAX_PRESENTATION_HTML_BYTES: usize = 32 * 1024 * 1024;

pub struct McpTool {
    name: String,
    schema: ToolSchema,
    remote: RemoteTool,
    client: Arc<McpClient>,
    /// Snapshot of the whole server catalog (shared by every tool of one MCP
    /// connection) so an MCP App can later call sibling tools on the same
    /// server — including app-only helpers that never entered the registry.
    catalog: Arc<Vec<RemoteTool>>,
    connector_id: String,
    require_approval: bool,
}

impl McpTool {
    pub fn new(tool: RemoteTool, client: Arc<McpClient>) -> Self {
        Self::with_catalog(tool, client, "", Arc::new(Vec::new()))
    }

    pub fn new_requiring_approval(tool: RemoteTool, client: Arc<McpClient>) -> Self {
        let mut wrapped = Self::new(tool, client);
        wrapped.require_approval = true;
        wrapped
    }

    /// Registration path: the caller already fetched the full server catalog,
    /// so every tool of one connection shares the same snapshot. The
    /// `catalog` is what an MCP App bridge validates sibling calls against.
    pub fn with_catalog(
        tool: RemoteTool,
        client: Arc<McpClient>,
        connector_id: impl Into<String>,
        catalog: Arc<Vec<RemoteTool>>,
    ) -> Self {
        let schema = ToolSchema::new(&tool.name, &tool.description, tool.input_schema.clone());
        Self {
            name: tool.name.clone(),
            schema,
            remote: tool,
            client: Arc::clone(&client),
            catalog,
            connector_id: connector_id.into(),
            require_approval: false,
        }
    }

    pub fn with_catalog_requiring_approval(
        tool: RemoteTool,
        client: Arc<McpClient>,
        connector_id: impl Into<String>,
        catalog: Arc<Vec<RemoteTool>>,
    ) -> Self {
        let mut wrapped = Self::with_catalog(tool, client, connector_id, catalog);
        wrapped.require_approval = true;
        wrapped
    }

    /// Expose the tool under another registry name (e.g. when the server's
    /// name collides with a built-in). Calls still use the server's name.
    pub fn renamed(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self.schema = ToolSchema::new(
            &self.name,
            &self.remote.description,
            self.remote.input_schema.clone(),
        );
        self
    }

    async fn emit_mcp_app(
        &self,
        uri: &str,
        args: &Value,
        result: &McpCallResult,
        env: &dyn ToolEnv,
    ) {
        let Ok(resource_result) = self.client.resource_read(uri).await else {
            return;
        };
        let Some(resource) = resource_result
            .get("contents")
            .and_then(Value::as_array)
            .and_then(|contents| {
                contents.iter().find(|resource| {
                    resource.get("uri").and_then(Value::as_str) == Some(uri)
                        && resource
                            .get("mimeType")
                            .and_then(Value::as_str)
                            .is_some_and(|mime| mime.starts_with("text/html"))
                })
            })
        else {
            return;
        };
        let Some(html) = resource.get("text").and_then(Value::as_str) else {
            return;
        };
        if html.len() > MAX_PRESENTATION_HTML_BYTES {
            tracing::warn!("MCP App resource '{uri}' exceeds presentation size cap");
            return;
        }
        let server = McpAppServerHandle::new(
            self.connector_id.clone(),
            self.remote.display_title(),
            Arc::clone(&self.catalog),
            Arc::downgrade(&self.client),
            self.require_approval,
        );
        env.emit(ToolEvent::Presentation {
            kind: "mcp_app".into(),
            payload: json!({
                "tool": self.remote,
                "arguments": args,
                "result": result,
                "resource": resource,
            }),
            server: Some(Arc::new(server)),
        })
        .await;
    }
}

/// Host-side `serverTools` bridge handed to the desktop host when an MCP App
/// is presented. The host stores it keyed by the app instance so `tools/call`
/// from the iframe reuses the exact MCP connection that presented the app.
/// The Host owns the stable client. A view holds only a Weak reference plus
/// a physical-connection generation: rebuilding an Agent does not kill the
/// server, but reconnecting cannot silently authorize callbacks from an old App.
pub struct McpAppServerHandle {
    connector_id: String,
    app_name: String,
    catalog: Arc<Vec<RemoteTool>>,
    client: Weak<McpClient>,
    generation: u64,
    require_approval: bool,
}

impl McpAppServerHandle {
    pub fn new(
        connector_id: String,
        app_name: String,
        catalog: Arc<Vec<RemoteTool>>,
        client: Weak<McpClient>,
        require_approval: bool,
    ) -> Self {
        let generation = client.upgrade().map_or(0, |c| c.generation());
        Self {
            connector_id,
            app_name,
            catalog,
            client,
            generation,
            require_approval,
        }
    }

    fn tool(&self, name: &str) -> Option<&RemoteTool> {
        self.catalog.iter().find(|tool| tool.name == name)
    }
}

#[async_trait]
impl McpAppServer for McpAppServerHandle {
    fn is_connected(&self) -> bool {
        self.client
            .upgrade()
            .is_some_and(|client| client.is_connected() && client.generation() == self.generation)
    }
    fn connector_id(&self) -> &str {
        &self.connector_id
    }
    fn app_name(&self) -> &str {
        &self.app_name
    }
    fn require_approval(&self) -> bool {
        self.require_approval
    }
    fn tools(&self) -> Vec<Value> {
        self.catalog
            .iter()
            .filter_map(|tool| serde_json::to_value(tool).ok())
            .collect()
    }
    fn visible_to_app(&self, name: &str) -> bool {
        self.tool(name).is_some_and(RemoteTool::visible_to_app)
    }
    fn read_only(&self, name: &str) -> bool {
        self.tool(name).is_some_and(RemoteTool::read_only)
    }
    fn input_schema(&self, name: &str) -> Option<Value> {
        self.tool(name).map(|tool| tool.input_schema.clone())
    }
    async fn call_tool(&self, name: &str, arguments: &Value) -> Result<Value, String> {
        if !self.visible_to_app(name) {
            return Err(format!(
                "MCP App tool '{name}' is not visible to apps on this server"
            ));
        }
        if let Some(schema) = self.input_schema(name) {
            validate_tool_arguments(&schema, arguments)?;
        }
        let client = self
            .client
            .upgrade()
            .ok_or_else(|| "the MCP server connection for this App is closed")?;
        if !self.is_connected() {
            return Err("stale-instance: reopen the MCP App after reconnecting".into());
        }
        let result = client
            .tool_call_checked_generation(
                self.tool(name).ok_or("MCP tool no longer exists")?,
                arguments,
                Some(self.generation),
            )
            .await
            .map_err(|error| error.to_string())?;
        serde_json::to_value(&result).map_err(|error| format!("serialize MCP tool result: {error}"))
    }
}

/// Lightweight MCP `inputSchema` check: object type, `required`, property
/// types, `additionalProperties: false`, `items`, `enum`, numeric bounds
/// (`minimum` / `maximum` / `exclusiveMinimum` / `exclusiveMaximum`), and
/// `pattern`. Combinators (`oneOf` / `anyOf` / `allOf` / `not`) are not
/// evaluated. Unknown keywords are ignored so a host can fail closed on the
/// common cases without a full JSON Schema crate.
pub fn validate_tool_arguments(schema: &Value, arguments: &Value) -> Result<(), String> {
    validate_against_schema(schema, arguments, "arguments")
}

fn validate_against_schema(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    if !schema.is_object() {
        return Ok(());
    }
    if let Some(type_constraint) = schema.get("type") {
        if !value_matches_type(value, type_constraint) {
            return Err(format!("{path} does not match the tool input schema"));
        }
    }
    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array) {
        if !enum_values.iter().any(|allowed| allowed == value) {
            return Err(format!("{path} is not one of the allowed values"));
        }
    }
    validate_numeric_bounds(schema, value, path)?;
    validate_pattern(schema, value, path)?;
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for name in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(name) {
                    return Err(format!("{path} is missing required property '{name}'"));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            if let Some(properties) = properties {
                if let Some(unknown) = object.keys().find(|key| !properties.contains_key(*key)) {
                    return Err(format!("{path} has unexpected property '{unknown}'"));
                }
            } else if !object.is_empty() {
                return Err(format!("{path} does not allow additional properties"));
            }
        }
        if let Some(properties) = properties {
            for (name, child) in object {
                if let Some(child_schema) = properties.get(name) {
                    validate_against_schema(child_schema, child, &format!("{path}.{name}"))?;
                }
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (index, child) in array.iter().enumerate() {
            validate_against_schema(items, child, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn json_number_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

fn validate_numeric_bounds(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let Some(number) = json_number_as_f64(value) else {
        return Ok(());
    };
    if let Some(min) = schema.get("minimum").and_then(json_number_as_f64) {
        let exclusive = schema.get("exclusiveMinimum") == Some(&Value::Bool(true));
        if exclusive && number <= min {
            return Err(format!("{path} must be greater than {min}"));
        }
        if !exclusive && number < min {
            return Err(format!("{path} must be at least {min}"));
        }
    }
    if let Some(excl_min) = schema.get("exclusiveMinimum").and_then(json_number_as_f64) {
        if number <= excl_min {
            return Err(format!("{path} must be greater than {excl_min}"));
        }
    }
    if let Some(max) = schema.get("maximum").and_then(json_number_as_f64) {
        let exclusive = schema.get("exclusiveMaximum") == Some(&Value::Bool(true));
        if exclusive && number >= max {
            return Err(format!("{path} must be less than {max}"));
        }
        if !exclusive && number > max {
            return Err(format!("{path} must be at most {max}"));
        }
    }
    if let Some(excl_max) = schema.get("exclusiveMaximum").and_then(json_number_as_f64) {
        if number >= excl_max {
            return Err(format!("{path} must be less than {excl_max}"));
        }
    }
    Ok(())
}

fn validate_pattern(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let Some(pattern) = schema.get("pattern").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(text) = value.as_str() else {
        return Ok(());
    };
    let regex = regex::RegexBuilder::new(pattern)
        .size_limit(1 << 20)
        .dfa_size_limit(1 << 20)
        .build()
        .map_err(|_| format!("{path} has an invalid pattern constraint"))?;
    if regex.is_match(text) {
        Ok(())
    } else {
        Err(format!("{path} does not match the required pattern"))
    }
}

fn value_matches_type(value: &Value, type_constraint: &Value) -> bool {
    match type_constraint {
        Value::String(type_name) => value_is_json_type(value, type_name),
        Value::Array(types) => types.iter().any(|item| {
            item.as_str()
                .is_some_and(|type_name| value_is_json_type(value, type_name))
        }),
        _ => true,
    }
}

fn value_is_json_type(value: &Value, type_name: &str) -> bool {
    match type_name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn safe_html_name(result: &McpCallResult, uri: &str) -> String {
    let candidate = result
        .structured_content
        .as_ref()
        .and_then(|value| value.get("filename"))
        .and_then(Value::as_str)
        .or_else(|| uri.rsplit('/').next())
        .unwrap_or("mcp-artifact.html");
    let leaf = candidate.rsplit(['/', '\\']).next().unwrap_or(candidate);
    let mut clean: String = leaf
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        .take(96)
        .collect();
    clean = clean.trim_start_matches('.').replace("..", ".");
    if clean.is_empty() {
        clean = "mcp-artifact.html".into();
    }
    if !clean.to_ascii_lowercase().ends_with(".html") {
        clean.push_str(".html");
    }
    clean
}

async fn materialize_html_resources(
    result: &McpCallResult,
    project_root: &Path,
    env: &dyn ToolEnv,
) -> Vec<PathBuf> {
    let mut written = Vec::new();
    for block in &result.content {
        let Some(resource) = block
            .get("resource")
            .filter(|_| block.get("type").and_then(Value::as_str) == Some("resource"))
        else {
            continue;
        };
        let Some(html) = resource.get("text").and_then(Value::as_str) else {
            continue;
        };
        if !resource
            .get("mimeType")
            .and_then(Value::as_str)
            .is_some_and(|mime| mime.starts_with("text/html"))
            || html.len() > MAX_PRESENTATION_HTML_BYTES
        {
            continue;
        }
        let uri = resource
            .get("uri")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let filename = safe_html_name(result, uri);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let (stem, extension) = filename.rsplit_once('.').unwrap_or((&filename, "html"));
        let relative = PathBuf::from(".wisp")
            .join("plugin-artifacts")
            .join(format!("{stem}-{stamp}.{extension}"));
        let path = project_root.join(&relative);
        let Some(parent) = path.parent() else {
            continue;
        };
        if tokio::fs::create_dir_all(parent).await.is_err()
            || tokio::fs::write(&path, html.as_bytes()).await.is_err()
        {
            continue;
        }
        env.emit(ToolEvent::FileChanged {
            path: relative.to_string_lossy().to_string(),
        })
        .await;
        written.push(relative);
    }
    written
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn schema(&self) -> ToolSchema {
        self.schema.clone()
    }
    fn defer_schema(&self) -> bool {
        true
    }
    fn minimum_approval(&self) -> Approval {
        if self.require_approval {
            Approval::Ask
        } else {
            Approval::Allow
        }
    }
    /// The server's own `readOnlyHint`, which the bundled bio/literature
    /// retrieval servers set on every query tool. Anything that omits it — or
    /// says `false` — stays blocked in plan mode.
    fn read_only(&self) -> bool {
        self.remote.read_only()
    }
    fn preview(&self, args: &Value) -> String {
        let s = args.to_string();
        s.chars().take(120).collect()
    }
    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let call = async {
            match self.client.tool_call_checked(&self.remote, args).await {
                Ok(result) => {
                    let mut output = crate::result::model_result(&result);
                    let artifacts =
                        materialize_html_resources(&result, env.project_root(), env).await;
                    if !artifacts.is_empty() {
                        output.content.push_str("\n\nGenerated artifacts: ");
                        output.content.push_str(
                            &artifacts
                                .iter()
                                .map(|path| path.to_string_lossy())
                                .collect::<Vec<_>>()
                                .join(", "),
                        );
                    }
                    if let Some(uri) = self.remote.ui_resource_uri() {
                        self.emit_mcp_app(uri, args, &result, env).await;
                    }
                    output
                }
                Err(e) => ToolResult::fail(format!("mcp {name} error: {e}", name = self.name)),
            }
        };
        tokio::select! {
            result = call => result,
            _ = async { loop {
                if env.is_cancelled() { break; }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            } } => ToolResult::fail("MCP wait cancelled by user; server kept alive. External operation outcome may be unknown; do not replay automatically."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct TestEnv {
        root: PathBuf,
        changed: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ToolEnv for TestEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn emit(&self, event: ToolEvent) {
            if let ToolEvent::FileChanged { path } = event {
                self.changed.lock().unwrap().push(path);
            }
        }
    }

    #[tokio::test]
    async fn embedded_html_becomes_a_bounded_project_artifact() {
        let root = std::env::temp_dir().join(format!(
            "wisp-mcp-artifact-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let env = TestEnv {
            root: root.clone(),
            changed: Mutex::new(Vec::new()),
        };
        let result = McpCallResult {
            content: vec![json!({
                "type": "resource",
                "resource": {
                    "uri": "motif://artifact/demo.html",
                    "mimeType": "text/html",
                    "text": "<!doctype html><title>Motif</title>"
                }
            })],
            structured_content: Some(json!({ "filename": "../demo.html" })),
            meta: None,
            is_error: false,
        };
        let paths = materialize_html_resources(&result, &root, &env).await;
        assert_eq!(paths.len(), 1);
        assert!(paths[0].starts_with(".wisp/plugin-artifacts"));
        assert!(!paths[0].to_string_lossy().contains(".."));
        assert!(root.join(&paths[0]).is_file());
        assert_eq!(
            env.changed.lock().unwrap().as_slice(),
            &[paths[0].to_string_lossy().to_string()]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_app_server_handle_serves_catalog_and_reports_stale_clients() {
        let catalog = Arc::new(vec![
            RemoteTool {
                name: "figure_visualize".into(),
                title: None,
                description: String::new(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
                meta: None,
                annotations: None,
            },
            RemoteTool {
                name: "figure_preview_exact".into(),
                title: None,
                description: String::new(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
                meta: Some(json!({ "ui": { "visibility": ["app"] } })),
                annotations: Some(json!({ "readOnlyHint": true })),
            },
            RemoteTool {
                name: "figure_edit".into(),
                title: None,
                description: String::new(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
                meta: Some(json!({ "ui": { "visibility": ["model"] } })),
                annotations: None,
            },
        ]);
        let handle = McpAppServerHandle::new(
            "figure-library".into(),
            "Figure Library".into(),
            catalog,
            Weak::new(),
            true,
        );
        assert_eq!(handle.connector_id(), "figure-library");
        assert_eq!(handle.app_name(), "Figure Library");
        assert!(handle.require_approval());
        // Unset visibility and explicit app visibility both allow App calls;
        // model-only tools must be refused.
        assert!(handle.visible_to_app("figure_visualize"));
        assert!(handle.visible_to_app("figure_preview_exact"));
        assert!(!handle.visible_to_app("figure_edit"));
        assert!(!handle.visible_to_app("figure_nope"));
        // The server's readOnlyHint comes through for plan-mode gating.
        assert!(handle.read_only("figure_preview_exact"));
        assert!(!handle.read_only("figure_visualize"));
        let listed = handle.tools();
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0]["name"], "figure_visualize");
        assert_eq!(listed[1]["annotations"]["readOnlyHint"], true);
        // A dead Weak client is the stale-instance the host reports after the
        // owning agent drops its Arc (session end, rebuild, connector restart).
        let error = handle
            .call_tool("figure_preview_exact", &json!({}))
            .await
            .unwrap_err();
        assert!(error.contains("connection for this App is closed"));
        let error = handle
            .call_tool("figure_edit", &json!({}))
            .await
            .unwrap_err();
        assert!(error.contains("not visible to apps"));
    }

    #[tokio::test]
    async fn parallel_app_handles_do_not_share_catalogs() {
        let first = McpAppServerHandle::new(
            "server-a".into(),
            "App A".into(),
            Arc::new(vec![RemoteTool {
                name: "alpha_only".into(),
                title: None,
                description: String::new(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
                meta: Some(json!({ "ui": { "visibility": ["app"] } })),
                annotations: None,
            }]),
            Weak::new(),
            false,
        );
        let second = McpAppServerHandle::new(
            "server-b".into(),
            "App B".into(),
            Arc::new(vec![RemoteTool {
                name: "beta_only".into(),
                title: None,
                description: String::new(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
                meta: Some(json!({ "ui": { "visibility": ["app"] } })),
                annotations: None,
            }]),
            Weak::new(),
            false,
        );
        assert!(first.visible_to_app("alpha_only"));
        assert!(!first.visible_to_app("beta_only"));
        assert!(second.visible_to_app("beta_only"));
        assert!(!second.visible_to_app("alpha_only"));
        let error = first.call_tool("beta_only", &json!({})).await.unwrap_err();
        assert!(error.contains("not visible to apps"));
    }

    #[test]
    fn tool_argument_schema_rejects_missing_and_wrong_types() {
        let schema = json!({
            "type": "object",
            "properties": {
                "token": { "type": "string" },
                "count": { "type": "integer" }
            },
            "required": ["token"],
            "additionalProperties": false
        });
        validate_tool_arguments(&schema, &json!({ "token": "ok", "count": 2 })).unwrap();
        let missing = validate_tool_arguments(&schema, &json!({ "count": 1 })).unwrap_err();
        assert!(missing.contains("token"));
        let wrong = validate_tool_arguments(&schema, &json!({ "token": 1 })).unwrap_err();
        assert!(wrong.contains("token"));
        let extra =
            validate_tool_arguments(&schema, &json!({ "token": "ok", "nope": true })).unwrap_err();
        assert!(extra.contains("nope"));
    }

    #[test]
    fn tool_argument_schema_rejects_enum_bounds_and_pattern() {
        let schema = json!({
            "type": "object",
            "properties": {
                "kind": { "enum": ["alpha", "beta"] },
                "count": { "type": "integer", "minimum": 1, "maximum": 3 },
                "score": {
                    "type": "number",
                    "exclusiveMinimum": 0.0,
                    "exclusiveMaximum": 1.0
                },
                "draft_count": {
                    "type": "integer",
                    "minimum": 0,
                    "exclusiveMinimum": true,
                    "maximum": 10,
                    "exclusiveMaximum": true
                },
                "id": { "type": "string", "pattern": "^[a-z]+-[0-9]+$" }
            },
            "required": ["kind"]
        });
        validate_tool_arguments(
            &schema,
            &json!({
                "kind": "alpha",
                "count": 2,
                "score": 0.5,
                "draft_count": 1,
                "id": "seq-12"
            }),
        )
        .unwrap();
        let bad_enum = validate_tool_arguments(&schema, &json!({ "kind": "gamma" })).unwrap_err();
        assert!(bad_enum.contains("allowed values"));
        let low =
            validate_tool_arguments(&schema, &json!({ "kind": "alpha", "count": 0 })).unwrap_err();
        assert!(low.contains("at least"));
        let high =
            validate_tool_arguments(&schema, &json!({ "kind": "alpha", "count": 4 })).unwrap_err();
        assert!(high.contains("at most"));
        let excl_low = validate_tool_arguments(&schema, &json!({ "kind": "alpha", "score": 0.0 }))
            .unwrap_err();
        assert!(excl_low.contains("greater than"));
        let excl_high = validate_tool_arguments(&schema, &json!({ "kind": "alpha", "score": 1.0 }))
            .unwrap_err();
        assert!(excl_high.contains("less than"));
        let draft_low =
            validate_tool_arguments(&schema, &json!({ "kind": "alpha", "draft_count": 0 }))
                .unwrap_err();
        assert!(draft_low.contains("greater than"));
        let draft_high =
            validate_tool_arguments(&schema, &json!({ "kind": "alpha", "draft_count": 10 }))
                .unwrap_err();
        assert!(draft_high.contains("less than"));
        let pattern = validate_tool_arguments(&schema, &json!({ "kind": "alpha", "id": "SEQ-12" }))
            .unwrap_err();
        assert!(pattern.contains("pattern"));
        let enum_only = json!({ "enum": [1, 2] });
        validate_tool_arguments(&enum_only, &json!(2)).unwrap();
        assert!(validate_tool_arguments(&enum_only, &json!(3))
            .unwrap_err()
            .contains("allowed values"));
        let combinators = json!({
            "oneOf": [{ "type": "string" }, { "type": "number" }],
            "anyOf": [{ "const": "nope" }],
            "allOf": [{ "type": "null" }]
        });
        validate_tool_arguments(&combinators, &json!({ "ignored": true })).unwrap();
    }
}
