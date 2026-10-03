//! `desktop_autopilot`: Cua Driver steered by TypeSafe Jev (System One),
//! handing the window back to the calling agent (System Two) whenever Jev
//! cannot settle a step.
//!
//! Hosts register it only when a TypeSafe key is configured and an MCP server
//! advertises Cua Driver's `get_window_state` plus an `element_token` click;
//! without a key the agent drives Cua Driver itself (the `computer-use`
//! skill). Each step observes the window's accessibility tree, offers its
//! labelled native controls as candidates, lets Jev pick one, and clicks it by
//! `element_token`. Jev only picks an ID from the table built here; it never
//! invents a target. Text entry, risky labels, an irreversible verdict, low
//! confidence, clicks with no visible effect, and every Driver or Jev error
//! stop the loop and return the reason, so the agent reasons about the window
//! and may resume with a `hint`. Candidate rules follow Cua's jev-use
//! reference (`libs/cua-driver/examples/jev-use`, `native.py`).

use crate::client::{McpClient, RemoteTool};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use wisp_llm::systemone::{self, Answer, NoulCriteria, Question};
use wisp_llm::{ProviderConfig, ToolSchema};
use wisp_tools::{Approval, Tool, ToolEnv, ToolEvent, ToolResult};

pub const NAME: &str = "desktop_autopilot";

// ponytail: one fixed policy from TypeSafe's guidance and the community
// runners (awlevin stops under 0.4, jev-use gates irreversible at 0.35).
// Pin a versioned model and measure on real windows before moving these.
const MIN_CONFIDENCE: f64 = 0.4;
const DONE_AT: f64 = 0.8;
const IRREVERSIBLE_AT: f64 = 0.35;
/// Cua measured Jev at up to 24 native candidates.
const MAX_CANDIDATES: usize = 24;
const DEFAULT_CLICKS: u64 = 12;
const MAX_CLICKS: u64 = 30;
const MAX_HISTORY: usize = 8;
const MAX_TEXT_LINES: usize = 60;
const MAX_LABEL_CHARS: usize = 120;
const HAND_BACK: &str = "hand_back";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Button,
    Toggle,
    Checkbox,
    Radio,
    Popup,
    MenuItem,
    Link,
    TextInput,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::Button => "button",
            Class::Toggle => "toggle",
            Class::Checkbox => "checkbox",
            Class::Radio => "radio",
            Class::Popup => "popup",
            Class::MenuItem => "menu_item",
            Class::Link => "link",
            Class::TextInput => "text_input",
        }
    }
}

/// Port of Driver's `normalized_role`: ASCII alphanumerics, lowercased,
/// without an `ax` prefix.
fn normalized_role(role: &str) -> String {
    let lower: String = role
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let role = lower.strip_prefix("ax").unwrap_or(&lower);
    match role {
        "pushbutton" => "button".into(),
        "pagetab" | "tabitem" => "tab".into(),
        _ => role.into(),
    }
}

/// Cua's reviewed raw-role table, per platform because the same normalized
/// name differs: AT-SPI `text` is editable, UIA `Text` is static. Unknown
/// roles are never candidates.
fn role_class(role: &str, os: &str) -> Option<Class> {
    use Class::*;
    let role = normalized_role(role);
    let class = match os {
        "macos" => match role.as_str() {
            "button" => Button,
            "switch" => Toggle,
            "checkbox" => Checkbox,
            "radiobutton" => Radio,
            "popupbutton" | "combobox" | "menubutton" => Popup,
            "menuitem" | "menubaritem" => MenuItem,
            "link" => Link,
            "textfield" | "textarea" | "searchfield" | "securetextfield" => TextInput,
            _ => return None,
        },
        "windows" => match role.as_str() {
            "button" | "splitbutton" => Button,
            "checkbox" => Checkbox,
            "radiobutton" => Radio,
            "combobox" => Popup,
            "menuitem" => MenuItem,
            "hyperlink" => Link,
            "edit" => TextInput,
            _ => return None,
        },
        _ => match role.as_str() {
            "button" => Button,
            "togglebutton" | "switch" => Toggle,
            "checkbox" => Checkbox,
            "radiobutton" => Radio,
            "combobox" => Popup,
            "menuitem" | "checkmenuitem" | "radiomenuitem" => MenuItem,
            "link" => Link,
            "entry" | "text" | "passwordtext" => TextInput,
            _ => return None,
        },
    };
    Some(class)
}

