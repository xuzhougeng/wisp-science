mod eval;

use anyhow::{bail, Result};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use wisp_core::{Agent, MemoryManager, Output};
use wisp_llm::ProviderConfig;
use wisp_skills::SkillIndex;

const HELP: &str = "Built-in commands:\n  /q, /quit       Quit\n  /n, /new        Start a new session (old session is backed up)\n  /c, /compact    Compact the context (full history is archived to .wisp/history/ first)\n  /h, /help       Show this help";
const USAGE: &str = "Usage:
  wisp-science
  wisp-science run [--output console|jsonl] <prompt>
  wisp-science eval [--save report.json] [--compare baseline.json]
  wisp-science dev

With no command, wisp-science starts the interactive terminal.";

#[derive(Debug, PartialEq, Eq)]
enum CliCommand {
    Interactive,
    Run {
        prompt: String,
        output: OutputFormat,
    },
    Eval {
        save: Option<PathBuf>,
        compare: Option<PathBuf>,
    },
    Dev,
    Help,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OutputFormat {
    #[default]
    Console,
    Jsonl,
}

fn parse_output(value: &str) -> Result<OutputFormat> {
    match value {
        "console" => Ok(OutputFormat::Console),
        "jsonl" => Ok(OutputFormat::Jsonl),
        _ => bail!("unknown output format '{value}'; expected console or jsonl"),
    }
}

fn parse_command(args: impl IntoIterator<Item = String>) -> Result<CliCommand> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Ok(CliCommand::Interactive);
    };

    match command.as_str() {
        "dev" => {
            if args.next().is_some() {
                bail!("dev does not accept arguments");
            }
            Ok(CliCommand::Dev)
        }
        "-h" | "--help" | "help" => Ok(CliCommand::Help),
        "run" => {
            let mut output = OutputFormat::Console;
            let mut prompt = Vec::new();
            let mut options = true;
            while let Some(arg) = args.next() {
                if options && arg == "--" {
                    options = false;
                } else if options && arg == "--output" {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--output requires a value"))?;
                    output = parse_output(&value)?;
                } else if options {
                    if let Some(value) = arg.strip_prefix("--output=") {
                        output = parse_output(value)?;
                    } else if arg.starts_with('-') {
                        bail!("unknown run option '{arg}'");
                    } else {
                        prompt.push(arg);
                    }
                } else {
                    prompt.push(arg);
                }
            }
            if prompt.is_empty() {
                bail!("run requires a prompt");
            }
            Ok(CliCommand::Run {
                prompt: prompt.join(" "),
                output,
            })
        }
        "eval" => {
            let mut save = None;
            let mut compare = None;
            while let Some(arg) = args.next() {
                let target = match arg.as_str() {
                    "--save" => &mut save,
                    "--compare" => &mut compare,
                    _ => bail!("unknown eval option '{arg}'"),
                };
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{arg} requires a path"))?;
                if target.replace(PathBuf::from(value)).is_some() {
                    bail!("{arg} may only be specified once");
                }
            }
            Ok(CliCommand::Eval { save, compare })
        }
        _ => bail!("unknown command '{command}'\n\n{USAGE}"),
    }
}

struct CliOutput;
impl CliOutput {
    fn dim(&self) -> &'static str {
        if std::io::stdout().is_terminal() {
            "\x1b[2m"
        } else {
            ""
        }
    }
    fn bold(&self) -> &'static str {
        if std::io::stdout().is_terminal() {
            "\x1b[1m"
        } else {
            ""
        }
    }
    fn cyan(&self) -> &'static str {
        if std::io::stdout().is_terminal() {
            "\x1b[36m"
        } else {
            ""
        }
    }
    fn green(&self) -> &'static str {
        if std::io::stdout().is_terminal() {
            "\x1b[32m"
        } else {
            ""
        }
    }
    fn red(&self) -> &'static str {
        if std::io::stdout().is_terminal() {
            "\x1b[31m"
        } else {
            ""
        }
    }
    fn yellow(&self) -> &'static str {
        if std::io::stdout().is_terminal() {
            "\x1b[33m"
        } else {
            ""
        }
    }
    fn reset(&self) -> &'static str {
        if std::io::stdout().is_terminal() {
            "\x1b[0m"
        } else {
            ""
        }
    }
}

