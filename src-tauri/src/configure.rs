//! Agent-facing app configuration: read/write an allowlisted subset of
//! Settings without sending the user through the Settings UI.
//!
//! Secrets, model profiles, workspace directory, and proxy stay out of this
//! catalog. Appearance is persisted in SQLite so the tool can read it, and
//! a `app_prefs` presentation lets the live UI apply the same change.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use tauri::State;
use wisp_llm::ToolSchema;
use wisp_store::Store;
use wisp_tools::{Tool, ToolEnv, ToolResult};

use crate::AppState;

const APPEARANCE_KEY: &str = "appearance_prefs";

const UI_FONT_DEFAULT: u16 = 14;
const CODE_FONT_DEFAULT: u16 = 12;
const FONT_SIZE_MAX: u16 = 30;
const DEFAULT_MAX_ITER: i64 = 100;
const DEFAULT_AUTO_CONTINUE_LIMIT: u64 = 10;

const LIGHT_PALETTES: &[&str] = &["paper", "codex", "github", "catppuccin", "everforest"];
const DARK_PALETTES: &[&str] = &["charcoal", "codex", "github", "catppuccin", "gruvbox"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Int,
    Bool,
    Enum,
    String,
}

#[derive(Debug, Clone, Copy)]
struct SettingSpec {
    key: &'static str,
    kind: ValueKind,
    writable: bool,
    summary: &'static str,
}

const CATALOG: &[SettingSpec] = &[
    SettingSpec {
        key: "ui_font_size",
        kind: ValueKind::Int,
        writable: true,
        summary: "UI font size in px (0-30). Relative strings such as +2 or -1 are allowed.",
    },
    SettingSpec {
        key: "code_font_size",
        kind: ValueKind::Int,
        writable: true,
        summary: "Code / monospace font size in px (0-30). Relative strings such as +2 or -1 are allowed.",
    },
    SettingSpec {
        key: "ui_font_family",
        kind: ValueKind::String,
        writable: true,
        summary: "UI font family name. Empty restores the default stack.",
    },
    SettingSpec {
        key: "code_font_family",
        kind: ValueKind::String,
        writable: true,
        summary: "Code font family name. Empty restores the default stack.",
    },
    SettingSpec {
        key: "theme",
        kind: ValueKind::Enum,
        writable: true,
        summary: "Color theme: system, light, or dark.",
    },
    SettingSpec {
        key: "light_palette",
        kind: ValueKind::Enum,
        writable: true,
        summary: "Light-theme palette: paper, codex, github, catppuccin, everforest.",
    },
    SettingSpec {
        key: "dark_palette",
        kind: ValueKind::Enum,
        writable: true,
        summary: "Dark-theme palette: charcoal, codex, github, catppuccin, gruvbox.",
    },
    SettingSpec {
        key: "locale",
        kind: ValueKind::Enum,
        writable: true,
        summary: "UI language: en or zh.",
    },
    SettingSpec {
        key: "max_iter",
        kind: ValueKind::Int,
        writable: true,
        summary: "Max model/tool iterations per turn. 0 means unlimited. Default 100.",
    },
    SettingSpec {
        key: "auto_compact",
        kind: ValueKind::Bool,
        writable: true,
        summary: "Automatically compact long conversations near the context limit.",
    },
    SettingSpec {
        key: "semantic_compact_on_model_switch",
        kind: ValueKind::Bool,
        writable: true,
        summary: "Run semantic compaction after switching the conversation model.",
    },
    SettingSpec {
        key: "semantic_compact_idle_hours",
        kind: ValueKind::Int,
        writable: true,
        summary: "Prompt for semantic compaction after this many idle hours. 0 disables. Default 24.",
    },
    SettingSpec {
        key: "auto_continue",
        kind: ValueKind::Bool,
        writable: true,
        summary: "Automatically continue when the model hits its output-token ceiling.",
    },
    SettingSpec {
        key: "auto_continue_limit",
        kind: ValueKind::Int,
        writable: true,
        summary: "Max automatic continue rounds per turn (minimum 1). Default 10.",
    },
    SettingSpec {
        key: "follow_up_questions",
        kind: ValueKind::Bool,
        writable: true,
        summary: "Generate three follow-up questions after each reply.",
    },
    SettingSpec {
        key: "decentralized_project_storage",
        kind: ValueKind::Bool,
        writable: true,
        summary: "Store new projects in their own folders. Existing project locations are unchanged.",
    },
    SettingSpec {
        key: "resume_last_session",
        kind: ValueKind::Bool,
        writable: true,
        summary: "Restore the most recent conversation when a workspace opens.",
    },
    SettingSpec {
        key: "notifications_enabled",
        kind: ValueKind::Bool,
        writable: true,
        summary: "Desktop notifications when the window is in the background.",
    },
    SettingSpec {
        key: "selection_popup_enabled",
        kind: ValueKind::Bool,
        writable: true,
        summary: "Show the selection quick-actions popup after selecting text.",
    },
    SettingSpec {
        key: "send_with_modifier",
        kind: ValueKind::Bool,
        writable: true,
        summary: "When true, send with Shift+Enter (Enter inserts a newline). When false, Enter sends.",
    },
    SettingSpec {
        key: "custom_css",
        kind: ValueKind::String,
        writable: true,
        summary: "User theme CSS injected after the built-in stylesheet. Empty clears it. Remote url()/ @import are stripped. Max 64KB.",
    },
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppearancePrefs {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_light_palette")]
    pub light_palette: String,
    #[serde(default = "default_dark_palette")]
    pub dark_palette: String,
    #[serde(default = "default_ui_font_size")]
    pub ui_font_size: u16,
    #[serde(default = "default_code_font_size")]
    pub code_font_size: u16,
    #[serde(default)]
    pub ui_font_family: String,
    #[serde(default)]
    pub code_font_family: String,
    #[serde(default = "default_true")]
    pub selection_popup_enabled: bool,
    #[serde(default)]
    pub send_with_modifier: bool,
    #[serde(default)]
    pub custom_css: String,
}

impl Default for AppearancePrefs {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            light_palette: default_light_palette(),
            dark_palette: default_dark_palette(),
            ui_font_size: UI_FONT_DEFAULT,
            code_font_size: CODE_FONT_DEFAULT,
            ui_font_family: String::new(),
            code_font_family: String::new(),
            selection_popup_enabled: true,
            send_with_modifier: false,
            custom_css: String::new(),
        }
    }
}