/// Static text Jev reads to judge progress; never a candidate.
fn is_static_text(role: &str, os: &str) -> bool {
    let role = normalized_role(role);
    match os {
        "macos" => role == "statictext",
        "windows" => role == "text",
        _ => matches!(role.as_str(), "label" | "static"),
    }
}

/// Whole-word label phrases: Cua's list plus Chinese. An accident guard, not
/// an authorization boundary; Jev's irreversibility check runs as well.
const RISK_PHRASES: &[&str] = &[
    "delete",
    "remove",
    "erase",
    "trash",
    "discard",
    "clear all",
    "format",
    "reset",
    "send",
    "submit",
    "post",
    "publish",
    "share",
    "reply",
    "buy",
    "purchase",
    "pay",
    "checkout",
    "order",
    "subscribe",
    "close",
    "quit",
    "exit",
    "don't save",
    "删除",
    "移除",
    "清空",
    "丢弃",
    "重置",
    "发送",
    "提交",
    "发布",
    "分享",
    "回复",
    "购买",
    "支付",
    "付款",
    "下单",
    "订阅",
    "关闭",
    "退出",
    "不保存",
];

fn risky(label: &str) -> bool {
    let text = label.to_lowercase().replace('’', "'");
    let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    RISK_PHRASES.iter().any(|phrase| {
        text.match_indices(phrase).any(|(at, _)| {
            !word(text[..at].chars().next_back()) && !word(text[at + phrase.len()..].chars().next())
        })
    })
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(text: String) -> String {
    if text.chars().count() <= MAX_LABEL_CHARS {
        text
    } else {
        text.chars().take(MAX_LABEL_CHARS).collect::<String>() + "…"
    }
}

fn slug(label: &str) -> String {
    let dashed: String = label
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                '-'
            }
        })
        .collect();
    let slug = dashed
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(32)
        .collect::<String>();
    if slug.is_empty() {
        "item".into()
    } else {
        slug
    }
}

fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// `(x, y, w, h)` from a Driver element frame or window bounds.
fn rect(value: Option<&Value>) -> Option<(f64, f64, f64, f64)> {
    let value = value?;
    let num = |keys: &[&str]| keys.iter().find_map(|key| value.get(*key)?.as_f64());
    Some((
        num(&["x"])?,
        num(&["y"])?,
        num(&["w", "width"])?,
        num(&["h", "height"])?,
    ))
}

fn on_screen(frame: Option<(f64, f64, f64, f64)>, window: Option<(f64, f64, f64, f64)>) -> bool {
    let (Some((x, y, w, h)), Some((wx, wy, ww, wh))) = (frame, window) else {
        return false;
    };
    w > 0.0 && h > 0.0 && (x + w).min(wx + ww) > x.max(wx) && (y + h).min(wy + wh) > y.max(wy)
}

/// Windows' title bar and its System/Minimize/Maximize/Close children belong
/// to the window manager, not the application.
fn in_window_chrome(element: &Value, by_index: &HashMap<u64, &Value>, os: &str) -> bool {
    if os != "windows" {
        return false;
    }
    let mut parent = element.get("parent_index").and_then(Value::as_u64);
    for _ in 0..by_index.len() {
        let Some(node) = parent.and_then(|index| by_index.get(&index)) else {
            return false;
        };
        if normalized_role(str_field(node, "role")) == "titlebar" {
            return true;
        }
        parent = node.get("parent_index").and_then(Value::as_u64);
    }
    false
}

#[derive(Debug, Clone)]
struct Candidate {
    id: String,
    class: Class,
    label: String,
    state: &'static str,
    token: String,
    risky: bool,
}

impl Candidate {
    fn description(&self) -> String {
        let label = &self.label;
        match self.class {
            Class::Button => format!("Press the button \"{label}\"."),
            Class::Toggle | Class::Checkbox => {
                format!("Toggle \"{label}\" (now {}).", self.state)
            }
            Class::Radio => format!("Select the option \"{label}\" (now {}).", self.state),
            Class::Popup => format!("Open the drop-down \"{label}\"."),
            Class::MenuItem => format!("Choose the menu item \"{label}\"."),
            Class::Link => format!("Follow the link \"{label}\"."),
            Class::TextInput => format!("Type into the field \"{label}\" (now {}).", self.state),
        }
    }
}