impl Output for CliOutput {
    fn assistant_text(&self, delta: &str) {
        print!("{delta}");
        std::io::stdout().flush().ok();
    }
    fn reasoning(&self, delta: &str) {
        print!("{}{}{}", self.dim(), delta, self.reset());
        std::io::stdout().flush().ok();
    }
    fn tool_call(&self, name: &str, preview: &str) {
        println!(
            "\n{}{} {}{} {}{}{}",
            self.cyan(),
            "›",
            name,
            self.reset(),
            self.dim(),
            preview,
            self.reset()
        );
    }
    fn tool_result(&self, name: &str, ok: bool, content: &str, _duration_ms: u64) {
        let icon = if ok { "✓" } else { "✗" };
        let color = if ok { self.green() } else { self.red() };
        println!(
            " {}{}{} {}{}",
            color,
            icon,
            self.reset(),
            self.dim(),
            self.reset()
        );
        // Truncate verbose tool results in the terminal.
        let lines: Vec<&str> = content.lines().collect();
        let show: Vec<&str> = lines.iter().take(20).copied().collect();
        for l in show {
            let l: String = l.chars().take(200).collect();
            println!(" {}⎿ {}{}", self.dim(), l, self.reset());
        }
        if lines.len() > 20 {
            println!(
                " {}⎿ ... and {} more lines{}",
                self.dim(),
                lines.len() - 20,
                self.reset()
            );
        }
        let _ = name;
    }
    fn usage(
        &self,
        round: usize,
        input: u64,
        output: u64,
        reasoning: u64,
        cached: u64,
        ctx_tokens: usize,
        max_context: usize,
    ) {
        let pct = if max_context > 0 {
            (ctx_tokens * 100 / max_context).min(100)
        } else {
            0
        };
        let color = if pct < 50 {
            self.green()
        } else if pct < 70 {
            self.yellow()
        } else {
            self.red()
        };
        let reasoning = if reasoning > 0 {
            format!(" ({reasoning} reasoning)")
        } else {
            String::new()
        };
        let cached = if cached > 0 {
            format!(" ({cached} cached)")
        } else {
            String::new()
        };
        println!(
            "\n{}round {}: {}k in{} / {}k out{} | ctx: {}%{}{}",
            self.dim(),
            round,
            input / 1000,
            cached,
            output / 1000,
            reasoning,
            color,
            pct,
            self.reset()
        );
    }
    fn compaction(&self, before: usize, after: usize, strategy: &str) {
        println!(
            "{}[compact {}] {} → {} (-{}){}",
            self.yellow(),
            strategy,
            before,
            after,
            before.saturating_sub(after),
            self.reset()
        );
    }
    fn context_warning(&self, ctx_tokens: usize, max_context: usize) {
        let pct = if max_context > 0 {
            (ctx_tokens * 100 / max_context).min(100)
        } else {
            0
        };
        println!(
            "\n{}context is at {pct}% of {}k tokens — run /compact to fold old turns (the full history is archived first, so nothing is lost){}",
            self.yellow(),
            max_context / 1000,
            self.reset()
        );
    }
    fn diff(&self, path: &str, _old: &str, _new: &str) {
        println!("{}diff: {}{}", self.cyan(), path, self.reset());
    }
    fn stdout_chunk(&self, chunk: &str) {
        print!("{}{}{}", self.dim(), chunk, self.reset());
        std::io::stdout().flush().ok();
    }
    fn confirm(&self, message: &str) -> bool {
        println!("{}{} [y/n]: {}", self.yellow(), message, self.reset());
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }
}

struct JsonlOutput<W> {
    writer: Mutex<W>,
}

impl<W: Write + Send> JsonlOutput<W> {
    fn new(writer: W) -> Self {
        Self {
            writer: Mutex::new(writer),
        }
    }

    fn emit(&self, event: serde_json::Value) {
        let Ok(mut writer) = self.writer.lock() else {
            return;
        };
        if serde_json::to_writer(&mut *writer, &event).is_ok() {
            let _ = writer.write_all(b"\n");
            let _ = writer.flush();
        }
    }

    fn start(&self, prompt: &str, model: &str, root: &std::path::Path) {
        self.emit(serde_json::json!({
            "type": "start",
            "prompt": prompt,
            "model": model,
            "root": root,
        }));
    }

    fn done(&self) {
        self.emit(serde_json::json!({"type": "done", "ok": true}));
    }