fn default_theme() -> String {
    "system".into()
}
fn default_light_palette() -> String {
    "paper".into()
}
fn default_dark_palette() -> String {
    "charcoal".into()
}
fn default_ui_font_size() -> u16 {
    UI_FONT_DEFAULT
}
fn default_code_font_size() -> u16 {
    CODE_FONT_DEFAULT
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppearancePrefsView {
    pub saved: bool,
    #[serde(flatten)]
    pub prefs: AppearancePrefs,
}

pub(crate) struct ConfigureTool {
    store: Store,
    app_data: PathBuf,
    project_id: String,
}

impl ConfigureTool {
    pub(crate) fn new(
        store: Store,
        app_data: impl Into<PathBuf>,
        project_id: impl Into<String>,
    ) -> Self {
        Self {
            store,
            app_data: app_data.into(),
            project_id: project_id.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ConfigureArgs {
    #[serde(default)]
    action: String,
    #[serde(default)]
    keys: Vec<String>,
    #[serde(default)]
    values: Map<String, Value>,
    #[serde(default)]
    project_id: String,
}

#[async_trait::async_trait]
impl Tool for ConfigureTool {
    fn name(&self) -> &str {
        "configure"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "configure",
            "Read or change Wisp app settings from this conversation, or report disk storage for the current project. \
Use this instead of sending the user to Settings for allowlisted preferences. \
Examples: bigger UI text → set ui_font_size to \"+2\"; switch to dark theme → set theme to \"dark\"; \
import a custom theme → set custom_css to the stylesheet text; \
\"show this project's storage\" → action storage. \
Secrets, API keys, model profiles, workspace directory, and proxy are not writable here. \
For specialists, get the `specialists` key then call save_specialist (pass id to update).",
            json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "set", "storage"],
                        "description": "get = read current values (omit keys for the full catalog). set = change allowlisted keys via values. storage = disk usage for this project, shown in the conversation."
                    },
                    "keys": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "For get: setting keys, or `specialists` to list personas. Omit to return every catalog key plus specialists."
                    },
                    "values": {
                        "type": "object",
                        "description": "For set: map of key → new value. Numeric keys accept +N / -N strings for a relative change."
                    },
                    "project_id": {
                        "type": "string",
                        "description": "For storage: project id to report. Omit for the current project. Pass * for every project plus app-data totals."
                    }
                },
                "required": ["action"]
            }),
        )
    }

    fn preview(&self, args: &Value) -> String {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("get")
            .trim();
        match action {
            "set" => {
                let keys = args
                    .get("values")
                    .and_then(Value::as_object)
                    .map(|values| {
                        let mut keys: Vec<&str> = values.keys().map(String::as_str).collect();
                        keys.sort_unstable();
                        keys.join(", ")
                    })
                    .unwrap_or_default();
                if keys.is_empty() {
                    "set".into()
                } else {
                    format!("set {keys}")
                }
            }
            "storage" => {
                let project = args
                    .get("project_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .unwrap_or(self.project_id.as_str());
                format!("storage {project}")
            }
            _ => {
                let keys = args
                    .get("keys")
                    .and_then(Value::as_array)
                    .map(|keys| {
                        keys.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                if keys.is_empty() {
                    "get".into()
                } else {
                    format!("get {keys}")
                }
            }
        }
    }

    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let parsed: ConfigureArgs = match serde_json::from_value(args.clone()) {
            Ok(parsed) => parsed,
            Err(error) => return ToolResult::fail(format!("configure args error: {error}")),
        };
        match parsed.action.trim() {
            "get" => match snapshot(&self.store, &parsed.keys).await {
                Ok(text) => ToolResult::ok(text),
                Err(error) => ToolResult::fail(format!("configure error: {error}")),
            },
            "set" => match apply_patch(&self.store, &parsed.values).await {
                Ok(outcome) => {
                    if !outcome.presentation.is_null() {
                        env.emit(wisp_tools::ToolEvent::Presentation {
                            kind: "app_prefs".into(),
                            payload: outcome.presentation,
                            server: None,
                        })
                        .await;
                    }
                    ToolResult::ok(outcome.report)
                }
                Err(error) => ToolResult::fail(format!("configure error: {error}")),
            },
            "storage" => {
                let project = parsed.project_id.trim();
                let project = if project.is_empty() {
                    self.project_id.as_str()
                } else {
                    project
                };
                match storage_report(&self.store, &self.app_data, project).await {
                    Ok((text, payload)) => {
                        env.emit(wisp_tools::ToolEvent::Presentation {
                            kind: "storage_usage".into(),
                            payload,
                            server: None,
                        })
                        .await;
                        ToolResult::ok(text)
                    }
                    Err(error) => ToolResult::fail(format!("configure error: {error}")),
                }
            }
            other => ToolResult::fail(format!(
                "configure error: unknown action '{other}'. Use get, set, or storage."
            )),
        }
    }
}

struct SetOutcome {
    report: String,
    presentation: Value,
}

fn spec(key: &str) -> Option<&'static SettingSpec> {
    CATALOG.iter().find(|spec| spec.key == key)
}

