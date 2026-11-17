//! Where a conversation's Python or R interpreter starts for an execution
//! context: the interpreter paths kept on the context, the worker copied to
//! a WSL distro or SSH host, and the command that starts it there. Moved
//! from the Tauri shell's `runtime_launcher` so the embedded host starts
//! interpreters on SSH hosts the same way; spawning the worker stays with
//! each host, which owns `wisp-runtime`.

use crate::{
    run_context::{RunCommand, RunCommandOutput, RunCommandRunner},
    ssh_hosts::SshConnection,
};
use anyhow::{anyhow, Result};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

/// Which interpreter a worker is for. Each host maps its runtime language
/// onto this; `wisp-runs` does not depend on `wisp-runtime`. It reads as
/// `python` or `r`, as `wisp_runtime::RuntimeLanguage` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterpreterLanguage {
    Python,
    R,
}

const DEPLOY_TIMEOUT: Duration = Duration::from_secs(30);
const PRESERVED_CONFIG_KEYS: [&str; 4] = [
    "python_executable",
    "python_path",
    "rscript_executable",
    "rscript_path",
];

pub fn preserve_interpreter_config(existing: &str, replacement: &str) -> Result<String> {
    let existing = json_object(existing, "existing execution context config")?;
    let mut replacement = json_object(replacement, "replacement execution context config")?;
    let target = replacement
        .as_object_mut()
        .expect("json_object always returns an object");
    for key in PRESERVED_CONFIG_KEYS {
        if let Some(value) = existing.get(key) {
            target.insert(key.into(), value.clone());
        }
    }
    Ok(serde_json::to_string(&replacement)?)
}

pub async fn save_interpreter_config(
    store: &wisp_store::Store,
    context_id: &str,
    python_executable: &str,
    rscript_executable: &str,
) -> Result<wisp_store::ExecutionContext> {
    update_interpreter_config(store, context_id, move |object| {
        set_interpreter(
            object,
            "python_executable",
            "python_path",
            python_executable,
        )?;
        set_interpreter(
            object,
            "rscript_executable",
            "rscript_path",
            rscript_executable,
        )
    })
    .await
}

/// Keeps one language's interpreter on a context and leaves the other's as
/// it was: what the agent's `set_runtime_interpreter` saves. Moved from the
/// Tauri shell's `runtime_launcher::save_runtime_interpreter`.
pub async fn save_runtime_interpreter(
    store: &wisp_store::Store,
    context_id: &str,
    language: InterpreterLanguage,
    executable: &str,
) -> Result<wisp_store::ExecutionContext> {
    update_interpreter_config(store, context_id, move |object| match language {
        InterpreterLanguage::Python => {
            set_interpreter(object, "python_executable", "python_path", executable)
        }
        InterpreterLanguage::R => {
            set_interpreter(object, "rscript_executable", "rscript_path", executable)
        }
    })
    .await
}

pub async fn update_interpreter_config(
    store: &wisp_store::Store,
    context_id: &str,
    update: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>) -> Result<()> + Send,
) -> Result<wisp_store::ExecutionContext> {
    let mut context = store
        .get_execution_context(context_id)
        .await?
        .ok_or_else(|| anyhow!("Execution context not found: {context_id}"))?;
    let mut config = json_object(&context.config_json, "execution context config")?;
    let object = config
        .as_object_mut()
        .expect("json_object always returns an object");
    update(object)?;
    context.config_json = serde_json::to_string(&config)?;
    context.updated_at = chrono::Utc::now().timestamp();
    store.upsert_execution_context(&context).await?;
    Ok(context)
}

pub fn set_interpreter(
    config: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    legacy_key: &str,
    value: &str,
) -> Result<()> {
    config.remove(legacy_key);
    let value = value.trim();
    if value.is_empty() {
        config.remove(key);
        return Ok(());
    }
    validate_context_value(key, value).map_err(anyhow::Error::msg)?;
    config.insert(key.into(), serde_json::Value::String(value.into()));
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
}