/// One `get_window_state` result reduced to what Jev may see: candidate
/// descriptions and states (never field values or tokens) plus static text.
#[derive(Debug)]
struct Observation {
    app: String,
    title: String,
    candidates: Vec<Candidate>,
    dropped: usize,
    text: Vec<String>,
    signature: u64,
}

fn observation(state: &Value, pid: u64, window_id: u64, os: &str) -> Result<Observation, String> {
    if state.get("pid").and_then(Value::as_u64) != Some(pid)
        || state.get("window_id").and_then(Value::as_u64) != Some(window_id)
    {
        return Err("get_window_state described a different window".into());
    }
    let elements = state
        .get("elements")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let by_index: HashMap<u64, &Value> = elements
        .iter()
        .filter_map(|e| Some((e.get("element_index")?.as_u64()?, e)))
        .collect();
    let window = rect(state.get("window_bounds"));
    let mut candidates = Vec::new();
    let mut text: Vec<String> = Vec::new();
    for element in elements {
        let role = str_field(element, "role");
        let label = collapse(str_field(element, "label"));
        let value = element.get("value").and_then(Value::as_str);
        if is_static_text(role, os) {
            let line = truncate(if label.is_empty() {
                collapse(value.unwrap_or_default())
            } else {
                label
            });
            if !line.is_empty() && text.len() < MAX_TEXT_LINES && !text.contains(&line) {
                text.push(line);
            }
            continue;
        }
        let Some(class) = role_class(role, os) else {
            continue;
        };
        let token = str_field(element, "element_token");
        if in_window_chrome(element, &by_index, os)
            || element.get("enabled") == Some(&Value::Bool(false))
            || !on_screen(rect(element.get("frame")), window)
            || label.is_empty()
            // A label that only echoes the value is typed content, not a name.
            || value.is_some_and(|v| collapse(v) == label)
            || element.get("in_web_content") == Some(&Value::Bool(true))
            || token.is_empty()
        {
            continue;
        }
        let selected = element.get("selected").and_then(Value::as_bool) == Some(true);
        let state = match class {
            Class::Toggle | Class::Checkbox if selected => "checked",
            Class::Toggle | Class::Checkbox => "unchecked",
            Class::Radio if selected => "selected",
            Class::Radio => "not selected",
            Class::TextInput if value.is_some_and(|v| !v.is_empty()) => "has text",
            Class::TextInput => "empty",
            _ => "enabled",
        };
        candidates.push(Candidate {
            id: String::new(),
            class,
            risky: risky(&label),
            label: truncate(label),
            state,
            token: token.into(),
        });
    }
    // ponytail: first 24 in tree order, like Cua's depth-first cap. Rank by
    // relevance to the goal if real windows lose the needed control here.
    let dropped = candidates.len().saturating_sub(MAX_CANDIDATES);
    candidates.truncate(MAX_CANDIDATES);
    let mut seen: HashMap<String, usize> = HashMap::new();
    for candidate in &mut candidates {
        let base = format!("{}:{}", candidate.class.name(), slug(&candidate.label));
        let n = seen.entry(base.clone()).or_default();
        *n += 1;
        candidate.id = if *n == 1 { base } else { format!("{base}:{n}") };
    }
    let mut hasher = DefaultHasher::new();
    for candidate in &candidates {
        (&candidate.id, candidate.state).hash(&mut hasher);
    }
    text.hash(&mut hasher);
    Ok(Observation {
        app: str_field(state, "app_name").into(),
        title: str_field(state, "window_title").into(),
        candidates,
        dropped,
        text,
        signature: hasher.finish(),
    })
}

#[derive(Debug, PartialEq)]
enum Move {
    Click(usize),
    Done(f64),
    HandBack(String),
}

fn top_choices(probabilities: &BTreeMap<String, f64>) -> String {
    let mut ranked: Vec<_> = probabilities.iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(a.1));
    let top: Vec<_> = ranked
        .iter()
        .take(3)
        .map(|(id, p)| format!("{id} {p:.2}"))
        .collect();
    format!(" (Jev's top choices: {})", top.join(", "))
}