pub(crate) async fn load_appearance(store: &Store) -> AppearancePrefsView {
    match store.get_setting(APPEARANCE_KEY).await {
        Ok(Some(raw)) => match serde_json::from_str::<AppearancePrefs>(&raw) {
            Ok(prefs) => AppearancePrefsView {
                saved: true,
                prefs: sanitize_appearance(prefs),
            },
            Err(_) => AppearancePrefsView {
                saved: false,
                prefs: AppearancePrefs::default(),
            },
        },
        _ => AppearancePrefsView {
            saved: false,
            prefs: AppearancePrefs::default(),
        },
    }
}

pub(crate) async fn save_appearance(
    store: &Store,
    prefs: AppearancePrefs,
) -> Result<AppearancePrefs, String> {
    let prefs = sanitize_appearance(prefs);
    let json = serde_json::to_string(&prefs).map_err(|error| error.to_string())?;
    store
        .set_setting(APPEARANCE_KEY, &json)
        .await
        .map_err(|error| error.to_string())?;
    Ok(prefs)
}

fn sanitize_appearance(mut prefs: AppearancePrefs) -> AppearancePrefs {
    prefs.theme = normalize_theme(&prefs.theme).unwrap_or_else(|_| default_theme());
    prefs.light_palette =
        normalize_enum(&prefs.light_palette, LIGHT_PALETTES).unwrap_or_else(default_light_palette);
    prefs.dark_palette =
        normalize_enum(&prefs.dark_palette, DARK_PALETTES).unwrap_or_else(default_dark_palette);
    prefs.ui_font_size = prefs.ui_font_size.min(FONT_SIZE_MAX);
    prefs.code_font_size = prefs.code_font_size.min(FONT_SIZE_MAX);
    prefs.ui_font_family = sanitize_font_family(&prefs.ui_font_family);
    prefs.code_font_family = sanitize_font_family(&prefs.code_font_family);
    prefs.custom_css = sanitize_custom_css(&prefs.custom_css);
    prefs
}

pub(crate) const CUSTOM_CSS_MAX_BYTES: usize = 64_000;

/// Strip constructs that can load remote content or break out of a `<style>` tag.
/// Keep in sync with `ui/src/app_support/prefs.rs`.
pub(crate) fn sanitize_custom_css(input: &str) -> String {
    let mut css = String::with_capacity(input.len().min(CUSTOM_CSS_MAX_BYTES));
    for ch in input.chars() {
        if ch == '\0' {
            continue;
        }
        if css.len() + ch.len_utf8() > CUSTOM_CSS_MAX_BYTES {
            break;
        }
        css.push(ch);
    }
    let css = strip_ascii_ci(&css, "javascript:");
    let css = strip_ascii_ci(&css, "expression(");
    let css = strip_ascii_ci(&css, "behavior:");
    let css = strip_ascii_ci(&css, "-moz-binding");
    let css = strip_ascii_ci(&css, "</style");
    let css = strip_ascii_ci(&css, "<script");
    let css = strip_at_keyword(&css, "@import");
    let css = strip_at_keyword(&css, "@namespace");
    strip_url_functions(&css)
}

fn strip_ascii_ci(input: &str, needle: &str) -> String {
    let needle_l = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while !rest.is_empty() {
        let lower = rest.to_ascii_lowercase();
        if let Some(idx) = lower.find(&needle_l) {
            out.push_str(&rest[..idx]);
            rest = &rest[idx + needle.len()..];
        } else {
            out.push_str(rest);
            break;
        }
    }
    out
}

fn strip_at_keyword(input: &str, keyword: &str) -> String {
    let needle = keyword.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while !rest.is_empty() {
        let lower = rest.to_ascii_lowercase();
        if let Some(idx) = lower.find(&needle) {
            out.push_str(&rest[..idx]);
            let after = &rest[idx + keyword.len()..];
            if let Some(end) = after.find([';', '\n']) {
                rest = &after[end + 1..];
            } else {
                rest = "";
            }
        } else {
            out.push_str(rest);
            break;
        }
    }
    out
}

fn strip_url_functions(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while !rest.is_empty() {
        let lower = rest.to_ascii_lowercase();
        if let Some(idx) = lower.find("url(") {
            out.push_str(&rest[..idx]);
            let after = &rest[idx + 4..];
            if let Some(end) = after.find(')') {
                rest = &after[end + 1..];
            } else {
                rest = "";
            }
        } else {
            out.push_str(rest);
            break;
        }
    }
    out
}

fn sanitize_font_family(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|c| !matches!(c, ';' | '{' | '}' | '"' | '\'' | '!' | '(' | ')'))
        .take(100)
        .collect::<String>()
        .trim()
        .to_string()
}

fn normalize_theme(value: &str) -> Result<String, String> {
    normalize_enum(value, &["system", "light", "dark"])
        .ok_or_else(|| "theme must be system, light, or dark".into())
}

fn normalize_enum(value: &str, allowed: &[&str]) -> Option<String> {
    let trimmed = value.trim();
    allowed
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(trimmed))
        .map(|candidate| (*candidate).to_string())
}

fn normalize_locale(value: &str) -> Result<String, String> {
    match value.trim() {
        "zh" | "zh-CN" | "zh-TW" | "zh-Hans" | "zh-Hant" | "chinese" | "中文" => Ok("zh".into()),
        "en" | "en-US" | "en-GB" | "english" => Ok("en".into()),
        other if !other.is_empty() => Err(format!("locale must be en or zh (got '{other}')")),
        _ => Err("locale must be en or zh".into()),
    }
}