/// The Python a context's worker starts with: the saved path, then what
/// its probe found, then (on this computer only) what `local` finds.
pub fn resolve_python_interpreter(
    context: &wisp_store::ExecutionContext,
    local: impl FnOnce() -> Option<String>,
) -> Result<String> {
    let config = json_object(&context.config_json, "execution context config")?;
    let capabilities = json_object(&context.capabilities_json, "execution context capabilities")?;
    if let Some(interpreter) = first_string(&config, &["python_executable", "python_path"])? {
        return Ok(interpreter);
    }
    if let Some(interpreter) = first_string(&capabilities, &["python_executable"])? {
        return Ok(interpreter);
    }
    if context.kind == wisp_store::ExecutionContextKind::Local {
        return local()
            .ok_or_else(|| anyhow!("Python interpreter not found; use local-env-setup to prepare Python and save the interpreter for Local"));
    }
    Err(anyhow!(
        "Python interpreter is unknown for {}; probe the context or configure python_executable",
        context.id
    ))
}

/// The `Rscript` a context's worker starts with, as for Python.
pub fn resolve_r_interpreter(
    context: &wisp_store::ExecutionContext,
    local: impl FnOnce() -> Option<String>,
) -> Result<String> {
    let config = json_object(&context.config_json, "execution context config")?;
    let capabilities = json_object(&context.capabilities_json, "execution context capabilities")?;
    if let Some(interpreter) = first_string(&config, &["rscript_executable", "rscript_path"])? {
        ensure_jsonlite_available(context, &capabilities, &interpreter)?;
        return Ok(interpreter);
    }
    if let Some(interpreter) = first_string(&capabilities, &["rscript_executable"])? {
        ensure_jsonlite_available(context, &capabilities, &interpreter)?;
        return Ok(interpreter);
    }
    if context.kind == wisp_store::ExecutionContextKind::Local {
        return local().ok_or_else(|| {
            anyhow!(
                "Rscript not found on PATH; configure the R interpreter for context '{}'",
                context.id
            )
        });
    }
    Err(anyhow!(
        "Rscript interpreter is unknown for {}; probe the context or configure rscript_executable",
        context.id
    ))
}

fn ensure_jsonlite_available(
    context: &wisp_store::ExecutionContext,
    capabilities: &serde_json::Value,
    interpreter: &str,
) -> Result<()> {
    let probed_interpreter = capabilities
        .get("rscript_executable")
        .and_then(serde_json::Value::as_str);
    if probed_interpreter == Some(interpreter)
        && capabilities
            .get("r_jsonlite")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
    {
        return Err(anyhow!(
            "R package 'jsonlite' is not available in {}; install it in the selected R environment",
            context.id
        ));
    }
    Ok(())
}

pub fn build_attached_command(
    context: &wisp_store::ExecutionContext,
    language: InterpreterLanguage,
    interpreter: &str,
    local_worker: &Path,
    remote_worker: Option<&str>,
    project_root: &Path,
) -> Result<AttachedCommand, String> {
    validate_context_value("runtime interpreter", interpreter)?;
    match context.kind {
        wisp_store::ExecutionContextKind::Local => Ok(AttachedCommand {
            program: PathBuf::from(interpreter),
            args: match language {
                InterpreterLanguage::Python => vec![local_worker.as_os_str().to_os_string()],
                InterpreterLanguage::R => vec![
                    OsString::from("--vanilla"),
                    local_worker.as_os_str().to_os_string(),
                ],
            },
            cwd: Some(project_root.to_path_buf()),
        }),
        wisp_store::ExecutionContextKind::Wsl => {
            let worker =
                remote_worker.ok_or_else(|| "remote worker path is required".to_string())?;
            let interpreter = shell_single_quote(interpreter);
            // `wsl.exe` strips double quotes from argv (#1081), so expand
            // `$HOME` without quoting it. Launch stdin is the kernel protocol
            // and cannot carry this command.
            let worker = wsl_argv_path_expression(worker)?;
            let execute = match language {
                InterpreterLanguage::Python => format!("exec {interpreter} {worker}",),
                InterpreterLanguage::R => format!("exec {interpreter} --vanilla {worker}"),
            };
            validate_context_value("project root", &project_root.to_string_lossy())?;
            let distro = wsl_distro(context)?;
            Ok(AttachedCommand {
                program: PathBuf::from("wsl.exe"),
                // Let wsl.exe translate the Windows path itself. Passing the
                // path through `sh -lc` first strips its backslashes before
                // an in-distro `wslpath` can see them (#1065).
                args: vec![
                    OsString::from("-d"),
                    OsString::from(distro),
                    OsString::from("--cd"),
                    project_root.as_os_str().to_os_string(),
                    OsString::from("--"),
                    OsString::from("sh"),
                    OsString::from("-lc"),
                    OsString::from(execute),
                ],
                cwd: None,
            })
        }
        wisp_store::ExecutionContextKind::Ssh => {
            let worker =
                remote_worker.ok_or_else(|| "remote worker path is required".to_string())?;
            let workdir = runtime_workdir(context)?;
            let interpreter = shell_single_quote(interpreter);
            let worker = remote_path_expression(worker)?;
            let script = match language {
                InterpreterLanguage::Python => format!(
                    "cd {} && exec {interpreter} {worker}",
                    remote_path_expression(&workdir)?,
                ),
                InterpreterLanguage::R => format!(
                    "cd {} && exec {interpreter} --vanilla {worker}",
                    remote_path_expression(&workdir)?,
                ),
            };
            let mut args = SshConnection::from_execution_context(context)?.ssh_args()?;
            args.push(script);
            Ok(AttachedCommand {
                program: PathBuf::from("ssh"),
                args: args.into_iter().map(OsString::from).collect(),
                cwd: None,
            })
        }
    }
}

