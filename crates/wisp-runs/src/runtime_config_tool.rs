//! Agent tool for saving an interpreter on an existing ExecutionContext.
//!
//! Moved from the Tauri shell's `runtime_config_tool` so the embedded host
//! offers the same `set_runtime_interpreter`. `wisp-runs` does not depend on
//! `wisp-runtime`: each host hands its runtime manager over through
//! [`InterpreterRuntimes`].

use std::path::PathBuf;

use serde::Deserialize;
use wisp_llm::ToolSchema;
use wisp_store::Store;
use wisp_tools::{Tool, ToolEnv, ToolResult};

use crate::runtime_launch::InterpreterLanguage;

/// One conversation's interpreter, as a host's runtime manager keys it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterpreterKey {
    pub project_id: String,
    pub scope_key: String,
    pub session_id: String,
    pub context_id: String,
    pub language: InterpreterLanguage,
}

/// What the tool needs of the interpreters a host keeps.
#[async_trait::async_trait]
pub trait InterpreterRuntimes: Send + Sync {
    /// Whether the host has this interpreter, running or not.
    fn has_runtime(&self, key: &InterpreterKey) -> bool;
    /// Starts this interpreter afresh in `project_root`.
    async fn restart(&self, key: &InterpreterKey, project_root: PathBuf) -> anyhow::Result<()>;
}

pub struct SetRuntimeInterpreterTool {
    store: Store,
    runtime_manager: Box<dyn InterpreterRuntimes>,
    project_id: String,
    scope_key: String,
    session_id: String,
    /// A context whose saved path this host would not read, and why.
    refused: Option<(String, String)>,
}

#[derive(Deserialize)]
struct SetRuntimeInterpreterArgs {
    context_id: String,
    language: InterpreterLanguage,
    executable: String,
}

impl SetRuntimeInterpreterTool {
    pub fn new_in_session(
        store: Store,
        runtime_manager: Box<dyn InterpreterRuntimes>,
        project_id: impl Into<String>,
        scope_key: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            store,
            runtime_manager,
            project_id: project_id.into(),
            scope_key: scope_key.into(),
            session_id: session_id.into(),
            refused: None,
        }
    }

    /// Refuses `context_id`, saving nothing, for a host whose interpreters
    /// there would not start with a saved path: `reason` says so.
    pub fn refusing(mut self, context_id: impl Into<String>, reason: impl Into<String>) -> Self {
        self.refused = Some((context_id.into(), reason.into()));
        self
    }
}

