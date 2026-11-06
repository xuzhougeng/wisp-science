//! Offline smoke driver for the same registered DOCX tools the agent calls.
//! Usage: docx_revision SOURCE OUTPUT PARAGRAPH_ID OLD NEW
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use wisp_tools::{Registry, ToolEnv, ToolEvent};

struct Env(PathBuf);

#[async_trait]
impl ToolEnv for Env {
    fn project_root(&self) -> &Path {
        &self.0
    }
    async fn confirm(&self, _: &str) -> bool {
        false
    }
    async fn emit(&self, _: ToolEvent) {}
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err("Usage: docx_revision SOURCE OUTPUT PARAGRAPH_ID OLD NEW".into());
    }
    let output = Path::new(&args[1]);
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let env = Env(dunce::canonicalize(parent)?);
    let source = dunce::canonicalize(&args[0])?;
    let output = env
        .0
        .join(output.file_name().ok_or("Output has no filename")?);
    let tools = Registry::builtins();
    let read = tools.run("read_docx", &json!({"path":source}), &env).await;
    if !read.success {
        return Err(read.content.into());
    }
    let view: Value = serde_json::from_str(&read.content)?;
    let result = tools
        .run(
            "suggest_docx_edit",
            &json!({
                "path":source, "output_path":output, "source_sha256":view["source_sha256"],
                "paragraph_id":args[2], "old":args[3], "new":args[4]
            }),
            &env,
        )
        .await;
    if !result.success {
        return Err(result.content.into());
    }
    println!("{}", result.content);
    Ok(())
}