/// Settle one step from Jev's answers. Every comparison fails closed: a NaN
/// probability or confidence hands back instead of acting.
fn settle(obs: &Observation, answers: &BTreeMap<String, Answer>) -> Result<Move, String> {
    let Some(Answer::Noul { noul: done }) = answers.get("goal_done") else {
        return Err("Jev omitted goal_done".into());
    };
    let Some(Answer::Choice {
        choice,
        probabilities,
        confidence,
    }) = answers.get("next_action")
    else {
        return Err("Jev omitted next_action".into());
    };
    if *done >= DONE_AT {
        return Ok(Move::Done(*done));
    }
    if choice == HAND_BACK {
        return Ok(Move::HandBack(format!(
            "jev_hand_back: Jev found no control that clearly advances the goal{}",
            top_choices(probabilities)
        )));
    }
    let index = obs
        .candidates
        .iter()
        .position(|c| &c.id == choice)
        .ok_or_else(|| format!("Jev chose unknown candidate {choice}"))?;
    if !(*confidence >= MIN_CONFIDENCE) {
        return Ok(Move::HandBack(format!(
            "low_confidence: {confidence:.2}{}",
            top_choices(probabilities)
        )));
    }
    let chosen = &obs.candidates[index];
    if chosen.class == Class::TextInput {
        return Ok(Move::HandBack(format!(
            "needs_text: the next step is typing into the field \"{}\"",
            chosen.label
        )));
    }
    if chosen.risky {
        return Ok(Move::HandBack(format!(
            "needs_confirmation: Jev chose \"{}\", whose label reads as delete/send/purchase/close",
            chosen.description()
        )));
    }
    Ok(Move::Click(index))
}

fn step_questions(obs: &Observation) -> BTreeMap<String, Question> {
    let mut criteria: BTreeMap<String, Option<String>> = obs
        .candidates
        .iter()
        .map(|c| (c.id.clone(), Some(c.description())))
        .collect();
    criteria.insert(
        HAND_BACK.into(),
        Some(
            "Stop and hand the window back: no other option clearly advances the goal, \
             the needed control is missing, or the step needs judgment."
                .into(),
        ),
    );
    BTreeMap::from([
        (
            "next_action".into(),
            Question::Choice {
                instructions: "Which single option best advances `goal` from the current \
                               window? Follow `hint` when present. Do not repeat an action that \
                               `history` says left the window unchanged."
                    .into(),
                criteria,
            },
        ),
        (
            "goal_done".into(),
            Question::Noul {
                instructions: "Does the current window, judged from `screen_text` and \
                               `controls`, already show that `goal` is achieved?"
                    .into(),
                criteria: Some(NoulCriteria {
                    yes: "The window itself shows the finished outcome".into(),
                    no: "The outcome is only likely, in progress, or still a step away".into(),
                }),
            },
        ),
    ])
}

fn irreversible_question() -> BTreeMap<String, Question> {
    BTreeMap::from([(
        "irreversible".into(),
        Question::Noul {
            instructions: "Would `action` in `app` commit an irreversible change outside the \
                           application's local, undoable state?"
                .into(),
            criteria: Some(NoulCriteria {
                yes: "Sends, posts, submits, purchases, pays, deletes, publishes, closes \
                      without saving, or changes account or permission settings"
                    .into(),
                no: "Only navigates, opens, selects, toggles a local option, or edits \
                     content that can still be undone"
                    .into(),
            }),
        },
    )])
}

/// Whether `catalog` is Cua Driver's: a window observation plus a click that
/// takes an exact window `target` and an `element_token`.
fn is_cua_driver(catalog: &[RemoteTool]) -> bool {
    let find = |name: &str| catalog.iter().find(|tool| tool.name == name);
    let click_props = find("click").map(|tool| &tool.input_schema["properties"]);
    find("get_window_state").is_some()
        && click_props
            .is_some_and(|p| p.get("element_token").is_some() && p.get("target").is_some())
}

pub struct DesktopAutopilot {
    client: Arc<McpClient>,
    jev: ProviderConfig,
    require_approval: bool,
}

