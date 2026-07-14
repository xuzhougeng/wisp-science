//! Agent tool for saving an interpreter on an existing ExecutionContext.

use serde::Deserialize;
use wisp_llm::ToolSchema;
use wisp_runtime::{RuntimeKey, RuntimeLanguage, RuntimeManager};
use wisp_store::Store;
use wisp_tools::{Tool, ToolEnv, ToolResult};

pub struct SetRuntimeInterpreterTool {
    store: Store,
    runtime_manager: RuntimeManager,
    project_id: String,
}

#[derive(Deserialize)]
struct SetRuntimeInterpreterArgs {
    context_id: String,
    language: RuntimeLanguage,
    executable: String,
}

impl SetRuntimeInterpreterTool {
    pub fn new(
        store: Store,
        runtime_manager: RuntimeManager,
        project_id: impl Into<String>,
    ) -> Self {
        Self {
            store,
            runtime_manager,
            project_id: project_id.into(),
        }
    }
}

fn language_names(language: RuntimeLanguage) -> (&'static str, &'static str) {
    match language {
        RuntimeLanguage::Python => ("python", "Python"),
        RuntimeLanguage::R => ("r", "R"),
    }
}

#[async_trait::async_trait]
impl Tool for SetRuntimeInterpreterTool {
    fn name(&self) -> &str {
        "set_runtime_interpreter"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "set_runtime_interpreter",
            "Save the Python or R executable for an existing execution context. This writes Wisp's persisted context settings; it does not set host environment variables or install software. If this project's matching REPL already exists, it is restarted and its in-memory variables are cleared.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "context_id": {
                        "type": "string",
                        "description": "Registered execution context id, for example local, ssh:CPU2, or wsl:Ubuntu"
                    },
                    "language": {
                        "type": "string",
                        "enum": ["python", "r"],
                        "description": "Runtime whose interpreter should be changed"
                    },
                    "executable": {
                        "type": "string",
                        "description": "Executable path or command as seen inside that context, for example C:\\Program Files\\R\\R-4.5.2\\bin\\Rscript.exe or /opt/conda/envs/research/bin/python"
                    }
                },
                "required": ["context_id", "language", "executable"]
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        let context = args
            .get("context_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let language = args
            .get("language")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let executable = args
            .get("executable")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        format!("[runtime @ {context}] {language} = {executable}")
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let args: SetRuntimeInterpreterArgs = match serde_json::from_value(args.clone()) {
            Ok(args) => args,
            Err(error) => {
                return ToolResult::fail(format!("set_runtime_interpreter args error: {error}"))
            }
        };
        let context_id = args.context_id.trim();
        let executable = args.executable.trim();
        if context_id.is_empty() {
            return ToolResult::fail("set_runtime_interpreter error: 'context_id' is required");
        }
        if executable.is_empty() {
            return ToolResult::fail("set_runtime_interpreter error: 'executable' is required");
        }

        if let Err(error) = crate::runtime_launcher::save_runtime_interpreter(
            &self.store,
            context_id,
            args.language,
            executable,
        )
        .await
        {
            return ToolResult::fail(format!("set_runtime_interpreter error: {error}"));
        }

        let (language, label) = language_names(args.language);
        let key = RuntimeKey {
            project_id: self.project_id.clone(),
            context_id: context_id.to_string(),
            language: args.language,
        };
        let had_session = self
            .runtime_manager
            .list()
            .iter()
            .any(|runtime| runtime.key == key);
        if !had_session {
            return ToolResult::ok(format!(
                "Saved the {label} interpreter for context '{context_id}' as '{executable}'. The next {language} call for this context will use it."
            ));
        }

        match self
            .runtime_manager
            .restart(key, env.project_root().to_path_buf())
            .await
        {
            Ok(_) => ToolResult::ok(format!(
                "Saved the {label} interpreter for context '{context_id}' as '{executable}' and restarted this project's {label} runtime. Its previous in-memory variables were cleared."
            )),
            Err(error) => ToolResult::fail(format!(
                "Saved the {label} interpreter for context '{context_id}' as '{executable}', but the runtime failed to restart: {error}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct NoEnv(PathBuf);

    #[async_trait::async_trait]
    impl ToolEnv for NoEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn emit(&self, _event: wisp_tools::ToolEvent) {}
    }

    fn manager() -> RuntimeManager {
        RuntimeManager::local(
            PathBuf::from("unused-app-data"),
            PathBuf::from("unused-worker.py"),
            None,
            vec![],
        )
    }

    #[tokio::test]
    async fn sets_one_context_interpreter_without_changing_the_other_fields() {
        let db = std::env::temp_dir().join(format!(
            "wisp_runtime_config_tool_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&db).await.unwrap();
        let mut context = wisp_store::ExecutionContext::new("ssh:CPU2", "CPU2").unwrap();
        context.config_json = serde_json::json!({
            "alias": "CPU2",
            "python_executable": "/opt/python/bin/python"
        })
        .to_string();
        store.upsert_execution_context(&context).await.unwrap();
        let tool = SetRuntimeInterpreterTool::new(store.clone(), manager(), "project-1");

        let result = tool
            .run(
                &serde_json::json!({
                    "context_id": "ssh:CPU2",
                    "language": "r",
                    "executable": " /opt/R/4.5/bin/Rscript "
                }),
                &NoEnv(std::env::temp_dir()),
            )
            .await;

        assert!(result.success, "{}", result.content);
        assert!(result.content.contains("next r call"));
        let saved = store
            .get_execution_context("ssh:CPU2")
            .await
            .unwrap()
            .unwrap();
        let config: serde_json::Value = serde_json::from_str(&saved.config_json).unwrap();
        assert_eq!(config["alias"], "CPU2");
        assert_eq!(config["python_executable"], "/opt/python/bin/python");
        assert_eq!(config["rscript_executable"], "/opt/R/4.5/bin/Rscript");
        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn rejects_unknown_languages_without_changing_the_context() {
        let db = std::env::temp_dir().join(format!(
            "wisp_runtime_config_tool_invalid_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&db).await.unwrap();
        let context = wisp_store::ExecutionContext::new("local", "Local").unwrap();
        let original = context.config_json.clone();
        store.upsert_execution_context(&context).await.unwrap();
        let tool = SetRuntimeInterpreterTool::new(store.clone(), manager(), "project-1");

        let result = tool
            .run(
                &serde_json::json!({
                    "context_id": "local",
                    "language": "julia",
                    "executable": "/opt/julia/bin/julia"
                }),
                &NoEnv(std::env::temp_dir()),
            )
            .await;

        assert!(!result.success);
        assert!(result.content.contains("unknown variant"));
        let saved = store.get_execution_context("local").await.unwrap().unwrap();
        assert_eq!(saved.config_json, original);
        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn schema_and_preview_make_the_scope_explicit() {
        let db = std::env::temp_dir().join(format!(
            "wisp_runtime_config_tool_schema_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&db).await.unwrap();
        let tool = SetRuntimeInterpreterTool::new(store, manager(), "project-1");
        assert_eq!(tool.name(), "set_runtime_interpreter");
        assert_eq!(
            tool.preview(&serde_json::json!({
                "context_id": "local",
                "language": "r",
                "executable": r"C:\Program Files\R\bin\Rscript.exe"
            })),
            r"[runtime @ local] r = C:\Program Files\R\bin\Rscript.exe"
        );
        let _ = std::fs::remove_file(db);
    }
}