    fn error(&self, error: &anyhow::Error) {
        self.emit(serde_json::json!({
            "type": "error",
            "message": error.to_string(),
        }));
    }

    #[cfg(test)]
    fn into_inner(self) -> W {
        self.writer
            .into_inner()
            .expect("JSONL writer mutex poisoned")
    }
}

impl<W: Write + Send> Output for JsonlOutput<W> {
    fn assistant_text(&self, delta: &str) {
        self.emit(serde_json::json!({"type": "text", "delta": delta}));
    }

    fn reasoning(&self, delta: &str) {
        self.emit(serde_json::json!({"type": "reasoning", "delta": delta}));
    }

    fn tool_call(&self, name: &str, preview: &str) {
        self.emit(serde_json::json!({
            "type": "tool_call",
            "name": name,
            "preview": preview,
        }));
    }

    fn tool_result(&self, name: &str, ok: bool, content: &str, duration_ms: u64) {
        self.emit(serde_json::json!({
            "type": "tool_result",
            "name": name,
            "ok": ok,
            "content": content,
            "duration_ms": duration_ms,
        }));
    }

    fn usage(
        &self,
        round: usize,
        input: u64,
        output: u64,
        reasoning: u64,
        cached: u64,
        ctx_tokens: usize,
        max_context: usize,
    ) {
        self.emit(serde_json::json!({
            "type": "usage",
            "round": round,
            "input_tokens": input,
            "output_tokens": output,
            "reasoning_tokens": reasoning,
            "cached_tokens": cached,
            "context_tokens": ctx_tokens,
            "max_context_tokens": max_context,
        }));
    }

    fn compaction(&self, before: usize, after: usize, strategy: &str) {
        self.emit(serde_json::json!({
            "type": "compaction",
            "before_tokens": before,
            "after_tokens": after,
            "strategy": strategy,
        }));
    }

    fn context_warning(&self, ctx_tokens: usize, max_context: usize) {
        self.emit(serde_json::json!({
            "type": "context_warning",
            "context_tokens": ctx_tokens,
            "max_context_tokens": max_context,
        }));
    }

    fn diff(&self, path: &str, old: &str, new: &str) {
        self.emit(serde_json::json!({
            "type": "diff",
            "path": path,
            "old": old,
            "new": new,
        }));
    }

    fn file_changed(&self, path: &str) {
        self.emit(serde_json::json!({"type": "file_changed", "path": path}));
    }

    fn stdout_chunk(&self, chunk: &str) {
        self.emit(serde_json::json!({"type": "stdout", "chunk": chunk}));
    }

    fn tool_presentation(&self, kind: &str, payload: &serde_json::Value) {
        self.emit(serde_json::json!({
            "type": "tool_presentation",
            "kind": kind,
            "payload": payload,
        }));
    }