impl DesktopAutopilot {
    /// `Some` only with a non-empty TypeSafe key and a Cua Driver catalog;
    /// otherwise the agent keeps driving Cua Driver directly.
    pub fn for_catalog(
        catalog: &[RemoteTool],
        client: Arc<McpClient>,
        typesafe_api_key: &str,
        proxy: Option<String>,
        require_approval: bool,
    ) -> Option<Self> {
        if typesafe_api_key.trim().is_empty() || !is_cua_driver(catalog) {
            return None;
        }
        let mut jev = ProviderConfig::openai(
            systemone::DEFAULT_BASE_URL,
            typesafe_api_key.trim(),
            "jev-latest",
        );
        jev.proxy = proxy;
        Some(Self {
            client,
            jev,
            require_approval,
        })
    }
}

/// `None` when the user stopped the turn first.
async fn cancellable<T>(
    env: &dyn ToolEnv,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        out = work => Some(out),
        _ = async {
            while !env.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        } => None,
    }
}

enum Outcome {
    Done(f64),
    HandBack(String),
}

struct Run<'a> {
    tool: &'a DesktopAutopilot,
    env: &'a dyn ToolEnv,
    goal: String,
    hint: Option<String>,
    pid: u64,
    window_id: u64,
    /// What was done so far; also the history Jev sees.
    steps: Vec<String>,
    jev_calls: u32,
    model: String,
    /// Controls past the candidate cap in the latest observation.
    dropped: usize,
}