async fn snapshot(store: &Store, keys: &[String]) -> Result<String, String> {
    let mut requested: Vec<String> = keys
        .iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect();
    let include_all = requested.is_empty();
    if include_all {
        requested = CATALOG.iter().map(|spec| spec.key.to_string()).collect();
        requested.push("specialists".into());
    }

    let current = current_values(store).await?;
    let mut lines = vec![
        "Wisp configuration (allowlisted; secrets and model profiles are not included)."
            .to_string(),
        String::new(),
    ];
    let mut unknown = Vec::new();
    for key in &requested {
        if key == "specialists" {
            lines.push(render_specialists(&crate::specialists::ensure(store).await));
            continue;
        }
        let Some(spec) = spec(key) else {
            unknown.push(key.clone());
            continue;
        };
        let value = current.get(key).cloned().unwrap_or(Value::Null);
        lines.push(format!(
            "- `{key}` = {} ({})",
            render_value(&value),
            spec.summary
        ));
    }
    if !unknown.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "Unknown keys (not in the catalog): {}.",
            unknown.join(", ")
        ));
        lines.push(format!("Writable keys: {}.", writable_keys_csv()));
    }
    Ok(lines.join("\n"))
}

fn render_specialists(list: &[crate::specialists::Specialist]) -> String {
    let mut lines = vec!["- `specialists`:".to_string()];
    for spec in list {
        let kind = if spec.builtin { "builtin" } else { "custom" };
        let model = if spec.model_id.trim().is_empty() {
            "follow-active-model"
        } else {
            spec.model_id.as_str()
        };
        lines.push(format!(
            "  - {} ({}, {kind}, model={model}): {}",
            spec.name,
            spec.id,
            if spec.description.trim().is_empty() {
                "no description"
            } else {
                spec.description.trim()
            }
        ));
    }
    lines.push(
        "  Create or update with `save_specialist` (pass `id` to edit a custom specialist).".into(),
    );
    lines.join("\n")
}

fn writable_keys_csv() -> String {
    CATALOG
        .iter()
        .filter(|spec| spec.writable)
        .map(|spec| spec.key)
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_value(value: &Value) -> String {
    match value {
        Value::String(text) if text.is_empty() => "(default)".into(),
        Value::String(text) if text.len() > 80 => format!("({} bytes)", text.len()),
        Value::String(text) => format!("\"{text}\""),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

async fn current_values(store: &Store) -> Result<Map<String, Value>, String> {
    let appearance = load_appearance(store).await.prefs;
    let locale = store
        .get_setting("locale")
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| "en".into());
    let max_iter = store
        .get_setting("max_iter")
        .await
        .map_err(|error| error.to_string())?
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value >= 0)
        .unwrap_or(DEFAULT_MAX_ITER);
    let auto_compact = store
        .get_setting("auto_compact")
        .await
        .map_err(|error| error.to_string())?
        .map(|value| value != "false")
        .unwrap_or(true);
    let semantic_compact_on_model_switch = store
        .get_setting("semantic_compact_on_model_switch")
        .await
        .map_err(|error| error.to_string())?
        .is_some_and(|value| value == "true");
    let semantic_compact_idle_hours = store
        .get_setting("semantic_compact_idle_hours")
        .await
        .map_err(|error| error.to_string())?
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(24);
    let auto_continue = store
        .get_setting("auto_continue")
        .await
        .map_err(|error| error.to_string())?
        .is_some_and(|value| value == "true");
    let auto_continue_limit = store
        .get_setting("auto_continue_limit")
        .await
        .map_err(|error| error.to_string())?
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_AUTO_CONTINUE_LIMIT)
        .max(1);
    let follow_up_questions = store
        .get_setting("follow_up_questions")
        .await
        .map_err(|error| error.to_string())?
        .map(|value| value == "true")
        .unwrap_or(true);
    let resume_last_session = store
        .get_setting("resume_last_session")
        .await
        .map_err(|error| error.to_string())?
        .map(|value| value == "true")
        .unwrap_or(true);
    let notifications_enabled = store
        .get_setting("notifications_enabled")
        .await
        .map_err(|error| error.to_string())?
        .map(|value| value != "false")
        .unwrap_or(true);

    let mut values = Map::new();
    values.insert("ui_font_size".into(), json!(appearance.ui_font_size));
    values.insert("code_font_size".into(), json!(appearance.code_font_size));
    values.insert("ui_font_family".into(), json!(appearance.ui_font_family));
    values.insert(
        "code_font_family".into(),
        json!(appearance.code_font_family),
    );
    values.insert("theme".into(), json!(appearance.theme));
    values.insert("light_palette".into(), json!(appearance.light_palette));
    values.insert("dark_palette".into(), json!(appearance.dark_palette));
    values.insert(
        "selection_popup_enabled".into(),
        json!(appearance.selection_popup_enabled),
    );
    values.insert(
        "send_with_modifier".into(),
        json!(appearance.send_with_modifier),
    );
    values.insert("custom_css".into(), json!(appearance.custom_css));
    values.insert("locale".into(), json!(locale));
    values.insert("max_iter".into(), json!(max_iter));
    values.insert("auto_compact".into(), json!(auto_compact));
    values.insert(
        "semantic_compact_on_model_switch".into(),
        json!(semantic_compact_on_model_switch),
    );
    values.insert(
        "semantic_compact_idle_hours".into(),
        json!(semantic_compact_idle_hours),
    );
    values.insert("auto_continue".into(), json!(auto_continue));
    values.insert("auto_continue_limit".into(), json!(auto_continue_limit));
    values.insert("follow_up_questions".into(), json!(follow_up_questions));
    values.insert("resume_last_session".into(), json!(resume_last_session));
    values.insert(
        "decentralized_project_storage".into(),
        json!(store
            .decentralized_project_storage()
            .await
            .map_err(|e| e.to_string())?),
    );
    values.insert("notifications_enabled".into(), json!(notifications_enabled));
    Ok(values)
}

