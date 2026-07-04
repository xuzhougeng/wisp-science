use anyhow::Result;
use std::io::{Write, IsTerminal};
use std::path::PathBuf;
use std::sync::Arc;
use wisp_core::{Agent, MemoryManager, Output};
use wisp_llm::ProviderConfig;
use wisp_skills::SkillIndex;

const HELP: &str = "Built-in commands:\n  /q, /quit       Quit\n  /n, /new        Start a new session (old session is backed up)\n  /c, /compact    Manually trigger a full context compact\n  /h, /help       Show this help";

struct CliOutput;
impl CliOutput {
    fn dim(&self) -> &'static str { if std::io::stdout().is_terminal() { "\x1b[2m" } else { "" } }
    fn bold(&self) -> &'static str { if std::io::stdout().is_terminal() { "\x1b[1m" } else { "" } }
    fn cyan(&self) -> &'static str { if std::io::stdout().is_terminal() { "\x1b[36m" } else { "" } }
    fn green(&self) -> &'static str { if std::io::stdout().is_terminal() { "\x1b[32m" } else { "" } }
    fn red(&self) -> &'static str { if std::io::stdout().is_terminal() { "\x1b[31m" } else { "" } }
    fn yellow(&self) -> &'static str { if std::io::stdout().is_terminal() { "\x1b[33m" } else { "" } }
    fn reset(&self) -> &'static str { if std::io::stdout().is_terminal() { "\x1b[0m" } else { "" } }
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
        println!("\n{}{} {}{} {}{}{}", self.cyan(), "›", name, self.reset(), self.dim(), preview, self.reset());
    }
    fn tool_result(&self, name: &str, ok: bool, content: &str) {
        let icon = if ok { "✓" } else { "✗" };
        let color = if ok { self.green() } else { self.red() };
        println!(" {}{}{} {}{}", color, icon, self.reset(), self.dim(), self.reset());
        // Truncate verbose tool results in the terminal.
        let lines: Vec<&str> = content.lines().collect();
        let show: Vec<&str> = lines.iter().take(20).copied().collect();
        for l in show {
            let l: String = l.chars().take(200).collect();
            println!(" {}⎿ {}{}", self.dim(), l, self.reset());
        }
        if lines.len() > 20 {
            println!(" {}⎿ ... and {} more lines{}", self.dim(), lines.len() - 20, self.reset());
        }
        let _ = name;
    }
    fn usage(&self, round: usize, input: u64, output: u64, ctx_tokens: usize, max_context: usize) {
        let pct = if max_context > 0 { (ctx_tokens * 100 / max_context).min(100) } else { 0 };
        let color = if pct < 50 { self.green() } else if pct < 70 { self.yellow() } else { self.red() };
        println!(
            "\n{}round {}: {}k in / {}k out | ctx: {}%{}{}",
            self.dim(), round, input / 1000, output / 1000, color, pct, self.reset()
        );
    }
    fn compaction(&self, before: usize, after: usize, strategy: &str) {
        println!("{}[compact {}] {} → {} (-{}){}", self.yellow(), strategy, before, after, before.saturating_sub(after), self.reset());
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

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

async fn wire_mcp(agent: &mut Agent, command: &str, args: &[String]) {
    match wisp_mcp::McpClient::launch(command, args).await {
        Ok(client) => register_mcp_tools(agent, std::sync::Arc::new(client), &format!("{command} {}", args.join(" "))).await,
        Err(e) => println!("mcp launch failed: {e}"),
    }
}

async fn register_mcp_tools(agent: &mut Agent, client: std::sync::Arc<wisp_mcp::McpClient>, label: &str) {
    match client.tools_list().await {
        Ok(tools) => {
            let n = tools.len();
            for t in tools {
                agent.add_tool(Box::new(wisp_mcp::McpTool::new(t, client.clone())));
            }
            println!("mcp wired: {n} tool(s) from '{label}'.");
        }
        Err(e) => println!("mcp tools_list failed: {e}"),
    }
}

fn provider_config() -> Result<ProviderConfig> {
    let kind = match env("WISP_PROVIDER", "openai").to_ascii_lowercase().as_str() {
        "anthropic" => "anthropic".to_string(),
        "openai_responses" | "openai-responses" | "responses" => "openai_responses".to_string(),
        _ => "openai".to_string(),
    };
    let api_key = env("WISP_API_KEY", "");
    let base_url = env("WISP_API_URL", match kind.as_str() {
        "anthropic" => "https://api.anthropic.com",
        "openai_responses" => "https://api.openai.com/v1",
        _ => "https://api.deepseek.com",
    });
    let model = env("WISP_MODEL", match kind.as_str() {
        "anthropic" => "claude-sonnet-5",
        "openai_responses" => "gpt-5.5",
        _ => "deepseek-v4-pro",
    });
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
    if let Some(b) = wisp_skills::bundled_dir() { paths.push(b); }
    paths.push(root.join(".wisp").join("skills"));
    if let Some(home) = dirs::home_dir() { paths.push(home.join(".wisp").join("skills")); }
    if let Ok(extra) = std::env::var("WISP_SKILLS_PATH") {
        for p in extra.split([':', ';']).filter(|s| !s.is_empty()) { paths.push(PathBuf::from(p)); }
    }
    paths
}

#[tokio::main]
async fn main() -> Result<()> {
    // `cargo run dev` passes "dev" as argv[1]; forward to the desktop shell.
    if std::env::args().nth(1).as_deref() == Some("dev") {
        let status = std::process::Command::new("cargo")
            .args(["tauri", "dev"])
            .status()?;
        std::process::exit(status.code().unwrap_or(1));
    }

    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("wisp=info".parse()?)).init();

    let root = std::env::current_dir()?;
    let cfg = provider_config()?;
    let max_context = env("WISP_MAX_CONTEXT", "1000000").parse::<usize>().unwrap_or(1_000_000);
    let max_iter = env("WISP_MAX_ITER", "100").parse::<usize>().unwrap_or(100);

    let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
    let memory = Arc::new(MemoryManager::new(&root));

    let mut agent = Agent::new(cfg, skills.clone(), memory.clone(), root.clone(), max_context, max_iter, true);
    agent.seed_system_prompt(&skills, None);

    // Provision a uv venv once; shared by the Python REPL and the bundled
    // bio-tools MCP server. Skipped silently if uv isn't installed.
    let app_data = root.join(".wisp");
    let py_env = wisp_python::PythonEnv::ensure(&app_data).ok();

    // Python REPL: needs a kernel_worker path. Default to the bundled worker.
    let worker = std::env::var("WISP_KERNEL_WORKER")
        .ok()
        .or_else(|| wisp_python::bundled_worker_path().map(|p| p.to_string_lossy().to_string()))
        .unwrap_or_default();
    let worker_path = wisp_python::resolve_bundled_script(&worker);
    if worker_path.is_file() {
        if let Some(env) = &py_env {
            match wisp_python::KernelClient::spawn(&env.python(), &worker_path) {
                Ok(client) => {
                    agent.add_tool(Box::new(wisp_python::ReplTool::new(client)));
                    println!("python repl wired ({worker}).");
                }
                Err(e) => println!("python repl skipped: {e}"),
            }
        } else {
            println!("python repl skipped: uv venv unavailable");
        }
    } else {
        println!("(kernel worker not found at {worker}; set WISP_KERNEL_WORKER=<path>)");
    }

    // MCP server: WISP_MCP_COMMAND overrides; otherwise WISP_MCP_PKG launches
    // the bundled bio-tools server (<pkg> e.g. mcp_pubmed) via the venv python.
    if let Ok(cmdline) = std::env::var("WISP_MCP_COMMAND") {
        let parts: Vec<String> = cmdline
            .split_whitespace()
            .map(|s| {
                if s.ends_with(".py") {
                    wisp_python::resolve_bundled_script(s).to_string_lossy().to_string()
                } else {
                    s.to_string()
                }
            })
            .collect();
        if parts.len() >= 2 {
            let args: Vec<String> = parts[1..].to_vec();
            wire_mcp(&mut agent, &parts[0], &args).await;
        }
    } else if let Some(env) = &py_env {
        let pkg = std::env::var("WISP_MCP_PKG").unwrap_or_else(|_| "mcp_bio".into());
        match wisp_mcp::McpClient::launch_bio_tools(&env.python(), &pkg).await {
            Ok(client) => register_mcp_tools(&mut agent, std::sync::Arc::new(client), &format!("bio-tools:{pkg}")).await,
            Err(e) => println!("mcp bio-tools:{pkg} launch failed: {e}"),
        }
    }

    let out = CliOutput;
    println!("{}wisp-science{} | {} | {}", out.bold(), out.reset(), agent.provider.model(), root.display());
    if skills.is_empty() {
        println!("{}(no skills loaded; set WISP_SKILLS_PATH to a SKILL.md catalog){}", out.dim(), out.reset());
    }
    println!("{HELP}\n");

    let stdin = std::io::stdin();
    loop {
        print!("{}❯{} ", out.bold(), out.reset());
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if stdin.read_line(&mut line).unwrap_or(0) == 0 { break; }
        let input = line.trim().to_string();
        if input.is_empty() { continue; }
        match input.as_str() {
            "/q" | "/quit" => break,
            "/h" | "/help" => { println!("{HELP}"); continue; }
            "/c" | "/compact" => {
                println!("{}manual full compact not yet wired in CLI — use /n for a fresh session.{}", out.dim(), out.reset());
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

        let stamped = format!("{}, Current date: {}", input, chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
        if let Err(e) = agent.run(&stamped, &out, None).await {
            eprintln!("{}Error: {}{}", out.red(), e, out.reset());
        }
        println!();
        agent.ctx.clear_runtime_injections();
        agent.save();
    }
    Ok(())
}