/// Copies the worker `source` to the context once per checksum and answers
/// where it is there; `protocol_version` is the runtime's kernel protocol.
pub async fn ensure_remote_worker(
    context: &wisp_store::ExecutionContext,
    language: InterpreterLanguage,
    protocol_version: u32,
    source: &str,
    runner: &dyn RunCommandRunner,
) -> Result<String, String> {
    let checksum = wisp_sync::sha256_hex(source.as_bytes());
    let (name, extension) = match language {
        InterpreterLanguage::Python => ("python", "py"),
        InterpreterLanguage::R => ("r", "R"),
    };
    let remote_path = format!(
        "~/.wisp-science/runtime/{name}-v{}-{checksum}.{extension}",
        protocol_version
    );
    let check = runner
        .run(
            remote_command(
                context,
                &format!("check {name} runtime worker"),
                checksum_script(&remote_path, &checksum),
                None,
            )?,
            DEPLOY_TIMEOUT,
        )
        .await?;
    if check.exit_code == 0 {
        return Ok(remote_path);
    }
    let deploy = runner
        .run(
            remote_command(
                context,
                &format!("deploy {name} runtime worker"),
                deploy_script(&remote_path, &checksum),
                Some(source.to_string()),
            )?,
            DEPLOY_TIMEOUT,
        )
        .await?;
    checked_command(&format!("{name} runtime worker deployment"), deploy)?;
    Ok(remote_path)
}

fn remote_command(
    context: &wisp_store::ExecutionContext,
    label: &str,
    script: String,
    stdin: Option<String>,
) -> Result<RunCommand, String> {
    match context.kind {
        wisp_store::ExecutionContextKind::Wsl => {
            // `wsl.exe` mangles double quotes in argv (#1081). Feed the shell
            // script through stdin with `sh -s` so `"$dir"` / `"$tmp"` survive.
            // Worker source also has to ride that stdin stream, so embed it as
            // a quoted heredoc instead of a second payload.
            Ok(RunCommand {
                context_id: context.id.clone(),
                program: "wsl.exe".into(),
                args: vec![
                    "-d".into(),
                    wsl_distro(context)?,
                    "--".into(),
                    "sh".into(),
                    "-s".into(),
                ],
                script: label.into(),
                cwd: None,
                stdin: Some(wsl_script_stdin(script, stdin)?),
                envs: Vec::new(),
            })
        }
        wisp_store::ExecutionContextKind::Ssh => {
            let connection = SshConnection::from_execution_context(context)?;
            let mut args = connection.ssh_args()?;
            args.push(format!("sh -lc {}", shell_single_quote(&script)));
            Ok(RunCommand {
                context_id: context.id.clone(),
                program: "ssh".into(),
                args,
                script: label.into(),
                cwd: None,
                stdin,
                envs: crate::ssh_hosts::auth_envs_for_connection(&connection)?,
            })
        }
        wisp_store::ExecutionContextKind::Local => {
            Err("remote deployment requires WSL or SSH".into())
        }
    }
}