async fn apply_patch(store: &Store, values: &Map<String, Value>) -> Result<SetOutcome, String> {
    if values.is_empty() {
        return Err("values is required for action set".into());
    }
    let mut appearance = load_appearance(store).await.prefs;
    let mut appearance_dirty = false;
    let mut changed = Vec::new();
    let mut presentation = Map::new();

    let mut keys: Vec<&String> = values.keys().collect();
    keys.sort();
    for key in keys {
        let incoming = values.get(key).expect("key from map");
        if looks_secret(key) {
            return Err(format!(
                "'{key}' is not writable through configure (secrets stay in Settings → Credentials)"
            ));
        }
        let Some(spec) = spec(key) else {
            return Err(format!(
                "unknown setting '{key}'. Call action get with no keys for the catalog."
            ));
        };
        if !spec.writable {
            return Err(format!("'{key}' is read-only"));
        }
        let applied = apply_one(
            store,
            &mut appearance,
            &mut appearance_dirty,
            spec,
            incoming,
        )
        .await?;
        changed.push(format!("{key}: {applied}"));
        presentation.insert(key.clone(), json_from_applied(spec, &applied));
    }

    if appearance_dirty {
        save_appearance(store, appearance.clone()).await?;
        presentation.insert("theme".into(), json!(appearance.theme));
        presentation.insert("light_palette".into(), json!(appearance.light_palette));
        presentation.insert("dark_palette".into(), json!(appearance.dark_palette));
        presentation.insert("ui_font_size".into(), json!(appearance.ui_font_size));
        presentation.insert("code_font_size".into(), json!(appearance.code_font_size));
        presentation.insert("ui_font_family".into(), json!(appearance.ui_font_family));
        presentation.insert(
            "code_font_family".into(),
            json!(appearance.code_font_family),
        );
        presentation.insert(
            "selection_popup_enabled".into(),
            json!(appearance.selection_popup_enabled),
        );
        presentation.insert(
            "send_with_modifier".into(),
            json!(appearance.send_with_modifier),
        );
        presentation.insert("custom_css".into(), json!(appearance.custom_css));
    }

    Ok(SetOutcome {
        report: format!("Updated {}.", changed.join("; ")),
        presentation: Value::Object(presentation),
    })
}

fn json_from_applied(spec: &SettingSpec, applied: &str) -> Value {
    match spec.kind {
        ValueKind::Bool => json!(applied == "true"),
        ValueKind::Int => applied
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| json!(applied)),
        ValueKind::Enum | ValueKind::String => json!(applied),
    }
}

fn looks_secret(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "api_key"
            | "apikey"
            | "token"
            | "secret"
            | "password"
            | "proxy"
            | "proxy_url"
            | "workspace_dir"
            | "api_url"
            | "provider"
            | "sync_relay_token"
    ) || lower.ends_with("_key")
        || lower.ends_with("_token")
        || lower.ends_with("_secret")
        || lower.ends_with("_password")
}

async fn apply_one(
    store: &Store,
    appearance: &mut AppearancePrefs,
    appearance_dirty: &mut bool,
    spec: &SettingSpec,
    incoming: &Value,
) -> Result<String, String> {
    match spec.key {
        "ui_font_size" => {
            appearance.ui_font_size = resolve_int(
                incoming,
                appearance.ui_font_size as i64,
                0,
                FONT_SIZE_MAX as i64,
            )? as u16;
            *appearance_dirty = true;
            Ok(appearance.ui_font_size.to_string())
        }
        "code_font_size" => {
            appearance.code_font_size = resolve_int(
                incoming,
                appearance.code_font_size as i64,
                0,
                FONT_SIZE_MAX as i64,
            )? as u16;
            *appearance_dirty = true;
            Ok(appearance.code_font_size.to_string())
        }
        "ui_font_family" => {
            appearance.ui_font_family = sanitize_font_family(&string_value(incoming)?);
            *appearance_dirty = true;
            Ok(if appearance.ui_font_family.is_empty() {
                "(default)".into()
            } else {
                appearance.ui_font_family.clone()
            })
        }
        "code_font_family" => {
            appearance.code_font_family = sanitize_font_family(&string_value(incoming)?);
            *appearance_dirty = true;
            Ok(if appearance.code_font_family.is_empty() {
                "(default)".into()
            } else {
                appearance.code_font_family.clone()
            })
        }
        "theme" => {
            appearance.theme = normalize_theme(&string_value(incoming)?)?;
            *appearance_dirty = true;
            Ok(appearance.theme.clone())
        }
        "light_palette" => {
            appearance.light_palette = normalize_enum(&string_value(incoming)?, LIGHT_PALETTES)
                .ok_or_else(|| {
                    format!("light_palette must be one of {}", LIGHT_PALETTES.join(", "))
                })?;
            *appearance_dirty = true;
            Ok(appearance.light_palette.clone())
        }
        "dark_palette" => {
            appearance.dark_palette = normalize_enum(&string_value(incoming)?, DARK_PALETTES)
                .ok_or_else(|| {
                    format!("dark_palette must be one of {}", DARK_PALETTES.join(", "))
                })?;
            *appearance_dirty = true;
            Ok(appearance.dark_palette.clone())
        }
        "selection_popup_enabled" => {
            appearance.selection_popup_enabled = bool_value(incoming)?;
            *appearance_dirty = true;
            Ok(appearance.selection_popup_enabled.to_string())
        }
        "send_with_modifier" => {
            appearance.send_with_modifier = bool_value(incoming)?;
            *appearance_dirty = true;
            Ok(appearance.send_with_modifier.to_string())
        }
        "custom_css" => {
            appearance.custom_css = sanitize_custom_css(&string_value(incoming)?);
            *appearance_dirty = true;
            Ok(if appearance.custom_css.is_empty() {
                "(cleared)".into()
            } else {
                format!("{} bytes", appearance.custom_css.len())
            })
        }
        "locale" => {
            let locale = normalize_locale(&string_value(incoming)?)?;
            store
                .set_setting("locale", &locale)
                .await
                .map_err(|error| error.to_string())?;
            Ok(locale)
        }
        "max_iter" => {
            let current = store
                .get_setting("max_iter")
                .await
                .map_err(|error| error.to_string())?
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_MAX_ITER);
            let next = resolve_int(incoming, current, 0, i64::MAX)?;
            store
                .set_setting("max_iter", &next.to_string())
                .await
                .map_err(|error| error.to_string())?;
            Ok(next.to_string())
        }
        "auto_compact" => write_bool_setting(store, "auto_compact", incoming).await,
        "semantic_compact_on_model_switch" => {
            write_bool_setting(store, "semantic_compact_on_model_switch", incoming).await
        }
        "semantic_compact_idle_hours" => {
            let current = store
                .get_setting("semantic_compact_idle_hours")
                .await
                .map_err(|error| error.to_string())?
                .and_then(|value| value.parse().ok())
                .unwrap_or(24);
            let next = resolve_int(incoming, current, 0, 10_000)?;
            store
                .set_setting("semantic_compact_idle_hours", &next.to_string())
                .await
                .map_err(|error| error.to_string())?;
            Ok(next.to_string())
        }
        "auto_continue" => write_bool_setting(store, "auto_continue", incoming).await,
        "auto_continue_limit" => {
            let current = store
                .get_setting("auto_continue_limit")
                .await
                .map_err(|error| error.to_string())?
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_AUTO_CONTINUE_LIMIT as i64);
            let next = resolve_int(incoming, current, 1, 10_000)?;
            store
                .set_setting("auto_continue_limit", &next.to_string())
                .await
                .map_err(|error| error.to_string())?;
            Ok(next.to_string())
        }
        "follow_up_questions" => write_bool_setting(store, "follow_up_questions", incoming).await,
        "resume_last_session" => write_bool_setting(store, "resume_last_session", incoming).await,
        "decentralized_project_storage" => {
            write_bool_setting(store, "decentralized_project_storage", incoming).await
        }
        "notifications_enabled" => {
            write_bool_setting(store, "notifications_enabled", incoming).await
        }
        other => Err(format!("unhandled setting '{other}'")),
    }
}