impl Run<'_> {
    async fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        let result = cancellable(self.env, self.tool.client.tool_call_rich(tool, &args))
            .await
            .ok_or("cancelled by the user; the outcome of a pending Driver call is unknown")?
            .map_err(|e| e.to_string())?;
        if result.is_error {
            return Err(result.text_content());
        }
        result
            .structured_content
            .ok_or_else(|| format!("{tool} returned no structuredContent"))
    }

    async fn ask(
        &mut self,
        state: Value,
        questions: BTreeMap<String, Question>,
    ) -> Result<BTreeMap<String, Answer>, String> {
        let decisions = cancellable(
            self.env,
            systemone::evaluate(&self.tool.jev, &state, &questions),
        )
        .await
        .ok_or("cancelled by the user")?
        .map_err(|e| e.to_string())?;
        self.jev_calls += 1;
        self.model = decisions.model;
        Ok(decisions.answers)
    }

    async fn log(&mut self, line: String) {
        self.env
            .emit(ToolEvent::Stdout {
                chunk: format!("{line}\n"),
            })
            .await;
        self.steps.push(line);
    }

    fn history(&self) -> &[String] {
        &self.steps[self.steps.len().saturating_sub(MAX_HISTORY)..]
    }

    async fn drive(&mut self, max_clicks: u64) -> Outcome {
        let os = std::env::consts::OS;
        let mut clicks = 0;
        // Screen signature before the last click, to spot clicks with no effect.
        let mut before_click: Option<u64> = None;
        let mut unchanged = 0;
        loop {
            if self.env.is_cancelled() {
                return Outcome::HandBack("cancelled by the user".into());
            }
            let observed = self
                .call(
                    "get_window_state",
                    json!({"pid": self.pid, "window_id": self.window_id, "include_screenshot": false}),
                )
                .await
                .and_then(|state| observation(&state, self.pid, self.window_id, os));
            let obs = match observed {
                Ok(obs) => obs,
                Err(e) => return Outcome::HandBack(format!("observe_failed: {e}")),
            };
            self.dropped = obs.dropped;
            if before_click == Some(obs.signature) {
                unchanged += 1;
                if let Some(last) = self.steps.last_mut() {
                    last.push_str(" — window unchanged");
                }
                if unchanged >= 2 {
                    return Outcome::HandBack(
                        "no_effect: the last two clicks left the window unchanged".into(),
                    );
                }
            } else {
                unchanged = 0;
            }
            if obs.candidates.is_empty() {
                return Outcome::HandBack(
                    "no_controls: the accessibility tree offers no labelled native control \
                     (custom-drawn surface, web content, or a partial tree)"
                        .into(),
                );
            }
            let mut state = json!({
                "goal": self.goal,
                "app": obs.app,
                "window": obs.title,
                "screen_text": obs.text,
                "controls": obs.candidates.iter()
                    .map(|c| format!("{} \"{}\" ({})", c.class.name(), c.label, c.state))
                    .collect::<Vec<_>>(),
                "history": self.history(),
            });
            if let Some(hint) = &self.hint {
                state["hint"] = json!(hint);
            }
            let settled = match self.ask(state, step_questions(&obs)).await {
                Ok(answers) => settle(&obs, &answers),
                Err(e) => Err(e),
            };
            let index = match settled {
                Ok(Move::Click(index)) => index,
                Ok(Move::Done(p)) => return Outcome::Done(p),
                Ok(Move::HandBack(reason)) => return Outcome::HandBack(reason),
                Err(e) => return Outcome::HandBack(format!("jev_error: {e}")),
            };
            let chosen = obs.candidates[index].clone();
            if clicks == max_clicks {
                return Outcome::HandBack(format!(
                    "step_limit: {max_clicks} click(s) without Jev judging the goal done; \
                     it would next \"{}\"",
                    chosen.description()
                ));
            }
            let check = json!({
                "goal": self.goal,
                "app": obs.app,
                "window": obs.title,
                "action": chosen.description(),
            });
            match self.ask(check, irreversible_question()).await {
                Ok(answers) => match answers.get("irreversible") {
                    Some(Answer::Noul { noul }) if *noul < IRREVERSIBLE_AT => {}
                    Some(Answer::Noul { noul }) => {
                        return Outcome::HandBack(format!(
                            "needs_confirmation: Jev rates \"{}\" irreversible (p={noul:.2})",
                            chosen.description()
                        ))
                    }
                    _ => return Outcome::HandBack("jev_error: Jev omitted irreversible".into()),
                },
                Err(e) => return Outcome::HandBack(format!("jev_error: {e}")),
            }
            let clicked = self
                .call(
                    "click",
                    json!({
                        "target": {"kind": "window", "pid": self.pid, "window_id": self.window_id},
                        "element_token": chosen.token,
                        "delivery_mode": "background",
                    }),
                )
                .await;
            match clicked {
                Ok(result) => {
                    let effect = result
                        .get("effect")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    self.log(format!("{} (effect: {effect})", chosen.description()))
                        .await;
                }
                // Tokens are snapshot-bound; observe again and let Jev re-pick.
                Err(e) if e.contains("stale") => {
                    self.log(format!(
                        "{} — token went stale, observing again",
                        chosen.description()
                    ))
                    .await;
                }
                Err(e) => return Outcome::HandBack(format!("click_failed: {e}")),
            }
            clicks += 1;
            before_click = Some(obs.signature);
        }
    }

    fn report(&self, outcome: &Outcome) -> String {
        let (head, next) = match outcome {
            Outcome::Done(p) => (
                format!("done (Jev goal_done p={p:.2})"),
                "Verify the goal yourself from a fresh get_window_state (and the filesystem \
                 for saves or exports) before reporting success."
                    .to_string(),
            ),
            Outcome::HandBack(reason) => (
                format!("handed back: {reason}"),
                format!(
                    "You are System Two now. Observe pid {} / window {} with get_window_state, \
                     resolve the reason above yourself through Cua Driver (ask the user before \
                     anything irreversible; never type secrets), then call {NAME} again with a \
                     `hint` to let Jev continue, or finish by hand.",
                    self.pid, self.window_id
                ),
            ),
        };
        let mut out = format!("{NAME}: {head}\ngoal: {}\n", self.goal);
        if self.steps.is_empty() {
            out.push_str("steps: none\n");
        } else {
            out.push_str("steps:\n");
            for (n, step) in self.steps.iter().enumerate() {
                out.push_str(&format!("{}. {step}\n", n + 1));
            }
        }
        if self.dropped > 0 {
            out.push_str(&format!(
                "note: {} control(s) past the {MAX_CANDIDATES}-candidate cap were not offered to Jev\n",
                self.dropped
            ));
        }
        let model = if self.model.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.model)
        };
        out.push_str(&format!("jev: {} call(s){model}\n{next}", self.jev_calls));
        out
    }
}