    fn confirm(&self, message: &str) -> bool {
        self.emit(serde_json::json!({
            "type": "approval_required",
            "message": message,
            "approved": false,
        }));
        false
    }
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn setup_message(jsonl: bool, message: std::fmt::Arguments<'_>) {
    if jsonl {
        eprintln!("{message}");
    } else {
        println!("{message}");
    }
}

async fn wire_mcp(agent: &mut Agent, command: &str, args: &[String], jsonl: bool) {
    match wisp_mcp::McpClient::launch(command, args).await {
        Ok(client) => {
            register_mcp_tools(
                agent,
                std::sync::Arc::new(client),
                &format!("{command} {}", args.join(" ")),
                jsonl,
            )
            .await
        }
        Err(e) => setup_message(jsonl, format_args!("mcp launch failed: {e}")),
    }
}

async fn register_mcp_tools(
    agent: &mut Agent,
    client: std::sync::Arc<wisp_mcp::McpClient>,
    label: &str,
    jsonl: bool,
) {
    match client.tools_list().await {
        Ok(tools) => {
            let n = tools.len();
            for t in tools {
                agent.add_tool(Box::new(wisp_mcp::McpTool::new(t, client.clone())));
            }
            setup_message(
                jsonl,
                format_args!("mcp wired: {n} tool(s) from '{label}'."),
            );
        }
        Err(e) => setup_message(jsonl, format_args!("mcp tools_list failed: {e}")),
    }
}

fn provider_config() -> Result<ProviderConfig> {
    let kind = match env("WISP_PROVIDER", "openai").to_ascii_lowercase().as_str() {
        "anthropic" => "anthropic".to_string(),
        "openai_responses" | "openai-responses" | "responses" => "openai_responses".to_string(),
        _ => "openai".to_string(),
    };
    let api_key = env("WISP_API_KEY", "");
    let base_url = env(
        "WISP_API_URL",
        match kind.as_str() {
            "anthropic" => "https://api.anthropic.com",
            "openai_responses" => "https://api.openai.com/v1",
            _ => "https://api.deepseek.com",
        },
    );
    let model = env(
        "WISP_MODEL",
        match kind.as_str() {
            "anthropic" => "claude-sonnet-5",
            "openai_responses" => "gpt-5.5",
            _ => "deepseek-v4-pro",
        },
    );
    if api_key.is_empty() {
        anyhow::bail!("WISP_API_KEY is not set (required). Set it to your provider API key.");
    }
    Ok(match kind.as_str() {
        "anthropic" => ProviderConfig::anthropic(base_url, api_key, model),
        "openai_responses" => ProviderConfig::openai_responses(base_url, api_key, model),
        _ => ProviderConfig::openai(base_url, api_key, model),
    })
}

fn skill_paths(root: &std::path::Path) -> Vec<PathBuf> {
    let mut paths = vec![];
    // Bundled catalog shipped inside the Wisp source tree (wisp/skills).
    if let Some(b) = wisp_skills::bundled_dir() {
        paths.push(b);
    }
    paths.push(root.join(".wisp").join("skills"));
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".wisp").join("skills"));
    }
    if let Ok(extra) = std::env::var("WISP_SKILLS_PATH") {
        for p in extra.split([':', ';']).filter(|s| !s.is_empty()) {
            paths.push(PathBuf::from(p));
        }
    }
    paths
}