async fn write_bool_setting(store: &Store, key: &str, incoming: &Value) -> Result<String, String> {
    let next = bool_value(incoming)?;
    store
        .set_setting(key, &next.to_string())
        .await
        .map_err(|error| error.to_string())?;
    Ok(next.to_string())
}

fn string_value(value: &Value) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Null => Ok(String::new()),
        _ => Err("expected a string, number, or boolean".into()),
    }
}

fn bool_value(value: &Value) -> Result<bool, String> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            other => Err(format!("expected a boolean (got '{other}')")),
        },
        Value::Number(number) if number.as_i64() == Some(1) => Ok(true),
        Value::Number(number) if number.as_i64() == Some(0) => Ok(false),
        _ => Err("expected a boolean".into()),
    }
}

fn resolve_int(value: &Value, current: i64, min: i64, max: i64) -> Result<i64, String> {
    let parsed = match value {
        Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| "expected an integer".to_string())?,
        Value::String(text) => {
            let text = text.trim();
            if let Some(rest) = text.strip_prefix('+') {
                current
                    + rest
                        .parse::<i64>()
                        .map_err(|_| format!("invalid relative value '{text}'"))?
            } else if text.starts_with('-') && text.len() > 1 && text.as_bytes()[1].is_ascii_digit()
            {
                current
                    + text
                        .parse::<i64>()
                        .map_err(|_| format!("invalid relative value '{text}'"))?
            } else {
                text.parse::<i64>()
                    .map_err(|_| format!("expected an integer (got '{text}')"))?
            }
        }
        _ => return Err("expected an integer or a +N/-N relative string".into()),
    };
    if parsed < min || parsed > max {
        return Err(format!("value {parsed} is outside {min}..={max}"));
    }
    Ok(parsed)
}

async fn storage_report(
    store: &Store,
    app_data: &Path,
    project_filter: &str,
) -> Result<(String, Value), String> {
    let projects = store
        .list_projects()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|(id, name, root, ..)| (id, name, PathBuf::from(root)))
        .filter(|(_, _, root)| !root.as_os_str().is_empty())
        .collect::<Vec<_>>();
    let usage = crate::settings_commands::collect_storage_usage(app_data.to_path_buf(), projects);
    Ok(render_storage(&usage, project_filter))
}