#[async_trait]
impl Tool for DesktopAutopilot {
    fn name(&self) -> &str {
        NAME
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            NAME,
            "Drive one exact Cua Driver window toward a goal with TypeSafe Jev (fast System One). \
             Jev picks each click from the window's labelled controls and hands the window back \
             to you, with the reason, for text entry, risky or irreversible actions, low \
             confidence, clicks with no effect, or errors. Verify a reported done yourself.",
            json!({
                "type": "object",
                "properties": {
                    "goal": {"type": "string", "description": "The finished state the window should show, in plain words."},
                    "pid": {"type": "integer", "description": "Exact process id from list_windows."},
                    "window_id": {"type": "integer", "description": "Exact window id from list_windows."},
                    "hint": {"type": "string", "description": "Your guidance after a hand-back, such as the control to use next."},
                    "max_steps": {"type": "integer", "minimum": 1, "maximum": MAX_CLICKS, "description": "Clicks before handing back (default 12)."}
                },
                "required": ["goal", "pid", "window_id"]
            }),
        )
    }
    /// Discovered with Cua Driver's own tools through `search_mcp_tools`.
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
        str_field(args, "goal").chars().take(120).collect()
    }
    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let goal = collapse(str_field(args, "goal"));
        let (Some(pid), Some(window_id)) = (
            args.get("pid").and_then(Value::as_u64),
            args.get("window_id").and_then(Value::as_u64),
        ) else {
            return ToolResult::fail(
                "pid and window_id must be the exact integers from list_windows",
            );
        };
        if goal.is_empty() {
            return ToolResult::fail("missing required argument 'goal'");
        }
        // Every click below runs unattended, so a per-click ask or deny rule
        // for Cua Driver's own `click` must keep the agent in charge.
        if env.approval_mode("click").await != Approval::Allow {
            return ToolResult::fail(
                "Cua Driver `click` requires approval or is denied here, so it cannot run \
                 unattended; drive Cua Driver directly instead.",
            );
        }
        let max_clicks = args
            .get("max_steps")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_CLICKS)
            .clamp(1, MAX_CLICKS);
        let mut run = Run {
            tool: self,
            env,
            goal,
            hint: Some(collapse(str_field(args, "hint"))).filter(|h| !h.is_empty()),
            pid,
            window_id,
            steps: Vec::new(),
            jev_calls: 0,
            model: String::new(),
            dropped: 0,
        };
        let outcome = run.drive(max_clicks).await;
        ToolResult::ok(run.report(&outcome))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from Cua's recorded WPF harness window
    /// (`fixtures/native/wpf-window-state-initial-v1.json`), plus a static
    /// label, a disabled button, and an off-screen button.
    fn wpf_window() -> Value {
        let el = |index: u64, role: &str, label: &str, parent: Option<u64>, y: f64| {
            let mut e = json!({
                "element_index": index, "element_token": format!("s1:{index}"), "role": role,
                "label": label, "enabled": true, "frame": {"x": 300.0, "y": y, "w": 90.0, "h": 20.0}
            });
            if let Some(parent) = parent {
                e["parent_index"] = json!(parent);
            }
            e
        };
        let mut elements = vec![
            el(0, "TitleBar", "CuaTestHarness WPF Tasks", None, 183.0),
            el(1, "MenuItem", "System", Some(0), 188.0),
            el(2, "Button", "Close", Some(0), 181.0),
            el(3, "Button", "Increment", None, 251.0),
            el(4, "Button", "Reset", None, 251.0),
            el(5, "CheckBox", "I agree", None, 279.0),
            el(6, "RadioButton", "Large", None, 302.0),
            el(7, "Edit", "Note", None, 325.0),
            el(8, "Text", "Counter: 2", None, 230.0),
            el(9, "Button", "Disabled", None, 345.0),
            el(10, "Button", "Elsewhere", None, 900.0),
            el(11, "Button", "Exit", None, 345.0),
        ];
        elements[6]["selected"] = json!(true);
        elements[7]["value"] = json!("draft");
        elements[9]["enabled"] = json!(false);
        json!({
            "pid": 6544, "window_id": 328014, "app_name": "CuaTestHarness.Wpf.exe",
            "window_title": "CuaTestHarness WPF Tasks", "elements": elements,
            "window_bounds": {"x": 279.0, "y": 180.0, "width": 466.0, "height": 353.0}
        })
    }

    #[test]
    fn window_state_becomes_application_candidates_and_screen_text() {
        let obs = observation(&wpf_window(), 6544, 328014, "windows").unwrap();
        let ids: Vec<_> = obs.candidates.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "button:increment",
                "button:reset",
                "checkbox:i-agree",
                "radio:large",
                "text_input:note",
                "button:exit"
            ]
        );
        let state = |id: &str| obs.candidates.iter().find(|c| c.id == id).unwrap().state;
        assert_eq!(state("radio:large"), "selected");
        assert_eq!(state("text_input:note"), "has text");
        let risky: Vec<_> = obs
            .candidates
            .iter()
            .filter(|c| c.risky)
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(risky, ["button:reset", "button:exit"]);
        assert_eq!(obs.text, ["Counter: 2"]);
        // Field values never reach what Jev sees.
        assert!(!format!("{obs:?}").contains("draft"));
        assert!(observation(&wpf_window(), 6544, 1, "windows").is_err());
        // The same raw role means different controls per platform.
        assert_eq!(role_class("Text", "windows"), None);
        assert_eq!(role_class("text", "linux"), Some(Class::TextInput));
        assert_eq!(
            role_class("AXSecureTextField", "macos"),
            Some(Class::TextInput)
        );
    }

    #[test]
    fn risk_phrases_match_whole_words_and_chinese() {
        for label in ["Send", "Don’t Save", "Clear all", "删除文件", "立即购买"] {
            assert!(risky(label), "{label}");
        }
        for label in ["Sender", "Save note", "Increment", "保存"] {
            assert!(!risky(label), "{label}");
        }
    }

    #[test]
    fn settle_acts_only_on_a_confident_safe_click() {
        let obs = observation(&wpf_window(), 6544, 328014, "windows").unwrap();
        let answers = |choice: &str, confidence: f64, done: f64| {
            BTreeMap::from([
                ("goal_done".to_string(), Answer::Noul { noul: done }),
                (
                    "next_action".to_string(),
                    Answer::Choice {
                        choice: choice.into(),
                        probabilities: BTreeMap::from([(choice.to_string(), confidence)]),
                        confidence,
                    },
                ),
            ])
        };
        let reason = |m: Move| match m {
            Move::HandBack(reason) => reason.split(':').next().unwrap().to_string(),
            other => panic!("expected a hand-back, got {other:?}"),
        };
        assert_eq!(
            settle(&obs, &answers("button:increment", 0.9, 0.1)),
            Ok(Move::Click(0))
        );
        assert_eq!(
            settle(&obs, &answers("button:increment", 0.9, 0.85)),
            Ok(Move::Done(0.85))
        );
        assert_eq!(
            reason(settle(&obs, &answers(HAND_BACK, 0.9, 0.1)).unwrap()),
            "jev_hand_back"
        );
        assert_eq!(
            reason(settle(&obs, &answers("button:increment", 0.3, 0.1)).unwrap()),
            "low_confidence"
        );
        assert_eq!(
            reason(settle(&obs, &answers("button:increment", f64::NAN, 0.1)).unwrap()),
            "low_confidence"
        );
        assert_eq!(
            reason(settle(&obs, &answers("text_input:note", 0.9, 0.1)).unwrap()),
            "needs_text"
        );
        assert_eq!(
            reason(settle(&obs, &answers("button:reset", 0.9, 0.1)).unwrap()),
            "needs_confirmation"
        );
        assert!(settle(&obs, &answers("button:invented", 0.9, 0.1)).is_err());
        assert!(settle(&obs, &BTreeMap::new()).is_err());
    }

    #[test]
    fn only_a_cua_driver_catalog_qualifies() {
        let tool = |name: &str, props: Value| RemoteTool {
            name: name.into(),
            title: None,
            description: String::new(),
            input_schema: json!({"type": "object", "properties": props}),
            output_schema: None,
            meta: None,
            annotations: None,
        };
        let observe = tool("get_window_state", json!({}));
        let token_click = tool("click", json!({"target": {}, "element_token": {}}));
        let xy_click = tool("click", json!({"x": {}, "y": {}}));
        assert!(is_cua_driver(&[observe.clone(), token_click.clone()]));
        assert!(!is_cua_driver(&[observe, xy_click]));
        assert!(!is_cua_driver(&[token_click]));
    }
}
