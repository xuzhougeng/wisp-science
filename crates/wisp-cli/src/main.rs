mod eval;
mod rpc;
mod runs;

use anyhow::{bail, Context, Result};
use std::collections::VecDeque;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use wisp_core::{Agent, MemoryManager, Output};
use wisp_llm::{Message, ProviderConfig, ToolCall};
use wisp_skills::SkillIndex;

const HELP: &str = "Built-in commands:\n  /q, /quit       Quit\n  /n, /new        Start a new session (old session is backed up)\n  /c, /compact    Compact the context (full history is archived to .wisp/history/ first)\n  /h, /help       Show this help";
const EVENT_SCHEMA: &str = "wisp.agent-event.v1";
const USAGE: &str = "Usage:
  wisp-science
  wisp-science run [--output console|jsonl] <prompt>
  wisp-science rpc
  wisp-science eval [--mode offline|live] [--suite suite.yaml] [options]
  wisp-science dev

Eval defaults to the built-in deterministic offline suite and requires no API key.
Use --mode live for a real configured model. Common options:
  --case ID                 Run one case (repeatable)
  --tag TAG                 Require a case tag (repeatable)
  --model MODEL             Live model to benchmark (repeatable)
  --repeat N                Repeat every selected case
  --parallel N              Maximum concurrent cases
  --timeout-ms N            Per-case wall-time timeout
  --artifacts DIR           Write trajectory JSONL files
  --keep-failed-workspace   Preserve failed workspaces under --artifacts
  --save REPORT             Save the JSON report
  --compare BASELINE        Compare against an eval v1 report
  --min-pass-rate PERCENT   Required attempt pass rate (default 100)

With no command, wisp-science starts the interactive terminal.";

#[derive(Debug, PartialEq, Eq)]
enum CliCommand {
    Interactive,
    Run {
        prompt: String,
        output: OutputFormat,
    },
    Eval(eval::EvalOptions),
    Rpc,
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
    let mut args = args.into_iter().collect::<Vec<_>>().into_iter();
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
        "rpc" => {
            if args.next().is_some() {
                bail!("rpc does not accept arguments");
            }
            Ok(CliCommand::Rpc)
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
            let mut options = eval::EvalOptions::default();
            while let Some(arg) = args.next() {
                let value = |args: &mut std::vec::IntoIter<String>| -> Result<String> {
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("{arg} requires a value"))
                };
                match arg.as_str() {
                    "--mode" => options.mode = eval::EvalMode::parse(&value(&mut args)?)?,
                    "--suite" => replace_path(&mut options.suite, &arg, value(&mut args)?)?,
                    "--save" => replace_path(&mut options.save, &arg, value(&mut args)?)?,
                    "--compare" => replace_path(&mut options.compare, &arg, value(&mut args)?)?,
                    "--artifacts" => replace_path(&mut options.artifacts, &arg, value(&mut args)?)?,
                    "--case" => options.cases.push(value(&mut args)?),
                    "--tag" => options.tags.push(value(&mut args)?),
                    "--model" => options.models.push(value(&mut args)?),
                    "--repeat" => options.repeat = parse_usize(&arg, &value(&mut args)?)?,
                    "--parallel" => options.parallel = parse_usize(&arg, &value(&mut args)?)?,
                    "--timeout-ms" => {
                        options.timeout_ms = Some(parse_u64(&arg, &value(&mut args)?)?)
                    }
                    "--max-tool-calls" => {
                        options.max_tool_calls = Some(parse_u64(&arg, &value(&mut args)?)?)
                    }
                    "--max-input-tokens" => {
                        options.max_input_tokens = Some(parse_u64(&arg, &value(&mut args)?)?)
                    }
                    "--max-duration-ms" => {
                        options.max_duration_ms = Some(parse_u64(&arg, &value(&mut args)?)?)
                    }
                    "--max-cost-microusd" => {
                        options.max_cost_microusd = Some(parse_u64(&arg, &value(&mut args)?)?)
                    }
                    "--input-cost-microusd-per-million" => {
                        options.input_cost_microusd_per_million =
                            parse_u64(&arg, &value(&mut args)?)?
                    }
                    "--output-cost-microusd-per-million" => {
                        options.output_cost_microusd_per_million =
                            parse_u64(&arg, &value(&mut args)?)?
                    }
                    "--reasoning-cost-microusd-per-million" => {
                        options.reasoning_cost_microusd_per_million =
                            parse_u64(&arg, &value(&mut args)?)?
                    }
                    "--max-token-regression-percent" => {
                        options.max_token_regression_percent =
                            Some(parse_percent(&arg, &value(&mut args)?)?)
                    }
                    "--max-round-regression" => {
                        options.max_round_regression = Some(parse_u64(&arg, &value(&mut args)?)?)
                    }
                    "--min-pass-rate" => {
                        options.min_pass_rate_percent = parse_percent(&arg, &value(&mut args)?)?
                    }
                    "--keep-failed-workspace" => options.keep_failed_workspace = true,
                    "--allow-regressions" => options.allow_regressions = true,
                    _ => bail!("unknown eval option '{arg}'"),
                }
            }
            Ok(CliCommand::Eval(options))
        }
        _ => bail!("unknown command '{command}'\n\n{USAGE}"),
    }
}