fn render_storage(usage: &Value, project_filter: &str) -> (String, Value) {
    let data_dir = usage
        .get("data_dir")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let total = usage
        .get("total_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let entries = usage
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let projects = usage
        .get("projects")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut lines = Vec::new();
    if project_filter == "*" {
        lines.push("# App storage".into());
        lines.push(format!("Data directory: `{data_dir}`"));
        lines.push(format!("Total: **{}**", format_bytes(total)));
        lines.push(String::new());
        lines.push("| Category | Size |".into());
        lines.push("| --- | ---: |".into());
        for entry in &entries {
            let key = entry.get("key").and_then(Value::as_str).unwrap_or("other");
            let bytes = entry.get("bytes").and_then(Value::as_u64).unwrap_or(0);
            lines.push(format!(
                "| {} | {} |",
                storage_label(key),
                format_bytes(bytes)
            ));
        }
        if !projects.is_empty() {
            lines.push(String::new());
            lines.push("## Projects".into());
            lines.push("| Project | Path | Size |".into());
            lines.push("| --- | --- | ---: |".into());
            for project in &projects {
                lines.push(project_row(project));
            }
        }
    } else {
        let project = projects.iter().find(|project| {
            project
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id == project_filter)
        });
        match project {
            Some(project) => {
                let name = project
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(project_filter);
                let path = project.get("path").and_then(Value::as_str).unwrap_or("");
                let bytes = project.get("bytes").and_then(Value::as_u64).unwrap_or(0);
                lines.push(format!("# Storage · {name}"));
                lines.push(format!("Workspace: `{path}`"));
                lines.push(format!("Project files: **{}**", format_bytes(bytes)));
                lines.push(String::new());
                lines.push(format!(
                    "App data (`{data_dir}`) totals **{}** across all workspaces (database, Python env, plugins).",
                    format_bytes(total)
                ));
                lines.push("| Category | Size |".into());
                lines.push("| --- | ---: |".into());
                for entry in &entries {
                    let key = entry.get("key").and_then(Value::as_str).unwrap_or("other");
                    let bytes = entry.get("bytes").and_then(Value::as_u64).unwrap_or(0);
                    lines.push(format!(
                        "| {} | {} |",
                        storage_label(key),
                        format_bytes(bytes)
                    ));
                }
            }
            None => {
                lines.push(format!(
                    "No project matched `{project_filter}`. Pass `*` to list every workspace."
                ));
                if !projects.is_empty() {
                    lines.push(String::new());
                    lines.push("| Project | Path | Size |".into());
                    lines.push("| --- | --- | ---: |".into());
                    for project in &projects {
                        lines.push(project_row(project));
                    }
                }
            }
        }
    }

    let payload = json!({
        "filter": project_filter,
        "data_dir": data_dir,
        "total_bytes": total,
        "entries": entries,
        "projects": projects,
    });
    (lines.join("\n"), payload)
}

fn project_row(project: &Value) -> String {
    let name = project.get("name").and_then(Value::as_str).unwrap_or("?");
    let path = project.get("path").and_then(Value::as_str).unwrap_or("");
    let bytes = project.get("bytes").and_then(Value::as_u64).unwrap_or(0);
    format!("| {name} | `{path}` | {} |", format_bytes(bytes))
}

fn storage_label(key: &str) -> &'static str {
    match key {
        "database" => "Database (sessions & tool results)",
        "python" => "Python environment",
        "plugins" => "Plugins",
        "workspace" => "Workspace files",
        _ => "Other",
    }
}

fn format_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else if n < 1024 * 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

#[tauri::command]
pub(crate) async fn get_appearance_prefs(
    state: State<'_, AppState>,
) -> Result<AppearancePrefsView, String> {
    Ok(load_appearance(&state.store).await)
}