fn language_names(language: InterpreterLanguage) -> (&'static str, &'static str) {
    match language {
        InterpreterLanguage::Python => ("python", "Python"),
        InterpreterLanguage::R => ("r", "R"),
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
            "Save the Python or R executable for an existing execution context. This writes Wisp's persisted context settings; it does not set host environment variables or install software. An interpreter inside a conda, mamba, or pixi environment prefix is launched with that prefix on the worker's own PATH, so it can load its own libraries. If this project's matching REPL already exists, it is restarted and its in-memory variables are cleared.",
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
        if self.scope_key != wisp_store::MAINLINE_SCOPE_KEY {
            return ToolResult::fail(
                "exploration_project_mutation_blocked: runtime interpreter settings cannot be changed inside an exploration",
            );
        }
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
        if let Some((_, reason)) = self.refused.as_ref().filter(|(id, _)| id == context_id) {
            return ToolResult::fail(format!("set_runtime_interpreter error: {reason}"));
        }

        if let Err(error) = crate::runtime_launch::save_runtime_interpreter(
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
        let key = InterpreterKey {
            project_id: self.project_id.clone(),
            scope_key: self.scope_key.clone(),
            session_id: self.session_id.clone(),
            context_id: context_id.to_string(),
            language: args.language,
        };
        let had_session = self.runtime_manager.has_runtime(&key);
        if !had_session {
            return ToolResult::ok(format!(
                "Saved the {label} interpreter for context '{context_id}' as '{executable}'. The next {language} call for this context will use it."
            ));
        }

        match self
            .runtime_manager
            .restart(&key, env.project_root().to_path_buf())
            .await
        {
            Ok(_) => ToolResult::ok(format!(
                "Saved the {label} interpreter for context '{context_id}' as '{executable}' and restarted this conversation's {label} runtime. Its previous in-memory variables were cleared; other conversations keep the old interpreter until their runtimes restart."
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
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

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

    /// Interpreters a host keeps, and the ones it was asked to restart;
    /// nothing is started.
    #[derive(Clone, Default)]
    struct Fake {
        running: Arc<Mutex<Vec<InterpreterKey>>>,
        restarted: Arc<Mutex<Vec<(InterpreterKey, PathBuf)>>>,
        fails: bool,
    }

    #[async_trait::async_trait]
    impl InterpreterRuntimes for Fake {
        fn has_runtime(&self, key: &InterpreterKey) -> bool {
            self.running.lock().unwrap().contains(key)
        }

        async fn restart(&self, key: &InterpreterKey, project_root: PathBuf) -> anyhow::Result<()> {
            self.restarted
                .lock()
                .unwrap()
                .push((key.clone(), project_root));
            if self.fails {
                anyhow::bail!("worker did not answer");
            }
            Ok(())
        }
    }

    fn manager() -> Box<dyn InterpreterRuntimes> {
        Box::new(Fake::default())
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
        let tool = SetRuntimeInterpreterTool::new_in_session(
            store.clone(),
            manager(),
            "project-1",
            wisp_store::MAINLINE_SCOPE_KEY,
            "frame-1",
        );

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
        let tool = SetRuntimeInterpreterTool::new_in_session(
            store.clone(),
            manager(),
            "project-1",
            wisp_store::MAINLINE_SCOPE_KEY,
            "frame-1",
        );

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
        let tool = SetRuntimeInterpreterTool::new_in_session(
            store,
            manager(),
            "project-1",
            wisp_store::MAINLINE_SCOPE_KEY,
            "frame-1",
        );
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

    #[tokio::test]
    async fn a_running_interpreter_is_restarted_and_a_refused_or_exploration_one_is_not_saved() {
        let db = std::env::temp_dir().join(format!(
            "wisp_runtime_config_tool_restart_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&db).await.unwrap();
        for id in ["ssh:gpu", "local"] {
            store
                .upsert_execution_context(&wisp_store::ExecutionContext::new(id, id).unwrap())
                .await
                .unwrap();
        }
        let key = InterpreterKey {
            project_id: "project-1".into(),
            scope_key: wisp_store::MAINLINE_SCOPE_KEY.into(),
            session_id: "frame-1".into(),
            context_id: "ssh:gpu".into(),
            language: InterpreterLanguage::Python,
        };
        let fake = Fake::default();
        fake.running.lock().unwrap().push(key.clone());
        let tool = SetRuntimeInterpreterTool::new_in_session(
            store.clone(),
            Box::new(fake.clone()),
            "project-1",
            wisp_store::MAINLINE_SCOPE_KEY,
            "frame-1",
        )
        .refusing("local", "this host does not read a path saved for 'local'");
        let root = std::env::temp_dir();
        let env = NoEnv(root.clone());

        let result = tool
            .run(
                &serde_json::json!({
                    "context_id": "ssh:gpu",
                    "language": "python",
                    "executable": "/opt/conda/bin/python"
                }),
                &env,
            )
            .await;
        assert!(result.success, "{}", result.content);
        assert!(result
            .content
            .contains("restarted this conversation's Python runtime"));
        assert_eq!(*fake.restarted.lock().unwrap(), vec![(key, root)]);

        // Refused: nothing is saved and nothing restarts.
        let original = store
            .get_execution_context("local")
            .await
            .unwrap()
            .unwrap()
            .config_json;
        let refused = tool
            .run(
                &serde_json::json!({
                    "context_id": " local ",
                    "language": "python",
                    "executable": "/opt/conda/bin/python"
                }),
                &env,
            )
            .await;
        assert!(!refused.success);
        assert_eq!(
            refused.content,
            "set_runtime_interpreter error: this host does not read a path saved for 'local'"
        );
        let saved = store.get_execution_context("local").await.unwrap().unwrap();
        assert_eq!(saved.config_json, original);
        assert_eq!(fake.restarted.lock().unwrap().len(), 1);

        // Inside an exploration nothing is changed either.
        let exploring = SetRuntimeInterpreterTool::new_in_session(
            store.clone(),
            Box::new(fake.clone()),
            "project-1",
            "exploration:e1",
            "frame-1",
        );
        let blocked = exploring
            .run(
                &serde_json::json!({
                    "context_id": "ssh:gpu",
                    "language": "r",
                    "executable": "/usr/bin/Rscript"
                }),
                &env,
            )
            .await;
        assert!(!blocked.success);
        assert!(blocked
            .content
            .starts_with("exploration_project_mutation_blocked"));

        // A restart that fails says the path was saved all the same.
        let failing = Fake {
            fails: true,
            ..Fake::default()
        };
        let r_key = InterpreterKey {
            language: InterpreterLanguage::R,
            ..fake.running.lock().unwrap()[0].clone()
        };
        failing.running.lock().unwrap().push(r_key);
        let tool = SetRuntimeInterpreterTool::new_in_session(
            store.clone(),
            Box::new(failing),
            "project-1",
            wisp_store::MAINLINE_SCOPE_KEY,
            "frame-1",
        );
        let result = tool
            .run(
                &serde_json::json!({
                    "context_id": "ssh:gpu",
                    "language": "r",
                    "executable": "/usr/bin/Rscript"
                }),
                &env,
            )
            .await;
        assert!(!result.success);
        assert!(result
            .content
            .contains("but the runtime failed to restart: worker did not answer"));
        let saved = store
            .get_execution_context("ssh:gpu")
            .await
            .unwrap()
            .unwrap();
        let config: serde_json::Value = serde_json::from_str(&saved.config_json).unwrap();
        assert_eq!(config["python_executable"], "/opt/conda/bin/python");
        assert_eq!(config["rscript_executable"], "/usr/bin/Rscript");
        let _ = std::fs::remove_file(db);
    }
}
