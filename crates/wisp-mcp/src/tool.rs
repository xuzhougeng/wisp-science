//! `McpTool` — wraps a remote MCP tool as a `wisp_tools::Tool`.

use crate::client::{McpCallResult, McpClient, RemoteTool};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use wisp_llm::ToolSchema;
use wisp_tools::{Approval, Tool, ToolEnv, ToolEvent, ToolResult};

const MAX_PRESENTATION_HTML_BYTES: usize = 32 * 1024 * 1024;

pub struct McpTool {
    name: String,
    schema: ToolSchema,
    remote: RemoteTool,
    client: Arc<McpClient>,
    require_approval: bool,
}

impl McpTool {
    pub fn new(tool: RemoteTool, client: Arc<McpClient>) -> Self {
        let schema = ToolSchema::new(&tool.name, &tool.description, tool.input_schema.clone());
        Self {
            name: tool.name.clone(),
            schema,
            remote: tool,
            client: Arc::clone(&client),
            require_approval: false,
        }
    }

    pub fn new_requiring_approval(tool: RemoteTool, client: Arc<McpClient>) -> Self {
        let mut wrapped = Self::new(tool, client);
        wrapped.require_approval = true;
        wrapped
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
        env.emit(ToolEvent::Presentation {
            kind: "mcp_app".into(),
            payload: json!({
                "tool": self.remote,
                "arguments": args,
                "result": result,
                "resource": resource,
            }),
        })
        .await;
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
    fn preview(&self, args: &Value) -> String {
        let s = args.to_string();
        s.chars().take(120).collect()
    }
    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        match self.client.tool_call_rich(&self.name, args).await {
            Ok(result) => {
                let mut content = result.text_content();
                let artifacts = materialize_html_resources(&result, env.project_root(), env).await;
                if !artifacts.is_empty() {
                    content.push_str("\n\nGenerated artifacts: ");
                    content.push_str(
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
                if content.trim().is_empty() {
                    content = "(no output)".into();
                }
                if result.is_error {
                    ToolResult::fail(content)
                } else {
                    ToolResult::ok(content)
                }
            }
            Err(e) => ToolResult::fail(format!("mcp {name} error: {e}", name = self.name)),
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
}