#[tauri::command]
pub(crate) async fn set_appearance_prefs(
    state: State<'_, AppState>,
    prefs: AppearancePrefs,
) -> Result<AppearancePrefs, String> {
    save_appearance(&state.store, prefs).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

    async fn test_store() -> (Store, PathBuf, PathBuf) {
        let unique = format!(
            "wisp_configure_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        );
        let root = std::env::temp_dir().join(&unique);
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("wisp.sqlite");
        let store = Store::open(&db).await.unwrap();
        (store, root, db)
    }

    fn tool(store: Store, app_data: PathBuf) -> ConfigureTool {
        ConfigureTool::new(store, app_data, "proj-a")
    }

    #[tokio::test]
    async fn get_returns_catalog_and_specialists() {
        let (store, root, db) = test_store().await;
        let env = NoEnv(root.clone());
        let result = tool(store, root.clone())
            .run(&json!({"action": "get"}), &env)
            .await;
        assert!(result.success, "{}", result.content);
        assert!(
            result.content.contains("`ui_font_size`"),
            "{}",
            result.content
        );
        assert!(
            result.content.contains("`specialists`"),
            "{}",
            result.content
        );
        assert!(result.content.contains("reviewer"), "{}", result.content);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn project_storage_preference_roundtrips_and_rejects_invalid_values() {
        let (store, root, db) = test_store().await;
        let env = NoEnv(root.clone());
        let configure = tool(store.clone(), root.clone());
        assert!(!store.decentralized_project_storage().await.unwrap());
        for enabled in [true, false] {
            let result = configure
                .run(
                    &json!({"action": "set", "values": {"decentralized_project_storage": enabled}}),
                    &env,
                )
                .await;
            assert!(result.success, "{}", result.content);
            assert_eq!(
                store.decentralized_project_storage().await.unwrap(),
                enabled
            );
            assert_eq!(
                current_values(&store).await.unwrap()["decentralized_project_storage"],
                json!(enabled)
            );
        }
        let invalid = configure
            .run(
                &json!({"action": "set", "values": {"decentralized_project_storage": "invalid"}}),
                &env,
            )
            .await;
        assert!(!invalid.success);
        assert!(!store.decentralized_project_storage().await.unwrap());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn set_font_size_absolute_and_relative() {
        let (store, root, db) = test_store().await;
        let env = NoEnv(root.clone());
        let configure = tool(store.clone(), root.clone());

        let first = configure
            .run(
                &json!({"action": "set", "values": {"ui_font_size": 16}}),
                &env,
            )
            .await;
        assert!(first.success, "{}", first.content);
        assert!(
            first.content.contains("ui_font_size: 16"),
            "{}",
            first.content
        );
        assert_eq!(load_appearance(&store).await.prefs.ui_font_size, 16);

        let second = configure
            .run(
                &json!({"action": "set", "values": {"ui_font_size": "+2"}}),
                &env,
            )
            .await;
        assert!(second.success, "{}", second.content);
        assert_eq!(load_appearance(&store).await.prefs.ui_font_size, 18);

        let third = configure
            .run(
                &json!({"action": "set", "values": {"ui_font_size": "-4"}}),
                &env,
            )
            .await;
        assert!(third.success, "{}", third.content);
        assert_eq!(load_appearance(&store).await.prefs.ui_font_size, 14);

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn rejects_unknown_and_secret_keys() {
        let (store, root, db) = test_store().await;
        let env = NoEnv(root.clone());
        let configure = tool(store, root.clone());

        let unknown = configure
            .run(
                &json!({"action": "set", "values": {"not_a_real_setting": 1}}),
                &env,
            )
            .await;
        assert!(!unknown.success);
        assert!(
            unknown.content.contains("unknown setting"),
            "{}",
            unknown.content
        );

        let secret = configure
            .run(
                &json!({"action": "set", "values": {"api_key": "sk-test"}}),
                &env,
            )
            .await;
        assert!(!secret.success);
        assert!(
            secret.content.contains("not writable"),
            "{}",
            secret.content
        );

        let workspace = configure
            .run(
                &json!({"action": "set", "values": {"workspace_dir": "C:\\\\tmp"}}),
                &env,
            )
            .await;
        assert!(!workspace.success);

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn rejects_out_of_range_font() {
        let (store, root, db) = test_store().await;
        let env = NoEnv(root.clone());
        let result = tool(store, root.clone())
            .run(
                &json!({"action": "set", "values": {"ui_font_size": 99}}),
                &env,
            )
            .await;
        assert!(!result.success);
        assert!(result.content.contains("outside"), "{}", result.content);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn set_locale_and_theme() {
        let (store, root, db) = test_store().await;
        let env = NoEnv(root.clone());
        let configure = tool(store.clone(), root.clone());
        let result = configure
            .run(
                &json!({"action": "set", "values": {"locale": "中文", "theme": "Dark"}}),
                &env,
            )
            .await;
        assert!(result.success, "{}", result.content);
        assert_eq!(
            store.get_setting("locale").await.unwrap().as_deref(),
            Some("zh")
        );
        assert_eq!(load_appearance(&store).await.prefs.theme, "dark");
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn storage_reports_current_project() {
        let (store, root, db) = test_store().await;
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("data.bin"), [0u8; 2048]).unwrap();
        store
            .create_project("proj-a", "Alpha", workspace.to_string_lossy().as_ref())
            .await
            .unwrap();

        let env = NoEnv(root.clone());
        let result = tool(store, root.clone())
            .run(&json!({"action": "storage"}), &env)
            .await;
        assert!(result.success, "{}", result.content);
        assert!(result.content.contains("Alpha"), "{}", result.content);
        assert!(result.content.contains("2.0 KB"), "{}", result.content);
        assert!(result.content.contains("Workspace:"), "{}", result.content);

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(db);
    }

    #[test]
    fn appearance_prefs_tolerate_missing_custom_css() {
        let prefs: AppearancePrefs = serde_json::from_str(
            r#"{"theme":"dark","light_palette":"paper","dark_palette":"charcoal","ui_font_size":14,"code_font_size":12}"#,
        )
        .unwrap();
        assert_eq!(prefs.theme, "dark");
        assert!(prefs.custom_css.is_empty());
    }

    #[test]
    fn custom_css_strips_remote_loads_and_style_breakouts() {
        let dirty = r#"
:root { --md-lead-bar-width: 0; --md-lead-bar-pad: 0; }
@import url("https://evil.example/x.css");
.md table { background: url(https://evil.example/pixel.png); }
</style><script>alert(1)</script>
"#;
        let clean = sanitize_custom_css(dirty);
        assert!(clean.contains("--md-lead-bar-width: 0"));
        assert!(!clean.to_ascii_lowercase().contains("@import"));
        assert!(!clean.to_ascii_lowercase().contains("url("));
        assert!(!clean.to_ascii_lowercase().contains("</style"));
        assert!(!clean.to_ascii_lowercase().contains("<script"));
        assert!(!clean.to_ascii_lowercase().contains("https://evil.example"));
    }

    #[test]
    fn custom_css_caps_length() {
        let huge = "a".repeat(CUSTOM_CSS_MAX_BYTES + 80);
        assert_eq!(sanitize_custom_css(&huge).len(), CUSTOM_CSS_MAX_BYTES);
    }

    #[tokio::test]
    async fn set_custom_css_persists_sanitized_stylesheet() {
        let (store, root, db) = test_store().await;
        let env = NoEnv(root.clone());
        let configure = tool(store.clone(), root.clone());
        let result = configure
            .run(
                &json!({
                    "action": "set",
                    "values": {
                        "custom_css": ":root { --md-lead-bar-width: 0; }\n@import url(https://x);"
                    }
                }),
                &env,
            )
            .await;
        assert!(result.success, "{}", result.content);
        let saved = load_appearance(&store).await.prefs.custom_css;
        assert!(saved.contains("--md-lead-bar-width: 0"));
        assert!(!saved.to_ascii_lowercase().contains("@import"));
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(db);
    }

    #[test]
    fn relative_int_parser_handles_plus_and_minus() {
        assert_eq!(resolve_int(&json!("+2"), 14, 0, 30).unwrap(), 16);
        assert_eq!(resolve_int(&json!("-3"), 14, 0, 30).unwrap(), 11);
        assert_eq!(resolve_int(&json!(18), 14, 0, 30).unwrap(), 18);
        assert!(resolve_int(&json!(99), 14, 0, 30).is_err());
    }

    #[tokio::test]
    async fn preview_lists_set_keys() {
        let (store, root, db) = test_store().await;
        let configure = tool(store, root.clone());
        assert_eq!(
            Tool::preview(
                &configure,
                &json!({"action": "set", "values": {"theme": "dark", "ui_font_size": 16}}),
            ),
            "set theme, ui_font_size"
        );
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(db);
    }
}