async fn run_prompt(agent: &mut Agent, prompt: &str, output: &dyn Output) -> Result<()> {
    let stamped = format!(
        "{}, Current date: {}",
        prompt,
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    let result = agent.run(&stamped, output, None).await;
    agent.ctx.clear_runtime_injections();
    agent.save();
    result
}

#[tokio::main]
async fn main() -> Result<()> {
    let command = parse_command(std::env::args().skip(1))?;
    if command == CliCommand::Help {
        println!("{USAGE}");
        return Ok(());
    }
    // `cargo run dev` passes "dev" as argv[1]; forward to the desktop shell.
    if command == CliCommand::Dev {
        let status = std::process::Command::new("cargo")
            .args(["tauri", "dev"])
            .status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
    let jsonl = matches!(
        &command,
        CliCommand::Run {
            output: OutputFormat::Jsonl,
            ..
        }
    );

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("wisp=info".parse()?),
        )
        .init();

    let cfg = match provider_config() {
        Ok(cfg) => cfg,
        Err(error) => {
            if jsonl {
                JsonlOutput::new(std::io::stdout()).error(&error);
            }
            return Err(error);
        }
    };
    if let CliCommand::Eval { save, compare } = &command {
        return eval::run(cfg, save.as_deref(), compare.as_deref()).await;
    }
    let root = match std::env::current_dir() {
        Ok(root) => root,
        Err(error) => {
            let error = anyhow::Error::from(error);
            if jsonl {
                JsonlOutput::new(std::io::stdout()).error(&error);
            }
            return Err(error);
        }
    };
    let max_context = env("WISP_MAX_CONTEXT", "1000000")
        .parse::<usize>()
        .unwrap_or(1_000_000);
    let max_iter = env("WISP_MAX_ITER", "100").parse::<usize>().unwrap_or(100);

    let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
    let memory = Arc::new(MemoryManager::new(&root));

    let mut agent = Agent::new(
        cfg,
        skills.clone(),
        memory.clone(),
        root.clone(),
        max_context,
        max_iter,
        true,
        None,
    );
    agent.seed_system_prompt(&skills, None);

    // Provision a uv venv once; shared by the Python REPL and the bundled
    // bio-tools MCP server. Skipped silently if uv isn't installed.
    let app_data = root.join(".wisp");
    let py_env = wisp_runtime::PythonEnv::ensure(&app_data).ok();

    // Python REPL: needs a kernel_worker path. Default to the bundled worker.
    let worker = std::env::var("WISP_KERNEL_WORKER")
        .ok()
        .or_else(|| wisp_runtime::bundled_worker_path().map(|p| p.to_string_lossy().to_string()))
        .unwrap_or_default();
    let worker_path = wisp_runtime::resolve_bundled_script(&worker);
    let r_worker = std::env::var("WISP_R_KERNEL_WORKER")
        .ok()
        .or_else(|| {
            wisp_runtime::bundled_r_worker_path().map(|path| path.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let r_worker_path = wisp_runtime::resolve_bundled_script(&r_worker);
    let runtime_manager = wisp_runtime::RuntimeManager::local(
        app_data.clone(),
        worker_path.clone(),
        Some(r_worker_path.clone()),
        vec![],
    );
    if worker_path.is_file() {
        if py_env.is_some() {
            agent.add_tool(Box::new(wisp_runtime::ReplTool::new(
                runtime_manager.clone(),
                root.to_string_lossy(),
            )));
            setup_message(jsonl, format_args!("python repl wired ({worker})."));
        } else {
            setup_message(
                jsonl,
                format_args!("python repl skipped: uv venv unavailable"),
            );
        }
    } else {
        setup_message(
            jsonl,
            format_args!("(kernel worker not found at {worker}; set WISP_KERNEL_WORKER=<path>)"),
        );
    }

    if r_worker_path.is_file() {
        agent.add_tool(Box::new(wisp_runtime::RTool::new(
            runtime_manager.clone(),
            root.to_string_lossy(),
        )));
        setup_message(jsonl, format_args!("r repl wired ({r_worker})."));
    } else {
        setup_message(
            jsonl,
            format_args!("(R worker not found at {r_worker}; set WISP_R_KERNEL_WORKER=<path>)"),
        );
    }

    // MCP server: WISP_MCP_COMMAND overrides; otherwise WISP_MCP_PKG launches
    // the bundled bio-tools server (<pkg> e.g. mcp_pubmed) via the venv python.
    if let Ok(cmdline) = std::env::var("WISP_MCP_COMMAND") {
        let parts: Vec<String> = cmdline
            .split_whitespace()
            .map(|s| {
                if s.ends_with(".py") {
                    wisp_runtime::resolve_bundled_script(s)
                        .to_string_lossy()
                        .to_string()
                } else {
                    s.to_string()
                }
            })
            .collect();
        if parts.len() >= 2 {
            let args: Vec<String> = parts[1..].to_vec();
            wire_mcp(&mut agent, &parts[0], &args, jsonl).await;
        }
    } else if let Some(env) = &py_env {
        let pkg = std::env::var("WISP_MCP_PKG").unwrap_or_else(|_| "mcp_bio".into());
        match wisp_mcp::McpClient::launch_bio_tools(&env.python(), &pkg, &[]).await {
            Ok(client) => {
                register_mcp_tools(
                    &mut agent,
                    std::sync::Arc::new(client),
                    &format!("bio-tools:{pkg}"),
                    jsonl,
                )
                .await
            }
            Err(e) => setup_message(
                jsonl,
                format_args!("mcp bio-tools:{pkg} launch failed: {e}"),
            ),
        }
    }

    let out = CliOutput;
    if let CliCommand::Run { prompt, output } = command {
        let result = match output {
            OutputFormat::Console => {
                let result = run_prompt(&mut agent, &prompt, &out).await;
                println!();
                result
            }
            OutputFormat::Jsonl => {
                let jsonl_out = JsonlOutput::new(std::io::stdout());
                jsonl_out.start(&prompt, agent.provider.model(), &root);
                let result = run_prompt(&mut agent, &prompt, &jsonl_out).await;
                match &result {
                    Ok(()) => jsonl_out.done(),
                    Err(error) => jsonl_out.error(error),
                }
                result
            }
        };
        runtime_manager.shutdown_all().await;
        return result;
    }

    println!(
        "{}wisp-science{} | {} | {}",
        out.bold(),
        out.reset(),
        agent.provider.model(),
        root.display()
    );
    if skills.is_empty() {
        println!(
            "{}(no skills loaded; set WISP_SKILLS_PATH to a SKILL.md catalog){}",
            out.dim(),
            out.reset()
        );
    }
    println!("{HELP}\n");

    let stdin = std::io::stdin();
    loop {
        print!("{}❯{} ", out.bold(), out.reset());
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if stdin.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let input = line.trim().to_string();
        if input.is_empty() {
            continue;
        }
        match input.as_str() {
            "/q" | "/quit" => break,
            "/h" | "/help" => {
                println!("{HELP}");
                continue;
            }
            "/c" | "/compact" => {
                match agent.compact().await {
                    Ok((before, after, archive)) => {
                        out.compaction(before, after, "manual");
                        println!(
                            "{}full history archived to {}{}",
                            out.dim(),
                            archive.display(),
                            out.reset()
                        );
                        agent.save();
                    }
                    Err(e) => println!("{}compact failed: {e}{}", out.red(), out.reset()),
                }
                continue;
            }
            "/n" | "/new" => {
                agent.ctx.backup(&agent.session_path);
                agent.ctx.clear();
                agent.seed_system_prompt(&skills, None);
                println!("{}New session created.{}", out.green(), out.reset());
                agent.save();
                continue;
            }
            _ => {}
        }

        if let Err(e) = run_prompt(&mut agent, &input, &out).await {
            eprintln!("{}Error: {}{}", out.red(), e, out.reset());
        }
        println!();
    }
    runtime_manager.shutdown_all().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(args: &[&str]) -> Result<CliCommand> {
        parse_command(args.iter().map(|arg| (*arg).to_string()))
    }

    #[test]
    fn parses_interactive_and_dev_commands() {
        assert_eq!(command(&[]).unwrap(), CliCommand::Interactive);
        assert_eq!(command(&["dev"]).unwrap(), CliCommand::Dev);
        assert_eq!(command(&["--help"]).unwrap(), CliCommand::Help);
    }

    #[test]
    fn parses_one_shot_output_and_prompt() {
        assert_eq!(
            command(&["run", "summarize", "results"]).unwrap(),
            CliCommand::Run {
                prompt: "summarize results".into(),
                output: OutputFormat::Console,
            }
        );
        assert_eq!(
            command(&["run", "--output=jsonl", "inspect", "--", "-data"]).unwrap(),
            CliCommand::Run {
                prompt: "inspect -data".into(),
                output: OutputFormat::Jsonl,
            }
        );
        assert_eq!(
            command(&["run", "--output", "jsonl", "inspect"]).unwrap(),
            CliCommand::Run {
                prompt: "inspect".into(),
                output: OutputFormat::Jsonl,
            }
        );
    }

    #[test]
    fn rejects_invalid_one_shot_arguments() {
        assert!(command(&["run"])
            .unwrap_err()
            .to_string()
            .contains("prompt"));
        assert!(command(&["run", "--output"])
            .unwrap_err()
            .to_string()
            .contains("value"));
        assert!(command(&["run", "--output", "xml", "inspect"])
            .unwrap_err()
            .to_string()
            .contains("unknown output format"));
        assert!(command(&["run", "--wat", "inspect"])
            .unwrap_err()
            .to_string()
            .contains("unknown run option"));
    }

    #[test]
    fn parses_eval_paths_and_rejects_duplicate_options() {
        assert_eq!(
            command(&[
                "eval",
                "--save",
                "current.json",
                "--compare",
                "baseline.json"
            ])
            .unwrap(),
            CliCommand::Eval {
                save: Some(PathBuf::from("current.json")),
                compare: Some(PathBuf::from("baseline.json")),
            }
        );
        assert!(command(&["eval", "--save"])
            .unwrap_err()
            .to_string()
            .contains("requires a path"));
        assert!(command(&["eval", "--save", "one", "--save", "two"])
            .unwrap_err()
            .to_string()
            .contains("only be specified once"));
    }

    #[test]
    fn jsonl_output_emits_one_valid_object_per_line() {
        let output = JsonlOutput::new(Vec::new());
        output.start("inspect", "test-model", std::path::Path::new("project"));
        output.assistant_text("hello\nworld");
        output.tool_call("read", "README.md");
        output.tool_result("read", true, "contents", 12);
        output.usage(2, 10, 20, 3, 4, 30, 100);
        assert!(!output.confirm("delete file?"));
        output.done();

        let bytes = output.into_inner();
        let events: Vec<serde_json::Value> = String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();

        assert_eq!(events.len(), 7);
        assert_eq!(events[0]["type"], "start");
        assert_eq!(events[1]["delta"], "hello\nworld");
        assert_eq!(events[3]["duration_ms"], 12);
        assert_eq!(events[4]["input_tokens"], 10);
        assert_eq!(events[5]["approved"], false);
        assert_eq!(events[6], serde_json::json!({"type": "done", "ok": true}));
    }
}