fn checksum_script(remote_path: &str, checksum: &str) -> String {
    let path = remote_path_expression(remote_path).expect("generated runtime path is valid");
    format!(
        "hash_file() {{ if command -v sha256sum >/dev/null 2>&1; then sha256sum \"$1\" | cut -d' ' -f1; else shasum -a 256 \"$1\" | cut -d' ' -f1; fi; }}; test -f {path} && test \"$(hash_file {path})\" = {}",
        shell_single_quote(checksum)
    )
}

fn deploy_script(remote_path: &str, checksum: &str) -> String {
    let path = remote_path_expression(remote_path).expect("generated runtime path is valid");
    format!(
        "set -eu\n\
         dir=\"$HOME/.wisp-science/runtime\"\n\
         mkdir -p \"$dir\"\n\
         tmp={path}.tmp.$$\n\
         cat > \"$tmp\"\n\
         if command -v sha256sum >/dev/null 2>&1; then actual=$(sha256sum \"$tmp\" | cut -d' ' -f1); else actual=$(shasum -a 256 \"$tmp\" | cut -d' ' -f1); fi\n\
         if test \"$actual\" != {}; then rm -f \"$tmp\"; exit 1; fi\n\
         chmod 600 \"$tmp\"\n\
         mv -f \"$tmp\" {path}",
        shell_single_quote(checksum)
    )
}

/// `wsl.exe` argv cannot carry a quoted script (#1081). `sh -s` reads the
/// script from stdin; if there is also a worker payload, fold it into that
/// same stream as a quoted heredoc so `cat > "$tmp"` still has something to
/// write.
fn wsl_script_stdin(script: String, payload: Option<String>) -> Result<String, String> {
    match payload {
        None => Ok(script),
        Some(payload) => embed_stdin_payload(&script, &payload),
    }
}

fn embed_stdin_payload(script: &str, payload: &str) -> Result<String, String> {
    const NEEDLE: &str = "cat > \"$tmp\"";
    if !script.contains(NEEDLE) {
        return Err(
            "WSL worker payload requires a script that writes stdin with cat > \"$tmp\"".into(),
        );
    }
    let delimiter = heredoc_delimiter("__WISP_WORKER__", payload);
    let newline = if payload.ends_with('\n') { "" } else { "\n" };
    Ok(script.replacen(
        NEEDLE,
        &format!("cat > \"$tmp\" <<'{delimiter}'\n{payload}{newline}{delimiter}"),
        1,
    ))
}

fn heredoc_delimiter(prefix: &str, body: &str) -> String {
    let mut delimiter = prefix.to_string();
    while body.lines().any(|line| line == delimiter) {
        delimiter.push('X');
    }
    delimiter
}

fn checked_command(label: &str, output: RunCommandOutput) -> Result<(), String> {
    if output.exit_code == 0 {
        return Ok(());
    }
    let detail = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    Err(format!(
        "{label} failed with exit {}: {detail}",
        output.exit_code
    ))
}

fn runtime_workdir(context: &wisp_store::ExecutionContext) -> Result<String, String> {
    let config = json_object(&context.config_json, "execution context config")
        .map_err(|error| error.to_string())?;
    let capabilities = json_object(&context.capabilities_json, "execution context capabilities")
        .map_err(|error| error.to_string())?;
    first_string(&config, &["workdir", "default_workdir"])
        .map_err(|error| error.to_string())?
        .or(first_string(&capabilities, &["pwd", "home"]).map_err(|error| error.to_string())?)
        .map_or_else(
            || Ok("~".into()),
            |value| {
                validate_context_value("runtime workdir", &value)?;
                Ok(value)
            },
        )
}

fn wsl_distro(context: &wisp_store::ExecutionContext) -> Result<String, String> {
    let config = json_object(&context.config_json, "WSL context config")
        .map_err(|error| error.to_string())?;
    let distro = first_string(&config, &["distro"])
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| {
            context
                .id
                .strip_prefix("wsl:")
                .unwrap_or(&context.id)
                .to_string()
        });
    validate_context_value("WSL distro", &distro)?;
    Ok(distro)
}

fn json_object(value: &str, label: &str) -> Result<serde_json::Value> {
    let value: serde_json::Value =
        serde_json::from_str(value).map_err(|error| anyhow!("Invalid {label}: {error}"))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(anyhow!("{label} must be a JSON object"))
    }
}