fn replace_path(target: &mut Option<PathBuf>, option: &str, value: String) -> Result<()> {
    if target.replace(PathBuf::from(value)).is_some() {
        bail!("{option} may only be specified once");
    }
    Ok(())
}

fn parse_u64(option: &str, value: &str) -> Result<u64> {
    value
        .parse()
        .with_context(|| format!("{option} requires a non-negative integer"))
}

fn parse_usize(option: &str, value: &str) -> Result<usize> {
    value
        .parse()
        .with_context(|| format!("{option} requires a non-negative integer"))
}

fn parse_percent(option: &str, value: &str) -> Result<u64> {
    parse_u64(option, value.trim_end_matches('%'))
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
        _context_usage: wisp_core::ContextUsage,
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

    fn compaction_started(&self, strategy: &str) {
        println!(
            "{}[compact {strategy}] preparing checkpoint...{}",
            self.yellow(),
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
    sequence: AtomicU64,
    session_id: String,
    turn_id: String,
    pending_calls: Mutex<VecDeque<ToolCall>>,
    active_call_ids: Mutex<VecDeque<String>>,
}

impl<W: Write + Send> JsonlOutput<W> {
    fn new(writer: W) -> Self {
        Self {
            writer: Mutex::new(writer),
            sequence: AtomicU64::new(0),
            session_id: uuid::Uuid::new_v4().to_string(),
            turn_id: uuid::Uuid::new_v4().to_string(),
            pending_calls: Mutex::new(VecDeque::new()),
            active_call_ids: Mutex::new(VecDeque::new()),
        }
    }

    fn emit(&self, mut event: serde_json::Value) {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        if let Some(object) = event.as_object_mut() {
            object.insert("schema".into(), EVENT_SCHEMA.into());
            object.insert("sequence".into(), sequence.into());
            object.insert("session_id".into(), self.session_id.clone().into());
            object.insert("turn_id".into(), self.turn_id.clone().into());
        }
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
        let call = self.pending_calls.lock().ok().and_then(|mut calls| {
            calls
                .iter()
                .position(|call| call.function.name == name)
                .and_then(|index| calls.remove(index))
        });
        let (call_id, arguments) = call
            .map(|call| {
                let arguments = call.args_value();
                (Some(call.id), Some(arguments))
            })
            .unwrap_or_default();
        if let Some(call_id) = &call_id {
            if let Ok(mut active) = self.active_call_ids.lock() {
                active.push_back(call_id.clone());
            }
        }
        self.emit(serde_json::json!({
            "type": "tool_call",
            "call_id": call_id,
            "name": name,
            "arguments": arguments,
            "preview": preview,
        }));
    }

    fn tool_result(&self, name: &str, ok: bool, content: &str, duration_ms: u64) {
        let call_id = self
            .active_call_ids
            .lock()
            .ok()
            .and_then(|mut ids| ids.pop_front());
        self.emit(serde_json::json!({
            "type": "tool_result",
            "call_id": call_id,
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
        _context_usage: wisp_core::ContextUsage,
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

    fn compaction_started(&self, strategy: &str) {
        self.emit(serde_json::json!({
            "type": "compaction_started",
            "strategy": strategy,
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

    fn tool_presentation(
        &self,
        kind: &str,
        payload: &serde_json::Value,
        _server: Option<std::sync::Arc<dyn wisp_tools::McpAppServer>>,
    ) {
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

    fn on_message(&self, message: &Message) {
        if !message.tool_calls.is_empty() {
            if let Ok(mut calls) = self.pending_calls.lock() {
                calls.extend(message.tool_calls.iter().cloned());
            }
        }
        self.emit(serde_json::json!({
            "type": "message",
            "role": message.role,
            "content": message.content,
            "reasoning": message.reasoning,
            "tool_call_id": message.tool_call_id,
            "tool_name": message.tool_name,
            "tool_calls": message.tool_calls,
        }));
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

fn parse_provider_kind(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "anthropic" => "anthropic".into(),
        "openai_responses" | "openai-responses" | "responses" => "openai_responses".into(),
        _ => "openai".into(),
    }
}

fn default_provider_url(kind: &str) -> &'static str {
    match kind {
        "anthropic" => "https://api.anthropic.com",
        "openai_responses" => "https://api.openai.com/v1",
        _ => "https://api.deepseek.com",
    }
}

fn default_provider_model(kind: &str) -> &'static str {
    match kind {
        "anthropic" => "claude-sonnet-5",
        "openai_responses" => "gpt-5.5",
        _ => "deepseek-v4-flash",
    }
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn build_named_provider_config(
    kind: &str,
    base_url: String,
    api_key: String,
    model: String,
) -> ProviderConfig {
    match kind {
        "anthropic" => ProviderConfig::anthropic(base_url, api_key, model),
        "openai_responses" => ProviderConfig::openai_responses(base_url, api_key, model),
        _ => ProviderConfig::openai(base_url, api_key, model),
    }
}

fn provider_config() -> Result<ProviderConfig> {
    let kind = parse_provider_kind(&env("WISP_PROVIDER", "openai"));
    let api_key = env("WISP_API_KEY", "");
    let base_url = env("WISP_API_URL", default_provider_url(&kind));
    let model = env("WISP_MODEL", default_provider_model(&kind));
    if api_key.is_empty() {
        anyhow::bail!("WISP_API_KEY is not set (required). Set it to your provider API key.");
    }
    let mut cfg = build_named_provider_config(&kind, base_url, api_key, model);
    cfg.max_tokens = parse_wisp_max_tokens(std::env::var("WISP_MAX_TOKENS").ok().as_deref())
        .unwrap_or(DEFAULT_HEADLESS_MAX_TOKENS);
    if let Some(effort) =
        parse_wisp_reasoning_effort(std::env::var("WISP_REASONING_EFFORT").ok().as_deref())
    {
        cfg.reasoning_effort = Some(effort);
    }
    Ok(cfg)
}

/// Dedicated image-analysis model used when the primary chat model cannot see.
/// `WISP_VISION_MODEL` is the gate; provider/url/key fall back to the primary
/// `WISP_*` values when omitted.
fn vision_provider_from_values(
    model: Option<&str>,
    provider: Option<&str>,
    api_url: Option<&str>,
    api_key: Option<&str>,
    fallback_provider: Option<&str>,
    fallback_url: Option<&str>,
    fallback_key: Option<&str>,
) -> Result<Option<ProviderConfig>> {
    let model = model.map(str::trim).filter(|value| !value.is_empty());
    let provider = provider.map(str::trim).filter(|value| !value.is_empty());
    let api_url = api_url.map(str::trim).filter(|value| !value.is_empty());
    if model.is_none() && provider.is_none() && api_url.is_none() {
        return Ok(None);
    }
    let Some(model) = model else {
        bail!(
            "WISP_VISION_MODEL is required when WISP_VISION_PROVIDER or WISP_VISION_API_URL is set"
        );
    };
    let kind = parse_provider_kind(provider.unwrap_or(fallback_provider.unwrap_or("openai")));
    let base_url = api_url
        .map(str::to_string)
        .or_else(|| provider.map(|_| default_provider_url(&kind).to_string()))
        .or_else(|| fallback_url.map(str::to_string))
        .unwrap_or_else(|| default_provider_url(&kind).to_string());
    let api_key = api_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            fallback_key
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("");
    if api_key.is_empty() {
        bail!("WISP_VISION_API_KEY or WISP_API_KEY is required for the dedicated vision model");
    }
    Ok(Some(build_named_provider_config(
        &kind,
        base_url,
        api_key.to_string(),
        model.to_string(),
    )))
}

fn vision_provider_config() -> Result<Option<ProviderConfig>> {
    vision_provider_from_values(
        env_opt("WISP_VISION_MODEL").as_deref(),
        env_opt("WISP_VISION_PROVIDER").as_deref(),
        env_opt("WISP_VISION_API_URL").as_deref(),
        env_opt("WISP_VISION_API_KEY").as_deref(),
        env_opt("WISP_PROVIDER").as_deref(),
        env_opt("WISP_API_URL").as_deref(),
        env_opt("WISP_API_KEY").as_deref(),
    )
}

const DEFAULT_HEADLESS_MAX_TOKENS: u64 = 32_768;
const DEFAULT_HEADLESS_MAX_ITER: usize = 0;
const DEFAULT_HEADLESS_VISION: bool = false;

fn parse_wisp_max_tokens(raw: Option<&str>) -> Option<u64> {
    raw.and_then(|value| value.parse().ok())
}

fn parse_wisp_max_iter(raw: Option<&str>) -> Option<usize> {
    raw.and_then(|value| value.parse().ok())
}

fn parse_wisp_vision(raw: Option<&str>) -> Option<bool> {
    let value = raw?.trim().to_ascii_lowercase();
    match value.as_str() {
        "1" | "true" | "on" | "yes" | "y" => Some(true),
        "0" | "false" | "off" | "no" | "n" => Some(false),
        _ => None,
    }
}

fn parse_wisp_reasoning_effort(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    if value.is_empty() {
        return None;
    }
    let value = value.to_ascii_lowercase();
    matches!(
        value.as_str(),
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
    )
    .then_some(value)
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
    result.map(|_| ())
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
        CliCommand::Rpc
            | CliCommand::Run {
                output: OutputFormat::Jsonl,
                ..
            }
    );

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("wisp=info".parse()?),
        )
        .init();

    if let CliCommand::Eval(options) = &command {
        let live_config = if options.mode == eval::EvalMode::Live {
            Some(provider_config()?)
        } else {
            None
        };
        let live_vision = if options.mode == eval::EvalMode::Live {
            vision_provider_config()?
        } else {
            None
        };
        return eval::run(live_config, live_vision, options).await;
    }
    let cfg = match provider_config() {
        Ok(cfg) => cfg,
        Err(error) => {
            if command == CliCommand::Rpc {
                rpc::startup_error(&error);
            } else if jsonl {
                JsonlOutput::new(std::io::stdout()).error(&error);
            }
            return Err(error);
        }
    };
    let root = match std::env::current_dir() {
        Ok(root) => root,
        Err(error) => {
            let error = anyhow::Error::from(error);
            if command == CliCommand::Rpc {
                rpc::startup_error(&error);
            } else if jsonl {
                JsonlOutput::new(std::io::stdout()).error(&error);
            }
            return Err(error);
        }
    };
    let max_context = env("WISP_MAX_CONTEXT", "1000000")
        .parse::<usize>()
        .unwrap_or(1_000_000);
    let max_iter = parse_wisp_max_iter(std::env::var("WISP_MAX_ITER").ok().as_deref())
        .unwrap_or(DEFAULT_HEADLESS_MAX_ITER);

    let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
    let memory = Arc::new(MemoryManager::new(&root));

    let vision_cfg = match vision_provider_config() {
        Ok(cfg) => cfg,
        Err(error) => {
            if command == CliCommand::Rpc {
                rpc::startup_error(&error);
            } else if jsonl {
                JsonlOutput::new(std::io::stdout()).error(&error);
            }
            return Err(error);
        }
    };
    let mut agent = Agent::new(
        cfg,
        skills.clone(),
        memory.clone(),
        root.clone(),
        max_context,
        max_iter,
        true,
        vision_cfg.clone(),
    );
    agent.ctx.supports_vision = parse_wisp_vision(std::env::var("WISP_VISION").ok().as_deref())
        .unwrap_or(DEFAULT_HEADLESS_VISION);
    if let Some(vision) = &vision_cfg {
        setup_message(jsonl, format_args!("vision model wired ({})", vision.model));
    }
    let run_store = wisp_runs::open_project_store(&root, runs::CLI_PROJECT_ID, "CLI").await?;
    let run_manager = wisp_runs::RunManager::new();
    runs::register_run_tools(
        &mut agent.tools,
        run_store.clone(),
        run_manager.clone(),
        runs::CLI_PROJECT_ID,
    );
    let compute = wisp_runs::cli_compute_section(&run_store).await;
    agent.seed_system_prompt(&skills, Some(compute));

    // Provision the Python REPL environment. Native bio tools do not use it.
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

    let bio_package = std::env::var("WISP_MCP_PKG").unwrap_or_else(|_| "mcp_bio".into());
    let native_bio =
        std::env::var("WISP_MCP_COMMAND").is_err() && wisp_bio::selected_by_package(&bio_package);
    if native_bio {
        let credentials: Vec<_> = std::env::vars()
            .filter(|(key, value)| {
                !value.is_empty()
                    && (key.ends_with("_API_KEY")
                        || key.ends_with("_EMAIL")
                        || key.ends_with("_TOKEN")
                        || key == "NCBI_EMAIL")
            })
            .collect();
        match wisp_bio::NativeBio::new(&credentials) {
            Ok(client) => {
                for tool in wisp_bio::tools_for_package(std::sync::Arc::new(client), &bio_package) {
                    agent.add_tool(tool);
                }
            }
            Err(error) => setup_message(jsonl, format_args!("native bio: {error}")),
        }
    }

    // Explicit stdio override; WISP_MCP_PKG selects native tools above.
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
    } else if !wisp_bio::selected_by_package(&bio_package) {
        setup_message(
            jsonl,
            format_args!("Unknown native bio package: {bio_package}"),
        );
    }

    let out = CliOutput;
    if command == CliCommand::Rpc {
        let result = rpc::serve(agent).await;
        runtime_manager.shutdown_all().await;
        return result;
    }
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
                let compute = wisp_runs::cli_compute_section(&run_store).await;
                agent.seed_system_prompt(&skills, Some(compute));
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
        assert_eq!(command(&["rpc"]).unwrap(), CliCommand::Rpc);
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
        let mut expected = eval::EvalOptions::default();
        expected.save = Some(PathBuf::from("current.json"));
        expected.compare = Some(PathBuf::from("baseline.json"));
        assert_eq!(
            command(&[
                "eval",
                "--save",
                "current.json",
                "--compare",
                "baseline.json"
            ])
            .unwrap(),
            CliCommand::Eval(expected)
        );
        assert!(command(&["eval", "--save"])
            .unwrap_err()
            .to_string()
            .contains("requires a value"));
        assert!(command(&["eval", "--save", "one", "--save", "two"])
            .unwrap_err()
            .to_string()
            .contains("only be specified once"));
    }

    #[test]
    fn parses_live_model_matrix() {
        let mut expected = eval::EvalOptions::default();
        expected.mode = eval::EvalMode::Live;
        expected.models = vec!["model-a".into(), "model-b".into()];
        assert_eq!(
            command(&["eval", "--mode", "live", "--model", "model-a", "--model", "model-b"])
                .unwrap(),
            CliCommand::Eval(expected)
        );
    }

    #[test]
    fn jsonl_output_emits_one_valid_object_per_line() {
        let output = JsonlOutput::new(Vec::new());
        output.start("inspect", "test-model", std::path::Path::new("project"));
        output.assistant_text("hello\nworld");
        output.tool_call("read", "README.md");
        output.tool_result("read", true, "contents", 12);
        output.usage(2, 10, 20, 3, 4, 30, 100, wisp_core::ContextUsage::default());
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
        assert_eq!(events[6]["type"], "done");
        assert_eq!(events[6]["ok"], true);
        for (sequence, event) in events.iter().enumerate() {
            assert_eq!(event["schema"], EVENT_SCHEMA);
            assert_eq!(event["sequence"], sequence);
            assert!(event["session_id"].is_string());
            assert!(event["turn_id"].is_string());
        }
    }

    #[test]
    fn jsonl_output_correlates_full_tool_calls_and_results() {
        let output = JsonlOutput::new(Vec::new());
        let mut message = Message::assistant("");
        message.tool_calls.push(ToolCall {
            id: "call-42".into(),
            kind: "function".into(),
            function: wisp_llm::FunctionCall {
                name: "read".into(),
                arguments: serde_json::json!({"path": "notes.txt"}).to_string(),
            },
        });
        output.on_message(&message);
        output.tool_call("read", "notes.txt");
        output.tool_result("read", true, "contents", 3);

        let events: Vec<serde_json::Value> = String::from_utf8(output.into_inner())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events[1]["call_id"], "call-42");
        assert_eq!(events[1]["arguments"]["path"], "notes.txt");
        assert_eq!(events[2]["call_id"], "call-42");
    }

    #[test]
    fn wisp_max_tokens_override_parses_set_unset_and_invalid() {
        assert_eq!(parse_wisp_max_tokens(None), None);
        assert_eq!(
            parse_wisp_max_tokens(None).unwrap_or(DEFAULT_HEADLESS_MAX_TOKENS),
            32_768
        );
        assert_eq!(parse_wisp_max_tokens(Some("65536")), Some(65536));
        assert_eq!(parse_wisp_max_tokens(Some("nope")), None);
        assert_eq!(
            parse_wisp_max_tokens(Some("nope")).unwrap_or(DEFAULT_HEADLESS_MAX_TOKENS),
            32_768
        );
        assert_eq!(parse_wisp_max_tokens(Some("")), None);
    }

    #[test]
    fn wisp_max_iter_override_parses_set_unset_and_invalid() {
        assert_eq!(parse_wisp_max_iter(None), None);
        assert_eq!(
            parse_wisp_max_iter(None).unwrap_or(DEFAULT_HEADLESS_MAX_ITER),
            0
        );
        assert_eq!(parse_wisp_max_iter(Some("50")), Some(50));
        assert_eq!(parse_wisp_max_iter(Some("0")), Some(0));
        assert_eq!(parse_wisp_max_iter(Some("nope")), None);
        assert_eq!(
            parse_wisp_max_iter(Some("nope")).unwrap_or(DEFAULT_HEADLESS_MAX_ITER),
            0
        );
    }

    #[test]
    fn wisp_vision_override_parses_set_unset_and_invalid() {
        assert_eq!(parse_wisp_vision(None), None);
        assert_eq!(
            parse_wisp_vision(None).unwrap_or(DEFAULT_HEADLESS_VISION),
            false
        );
        assert_eq!(parse_wisp_vision(Some("0")), Some(false));
        assert_eq!(parse_wisp_vision(Some("1")), Some(true));
        assert_eq!(parse_wisp_vision(Some("TRUE")), Some(true));
        assert_eq!(parse_wisp_vision(Some("off")), Some(false));
        assert_eq!(parse_wisp_vision(Some("nope")), None);
        assert_eq!(
            parse_wisp_vision(Some("nope")).unwrap_or(DEFAULT_HEADLESS_VISION),
            false
        );
    }

    #[test]
    fn dedicated_vision_model_is_optional_until_any_vision_env_is_set() {
        assert!(
            vision_provider_from_values(None, None, None, None, None, None, None)
                .unwrap()
                .is_none()
        );
        let err =
            vision_provider_from_values(None, Some("openai"), None, Some("sk"), None, None, None)
                .unwrap_err()
                .to_string();
        assert!(err.contains("WISP_VISION_MODEL"), "{err}");
    }

    #[test]
    fn dedicated_vision_model_falls_back_to_primary_endpoint() {
        let cfg = vision_provider_from_values(
            Some("qwen-vl"),
            None,
            None,
            None,
            Some("openai"),
            Some("https://api.example.com/v1"),
            Some("primary-key"),
        )
        .unwrap()
        .expect("vision config");
        assert_eq!(cfg.model, "qwen-vl");
        assert_eq!(cfg.base_url, "https://api.example.com/v1");
        assert_eq!(cfg.api_key, "primary-key");
    }

    #[test]
    fn dedicated_vision_model_uses_its_own_provider_url_and_key() {
        let cfg = vision_provider_from_values(
            Some("gpt-4o-mini"),
            Some("openai_responses"),
            Some("https://api.openai.com/v1"),
            Some("vision-key"),
            Some("anthropic"),
            Some("https://api.anthropic.com"),
            Some("primary-key"),
        )
        .unwrap()
        .expect("vision config");
        assert_eq!(cfg.model, "gpt-4o-mini");
        assert_eq!(cfg.base_url, "https://api.openai.com/v1");
        assert_eq!(cfg.api_key, "vision-key");
        assert!(matches!(cfg.kind, wisp_llm::ProviderKind::OpenAiResponses));
    }

    #[test]
    fn dedicated_vision_provider_without_url_uses_that_provider_default() {
        let cfg = vision_provider_from_values(
            Some("claude-sonnet-5"),
            Some("anthropic"),
            None,
            Some("sk"),
            Some("openai"),
            Some("https://api.deepseek.com"),
            Some("primary-key"),
        )
        .unwrap()
        .expect("vision config");
        assert_eq!(cfg.base_url, "https://api.anthropic.com");
        assert!(matches!(cfg.kind, wisp_llm::ProviderKind::Anthropic));
    }

    #[test]
    fn dedicated_vision_model_requires_an_api_key() {
        let err = vision_provider_from_values(
            Some("qwen-vl"),
            Some("openai"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("WISP_VISION_API_KEY"), "{err}");
    }

    #[test]
    fn wisp_reasoning_effort_override_parses_set_unset_and_invalid() {
        assert_eq!(parse_wisp_reasoning_effort(None), None);
        assert_eq!(
            parse_wisp_reasoning_effort(Some("high")),
            Some("high".into())
        );
        assert_eq!(parse_wisp_reasoning_effort(Some("MAX")), Some("max".into()));
        assert_eq!(parse_wisp_reasoning_effort(Some("nope")), None);
        assert_eq!(parse_wisp_reasoning_effort(Some("")), None);
        assert_eq!(parse_wisp_reasoning_effort(Some("  ")), None);
    }
}