fn first_string(value: &serde_json::Value, keys: &[&str]) -> Result<Option<String>> {
    for key in keys {
        match value.get(*key) {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(value)) if !value.trim().is_empty() => {
                validate_context_value(key, value).map_err(anyhow::Error::msg)?;
                return Ok(Some(value.clone()));
            }
            Some(serde_json::Value::String(_)) => {}
            Some(_) => return Err(anyhow!("execution context field '{key}' must be a string")),
        }
    }
    Ok(None)
}

fn validate_context_value(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.contains(['\0', '\n', '\r']) {
        Err(format!(
            "{label} must be non-empty and contain no line breaks"
        ))
    } else {
        Ok(())
    }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn remote_path_expression(path: &str) -> Result<String, String> {
    validate_context_value("remote path", path)?;
    if path == "~" {
        Ok("\"$HOME\"".into())
    } else if let Some(rest) = path.strip_prefix("~/") {
        Ok(format!("\"$HOME\"/{}", shell_single_quote(rest)))
    } else {
        Ok(shell_single_quote(path))
    }
}

/// Like [`remote_path_expression`], but without the double quotes that
/// `wsl.exe` strips from argv (#1081). Safe for the default WSL home path.
fn wsl_argv_path_expression(path: &str) -> Result<String, String> {
    validate_context_value("remote path", path)?;
    if path == "~" {
        Ok("$HOME".into())
    } else if let Some(rest) = path.strip_prefix("~/") {
        Ok(format!("$HOME/{}", shell_single_quote(rest)))
    } else {
        Ok(shell_single_quote(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::{collections::VecDeque, sync::Mutex};

    struct FakeRunner {
        outputs: Mutex<VecDeque<RunCommandOutput>>,
        commands: Mutex<Vec<RunCommand>>,
    }

    impl FakeRunner {
        fn new(exit_codes: impl IntoIterator<Item = i64>) -> Self {
            Self {
                outputs: Mutex::new(
                    exit_codes
                        .into_iter()
                        .map(|exit_code| RunCommandOutput {
                            exit_code,
                            stdout: String::new(),
                            stderr: String::new(),
                        })
                        .collect(),
                ),
                commands: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl RunCommandRunner for FakeRunner {
        async fn run(
            &self,
            command: RunCommand,
            _timeout: Duration,
        ) -> Result<RunCommandOutput, String> {
            self.commands.lock().unwrap().push(command);
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| "unexpected command".into())
        }
    }

    #[test]
    fn local_launch_keeps_windows_paths_with_spaces_as_single_arguments() {
        let mut context = wisp_store::ExecutionContext::new("local", "Local").unwrap();
        context.config_json = serde_json::json!({
            "python_executable": r"C:\Program Files\Python\python.exe"
        })
        .to_string();
        let interpreter = resolve_python_interpreter(&context, || None).unwrap();
        let command = build_attached_command(
            &context,
            InterpreterLanguage::Python,
            &interpreter,
            Path::new(r"C:\Program Files\Wisp\kernel_worker.py"),
            None,
            Path::new(r"C:\Research Project"),
        )
        .unwrap();
        assert_eq!(
            command.program,
            PathBuf::from(r"C:\Program Files\Python\python.exe")
        );
        assert_eq!(command.args.len(), 1);
        assert_eq!(
            command.args[0],
            OsString::from(r"C:\Program Files\Wisp\kernel_worker.py")
        );

        context.config_json = serde_json::json!({
            "rscript_executable": r"C:\Program Files\R\bin\Rscript.exe"
        })
        .to_string();
        let rscript = resolve_r_interpreter(&context, || None).unwrap();
        let command = build_attached_command(
            &context,
            InterpreterLanguage::R,
            &rscript,
            Path::new(r"C:\Program Files\Wisp\kernel_worker.R"),
            None,
            Path::new(r"C:\Research Project"),
        )
        .unwrap();
        assert_eq!(command.args[0], OsString::from("--vanilla"));
        assert_eq!(
            command.args[1],
            OsString::from(r"C:\Program Files\Wisp\kernel_worker.R")
        );
    }

    #[test]
    fn wsl_uses_host_project_root_as_cd_argument_while_ssh_preserves_context_workdir() {
        let mut wsl = wisp_store::ExecutionContext::new("wsl:Ubuntu-24.04", "WSL").unwrap();
        wsl.config_json = serde_json::json!({
            "distro": "Ubuntu 24.04",
            "workdir": "/scratch/project one"
        })
        .to_string();
        wsl.capabilities_json = serde_json::json!({
            "python_executable": "/opt/conda env/bin/python"
        })
        .to_string();
        let wsl_python = resolve_python_interpreter(&wsl, || None).unwrap();
        let wsl_command = build_attached_command(
            &wsl,
            InterpreterLanguage::Python,
            &wsl_python,
            Path::new("unused"),
            Some("~/.wisp-science/runtime/python.py"),
            Path::new(r"C:\Users\me\project one"),
        )
        .unwrap();
        let wsl_args = wsl_command
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(wsl_command.program, PathBuf::from("wsl.exe"));
        assert_eq!(
            &wsl_args[..7],
            [
                "-d",
                "Ubuntu 24.04",
                "--cd",
                r"C:\Users\me\project one",
                "--",
                "sh",
                "-lc"
            ]
        );
        let wsl_script = wsl_args.last().unwrap();
        assert!(!wsl_script.contains("wslpath"), "{wsl_script}");
        assert!(!wsl_script.contains(r"C:\Users\me"), "{wsl_script}");
        assert!(!wsl_script.contains("/scratch/project one"), "{wsl_script}");
        assert!(
            !wsl_script.contains('"'),
            "wsl.exe strips double quotes from argv (#1081): {wsl_script}"
        );
        assert!(wsl_script.contains("$HOME/"), "{wsl_script}");

        let mut ssh = wisp_store::ExecutionContext::new("ssh:gpu-box", "GPU").unwrap();
        ssh.config_json = serde_json::json!({
            "user": "alice",
            "port": 2222,
            "identity_file": "/home/alice/.ssh/lab key",
            "python_executable": "/opt/python/bin/python",
            "workdir": "/scratch/ssh project"
        })
        .to_string();
        let ssh_command = build_attached_command(
            &ssh,
            InterpreterLanguage::Python,
            "/opt/python/bin/python",
            Path::new("unused"),
            Some("~/.wisp-science/runtime/python.py"),
            Path::new("unused"),
        )
        .unwrap();
        let ssh_args = ssh_command
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(ssh_command.program, PathBuf::from("ssh"));
        assert!(ssh_args.windows(2).any(|args| args == ["-p", "2222"]));
        assert!(ssh_args
            .windows(2)
            .any(|args| args == ["-i", "/home/alice/.ssh/lab key"]));
        assert!(ssh_args.contains(&"alice@gpu-box".to_string()));
        assert!(ssh_args.last().unwrap().contains("/scratch/ssh project"));

        wsl.capabilities_json = serde_json::json!({
            "rscript_executable": "/opt/R/bin/Rscript",
            "r_jsonlite": true
        })
        .to_string();
        let rscript = resolve_r_interpreter(&wsl, || None).unwrap();
        let r_command = build_attached_command(
            &wsl,
            InterpreterLanguage::R,
            &rscript,
            Path::new("unused"),
            Some("~/.wisp-science/runtime/r.R"),
            Path::new(r"C:\Users\me\project one"),
        )
        .unwrap();
        assert!(r_command
            .args
            .last()
            .unwrap()
            .to_string_lossy()
            .contains("--vanilla"));
        assert_eq!(
            r_command.args[3],
            OsString::from(r"C:\Users\me\project one")
        );
        assert!(!r_command
            .args
            .last()
            .unwrap()
            .to_string_lossy()
            .contains("wslpath"));
        assert!(
            !r_command
                .args
                .last()
                .unwrap()
                .to_string_lossy()
                .contains('"'),
            "wsl.exe strips double quotes from argv (#1081)"
        );
    }

    #[test]
    fn wsl_launch_keeps_a_native_home_project_out_of_the_shell_script() {
        let mut wsl = wisp_store::ExecutionContext::new("wsl:Ubuntu-22.04", "WSL").unwrap();
        wsl.config_json = serde_json::json!({
            "distro": "Ubuntu-22.04",
            "python_executable": "/usr/bin/python3"
        })
        .to_string();
        let command = build_attached_command(
            &wsl,
            InterpreterLanguage::Python,
            "/usr/bin/python3",
            Path::new("unused"),
            Some("~/.wisp-science/runtime/python.py"),
            Path::new(r"\\wsl$\Ubuntu-22.04\home\me\project"),
        )
        .unwrap();
        let args = command
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            &args[..7],
            [
                "-d",
                "Ubuntu-22.04",
                "--cd",
                r"\\wsl$\Ubuntu-22.04\home\me\project",
                "--",
                "sh",
                "-lc"
            ]
        );
        let script = args.last().unwrap();
        assert!(!script.contains("wslpath"), "{script}");
        assert!(!script.contains(r"\\wsl$"), "{script}");
        assert!(!script.contains('"'), "{script}");
    }

    #[test]
    fn known_missing_jsonlite_is_an_actionable_r_capability_error() {
        let mut context = wisp_store::ExecutionContext::new("ssh:r-box", "R").unwrap();
        context.capabilities_json = serde_json::json!({
            "rscript_executable": "/usr/bin/Rscript",
            "r_jsonlite": false
        })
        .to_string();
        let error = resolve_r_interpreter(&context, || None).unwrap_err();
        assert!(error.to_string().contains("jsonlite"));
        assert!(error.to_string().contains("install"));

        context.config_json = serde_json::json!({
            "rscript_executable": "/opt/project-R/bin/Rscript"
        })
        .to_string();
        assert_eq!(
            resolve_r_interpreter(&context, || None).unwrap(),
            "/opt/project-R/bin/Rscript"
        );
    }

    #[tokio::test]
    async fn interpreter_config_is_persisted_per_context_and_preserves_transport_fields() {
        let db = std::env::temp_dir().join(format!(
            "wisp_runtime_config_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&db).await.unwrap();
        let mut context = wisp_store::ExecutionContext::new("ssh:cpu2", "CPU2").unwrap();
        context.config_json = serde_json::json!({
            "alias": "cpu2",
            "workdir": "/data/project",
            "python_path": "/legacy/python",
            "rscript_path": "/legacy/Rscript"
        })
        .to_string();
        store.upsert_execution_context(&context).await.unwrap();

        let saved = save_interpreter_config(
            &store,
            "ssh:cpu2",
            "/opt/conda/envs/research/bin/python",
            "/opt/R/4.5/bin/Rscript",
        )
        .await
        .unwrap();
        let config: serde_json::Value = serde_json::from_str(&saved.config_json).unwrap();
        assert_eq!(config["alias"], "cpu2");
        assert_eq!(config["workdir"], "/data/project");
        assert_eq!(
            config["python_executable"],
            "/opt/conda/envs/research/bin/python"
        );
        assert_eq!(config["rscript_executable"], "/opt/R/4.5/bin/Rscript");
        assert!(config.get("python_path").is_none());
        assert!(config.get("rscript_path").is_none());

        let cleared = save_interpreter_config(&store, "ssh:cpu2", "", "")
            .await
            .unwrap();
        let config: serde_json::Value = serde_json::from_str(&cleared.config_json).unwrap();
        assert_eq!(config["alias"], "cpu2");
        assert!(config.get("python_executable").is_none());
        assert!(config.get("rscript_executable").is_none());
        let _ = std::fs::remove_file(db);
    }

    #[test]
    fn interpreter_config_rejects_line_breaks() {
        let mut config = serde_json::Map::new();
        let error = set_interpreter(
            &mut config,
            "python_executable",
            "python_path",
            "/opt/python\n--version",
        )
        .unwrap_err();
        assert!(error.to_string().contains("line breaks"));
    }

    #[tokio::test]
    async fn remote_deployment_skips_checksum_hits_and_uploads_misses() {
        let context = wisp_store::ExecutionContext::new("wsl:Ubuntu", "WSL").unwrap();
        let hit = FakeRunner::new([0]);
        let path = ensure_remote_worker(
            &context,
            InterpreterLanguage::Python,
            1,
            "print('worker')",
            &hit,
        )
        .await
        .unwrap();
        assert!(path.contains("python-v1-"));
        let hit_commands = hit.commands.lock().unwrap();
        assert_eq!(hit_commands.len(), 1);
        assert_eq!(
            hit_commands[0].args,
            vec![
                "-d".to_string(),
                "Ubuntu".to_string(),
                "--".to_string(),
                "sh".to_string(),
                "-s".to_string()
            ]
        );
        drop(hit_commands);

        let miss = FakeRunner::new([1, 0]);
        ensure_remote_worker(
            &context,
            InterpreterLanguage::R,
            1,
            "print('worker')",
            &miss,
        )
        .await
        .unwrap();
        let commands = miss.commands.lock().unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].script, "check r runtime worker");
        assert_eq!(commands[1].script, "deploy r runtime worker");
        let deploy_stdin = commands[1].stdin.as_deref().unwrap();
        assert!(deploy_stdin.contains("print('worker')"), "{deploy_stdin}");
        assert!(
            deploy_stdin.contains("<<'__WISP_WORKER__'"),
            "{deploy_stdin}"
        );
    }

    #[test]
    fn wsl_worker_commands_feed_quoted_scripts_on_stdin() {
        let mut context = wisp_store::ExecutionContext::new("wsl:Ubuntu-22.04", "WSL").unwrap();
        context.config_json = serde_json::json!({ "distro": "Ubuntu-22.04" }).to_string();
        let wsl_args = vec![
            "-d".to_string(),
            "Ubuntu-22.04".to_string(),
            "--".to_string(),
            "sh".to_string(),
            "-s".to_string(),
        ];

        let check = remote_command(
            &context,
            "check python runtime worker",
            checksum_script("~/.wisp-science/runtime/python.py", "abc123"),
            None,
        )
        .unwrap();
        assert_eq!(check.program, "wsl.exe");
        assert_eq!(check.args, wsl_args);
        assert!(
            check.args.iter().all(|arg| !arg.contains('"')),
            "{:?}",
            check.args
        );
        let check_stdin = check.stdin.as_deref().unwrap();
        assert!(check_stdin.contains("sha256sum \"$1\""), "{check_stdin}");
        assert!(check_stdin.contains("\"$HOME\""), "{check_stdin}");

        let worker = "print('worker')\n__WISP_WORKER__\nmore";
        let deploy = remote_command(
            &context,
            "deploy python runtime worker",
            deploy_script("~/.wisp-science/runtime/python.py", "abc123"),
            Some(worker.into()),
        )
        .unwrap();
        assert_eq!(deploy.args, wsl_args);
        assert!(
            deploy.args.iter().all(|arg| !arg.contains('"')),
            "{:?}",
            deploy.args
        );
        let deploy_stdin = deploy.stdin.as_deref().unwrap();
        assert!(deploy_stdin.contains("mkdir -p \"$dir\""), "{deploy_stdin}");
        assert!(
            deploy_stdin.contains("sha256sum \"$tmp\""),
            "{deploy_stdin}"
        );
        assert!(
            deploy_stdin.contains("<<'__WISP_WORKER__X'"),
            "{deploy_stdin}"
        );
        assert!(deploy_stdin.contains(worker), "{deploy_stdin}");
        assert!(
            !deploy
                .args
                .iter()
                .any(|arg| arg.contains("mkdir") || arg.contains("$HOME")),
            "quoted deploy script must not be in wsl.exe argv: {:?}",
            deploy.args
        );
    }

    #[test]
    fn ssh_worker_commands_keep_the_payload_on_ssh_stdin() {
        let mut context = wisp_store::ExecutionContext::new("ssh:gpu", "GPU").unwrap();
        context.config_json = serde_json::json!({
            "user": "alice",
            "identity_file": "/home/alice/.ssh/lab"
        })
        .to_string();
        let deploy = remote_command(
            &context,
            "deploy python runtime worker",
            deploy_script("~/.wisp-science/runtime/python.py", "abc123"),
            Some("print('worker')".into()),
        )
        .unwrap();
        assert_eq!(deploy.program, "ssh");
        assert_eq!(deploy.stdin.as_deref(), Some("print('worker')"));
        let remote = deploy.args.last().unwrap();
        assert!(remote.starts_with("sh -lc "), "{remote}");
        assert!(remote.contains("mkdir -p \"$dir\""), "{remote}");
    }

    #[test]
    fn wsl_worker_payload_requires_the_stdin_cat_sink() {
        let context = wisp_store::ExecutionContext::new("wsl:Ubuntu", "WSL").unwrap();
        let error = remote_command(
            &context,
            "deploy python runtime worker",
            "echo ready".into(),
            Some("print('worker')".into()),
        )
        .unwrap_err();
        assert!(error.contains("cat > \"$tmp\""), "{error}");
    }
}
