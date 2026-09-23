use super::*;

const MODEL_SWITCH_WARNING_DISABLED_KEY: &str = "wisp-model-switch-warning-disabled";

const PRIVACY_MODE_ACTIVE_KEY: &str = "wisp-privacy-mode-active";
const PRIVACY_MODE_PROJECTS_KEY: &str = "wisp-privacy-mode-projects";

pub(crate) fn load_privacy_mode() -> (bool, HashSet<String>) {
    let storage = web_sys::window().and_then(|window| window.local_storage().ok().flatten());
    let projects = storage
        .as_ref()
        .and_then(|storage| storage.get_item(PRIVACY_MODE_PROJECTS_KEY).ok().flatten())
        .and_then(|value| serde_json::from_str::<HashSet<String>>(&value).ok())
        .unwrap_or_default();
    let active = !projects.is_empty()
        && storage
            .and_then(|storage| storage.get_item(PRIVACY_MODE_ACTIVE_KEY).ok().flatten())
            .is_some_and(|value| value == "1");
    (active, projects)
}

pub(crate) fn privacy_host_args(active: bool, projects: &HashSet<String>) -> serde_json::Value {
    let mut project_ids = projects.iter().map(String::as_str).collect::<Vec<_>>();
    project_ids.sort_unstable();
    serde_json::json!({
        "active": active && !project_ids.is_empty(),
        "project_ids": project_ids,
    })
}

pub(crate) fn mirror_privacy_mode(active: bool, projects: &HashSet<String>) {
    let args = privacy_host_args(active, projects);
    leptos::spawn_local(async move {
        if let Ok(args) = serde_wasm_bindgen::to_value(&args) {
            let _ = crate::bindings::invoke("set_privacy_mode", args).await;
        }
    });
}

pub(crate) fn save_privacy_mode(active: bool, projects: &HashSet<String>) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        if let Ok(value) = serde_json::to_string(projects) {
            let _ = storage.set_item(PRIVACY_MODE_PROJECTS_KEY, &value);
        }
        let _ = if active && !projects.is_empty() {
            storage.set_item(PRIVACY_MODE_ACTIVE_KEY, "1")
        } else {
            storage.remove_item(PRIVACY_MODE_ACTIVE_KEY)
        };
    }
    mirror_privacy_mode(active, projects);
}

pub(crate) fn model_switch_warning_disabled() -> bool {
    web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| {
            storage
                .get_item(MODEL_SWITCH_WARNING_DISABLED_KEY)
                .ok()
                .flatten()
        })
        .is_some_and(|value| value == "1")
}

pub(crate) fn disable_model_switch_warning() {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.set_item(MODEL_SWITCH_WARNING_DISABLED_KEY, "1");
    }
}

const SELECTION_POPUP_DISABLED_KEY: &str = "selectionPopupDisabled";

const SEND_WITH_MODIFIER_KEY: &str = "wisp-send-with-modifier";

pub(crate) fn load_selection_popup_enabled() -> bool {
    !web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(SELECTION_POPUP_DISABLED_KEY).ok().flatten())
        .is_some_and(|v| v == "1")
}

pub(crate) fn save_selection_popup_enabled(enabled: bool) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = if enabled {
            s.remove_item(SELECTION_POPUP_DISABLED_KEY)
        } else {
            s.set_item(SELECTION_POPUP_DISABLED_KEY, "1")
        };
    }
}

pub(crate) fn load_send_with_modifier() -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(SEND_WITH_MODIFIER_KEY).ok().flatten())
        .is_some_and(|v| v == "1")
}

pub(crate) fn save_send_with_modifier(enabled: bool) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = if enabled {
            s.set_item(SEND_WITH_MODIFIER_KEY, "1")
        } else {
            s.remove_item(SEND_WITH_MODIFIER_KEY)
        };
    }
}

pub(crate) fn load_theme_mode() -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(THEME_STORAGE_KEY).ok().flatten())
        .filter(|mode| matches!(mode.as_str(), "light" | "dark" | "system"))
        .unwrap_or_else(|| "system".into())
}

pub(crate) fn apply_theme_mode(mode: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Some(root) = window.document().and_then(|d| d.document_element()) {
        let _ = root.set_attribute("data-theme", mode);
    }
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item(THEME_STORAGE_KEY, mode);
    }
}

fn load_palette_mode(key: &str, fallback: &str, valid: &[&str]) -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(key).ok().flatten())
        .filter(|palette| valid.contains(&palette.as_str()))
        .unwrap_or_else(|| fallback.into())
}

pub(crate) fn load_light_palette() -> String {
    load_palette_mode(
        "wisp-light-palette",
        "paper",
        &["paper", "codex", "github", "catppuccin", "everforest"],
    )
}

pub(crate) fn load_dark_palette() -> String {
    load_palette_mode(
        "wisp-dark-palette",
        "charcoal",
        &["charcoal", "codex", "github", "catppuccin", "gruvbox"],
    )
}

pub(crate) fn apply_palette_modes(light: &str, dark: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Some(root) = window.document().and_then(|d| d.document_element()) {
        let _ = root.set_attribute("data-light-palette", light);
        let _ = root.set_attribute("data-dark-palette", dark);
    }
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item("wisp-light-palette", light);
        let _ = storage.set_item("wisp-dark-palette", dark);
    }
}

/// Load a small persisted view preference (sidebar sort/group), constrained to
/// a known set of values so a stale/garbage localStorage entry can't wedge the UI.
pub(crate) fn load_view_pref(key: &str, fallback: &str, valid: &[&str]) -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(key).ok().flatten())
        .filter(|v| valid.contains(&v.as_str()))
        .unwrap_or_else(|| fallback.into())
}

pub(crate) fn save_view_pref(key: &str, value: &str) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(key, value);
    }
}

fn load_font_size(key: &str, fallback: u16, min: u16, max: u16) -> u16 {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(key).ok().flatten())
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(fallback)
        .clamp(min, max)
}

pub(crate) fn load_ui_font_size() -> u16 {
    load_font_size("wisp-ui-font-size", 14, 0, 30)
}

pub(crate) fn load_code_font_size() -> u16 {
    load_font_size("wisp-code-font-size", 12, 0, 30)
}

/// A user-chosen font family is substituted into the `--font-ui` /
/// `--font-mono` stacks via `var(--font-user-*)`, so strip anything that could
/// break out of the custom-property value.
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

fn load_font_family(key: &str) -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(key).ok().flatten())
        .map(|value| sanitize_font_family(&value))
        .unwrap_or_default()
}

pub(crate) fn load_ui_font_family() -> String {
    load_font_family("wisp-font-ui")
}

pub(crate) fn load_code_font_family() -> String {
    load_font_family("wisp-font-mono")
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AppPrefsPatch {
    pub theme: Option<String>,
    pub light_palette: Option<String>,
    pub dark_palette: Option<String>,
    pub ui_font_size: Option<u16>,
    pub code_font_size: Option<u16>,
    pub ui_font_family: Option<String>,
    pub code_font_family: Option<String>,
    pub selection_popup_enabled: Option<bool>,
    pub send_with_modifier: Option<bool>,
    pub custom_css: Option<String>,
    pub locale: Option<String>,
    pub max_iter: Option<i64>,
    pub auto_compact: Option<bool>,
    pub semantic_compact_on_model_switch: Option<bool>,
    pub semantic_compact_idle_hours: Option<u64>,
    pub auto_continue: Option<bool>,
    pub auto_continue_limit: Option<u64>,
    pub follow_up_questions: Option<bool>,
    pub resume_last_session: Option<bool>,
    pub notifications_enabled: Option<bool>,
}

pub(crate) fn parse_app_prefs_payload(payload: &serde_json::Value) -> AppPrefsPatch {
    AppPrefsPatch {
        theme: payload
            .get("theme")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        light_palette: payload
            .get("light_palette")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        dark_palette: payload
            .get("dark_palette")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        ui_font_size: payload
            .get("ui_font_size")
            .and_then(|value| value.as_u64())
            .map(|value| value.min(30) as u16),
        code_font_size: payload
            .get("code_font_size")
            .and_then(|value| value.as_u64())
            .map(|value| value.min(30) as u16),
        ui_font_family: payload
            .get("ui_font_family")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        code_font_family: payload
            .get("code_font_family")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        selection_popup_enabled: payload.get("selection_popup_enabled").and_then(|value| {
            value.as_bool().or_else(|| {
                value.as_str().and_then(|text| match text {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                })
            })
        }),
        send_with_modifier: payload.get("send_with_modifier").and_then(|value| {
            value.as_bool().or_else(|| {
                value.as_str().and_then(|text| match text {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                })
            })
        }),
        custom_css: payload
            .get("custom_css")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        locale: payload
            .get("locale")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        max_iter: payload.get("max_iter").and_then(|value| value.as_i64()),
        auto_compact: payload
            .get("auto_compact")
            .and_then(|value| value.as_bool()),
        semantic_compact_on_model_switch: payload
            .get("semantic_compact_on_model_switch")
            .and_then(|value| value.as_bool()),
        semantic_compact_idle_hours: payload
            .get("semantic_compact_idle_hours")
            .and_then(|value| value.as_u64()),
        auto_continue: payload
            .get("auto_continue")
            .and_then(|value| value.as_bool()),
        auto_continue_limit: payload
            .get("auto_continue_limit")
            .and_then(|value| value.as_u64()),
        follow_up_questions: payload
            .get("follow_up_questions")
            .and_then(|value| value.as_bool()),
        resume_last_session: payload
            .get("resume_last_session")
            .and_then(|value| value.as_bool()),
        notifications_enabled: payload
            .get("notifications_enabled")
            .and_then(|value| value.as_bool()),
    }
}

pub(crate) fn apply_prefs_patch(
    patch: &AppPrefsPatch,
    theme_mode: RwSignal<String>,
    light_palette: RwSignal<String>,
    dark_palette: RwSignal<String>,
    ui_font_size: RwSignal<u16>,
    code_font_size: RwSignal<u16>,
    ui_font_family: RwSignal<String>,
    code_font_family: RwSignal<String>,
    selection_popup_enabled: RwSignal<bool>,
    send_with_modifier: RwSignal<bool>,
    custom_css: RwSignal<String>,
    locale: RwSignal<Locale>,
    settings: RwSignal<Settings>,
) {
    if let Some(theme) = &patch.theme {
        if matches!(theme.as_str(), "light" | "dark" | "system") {
            theme_mode.set(theme.clone());
        }
    }
    if let Some(palette) = &patch.light_palette {
        light_palette.set(palette.clone());
    }
    if let Some(palette) = &patch.dark_palette {
        dark_palette.set(palette.clone());
    }
    if let Some(size) = patch.ui_font_size {
        ui_font_size.set(size);
    }
    if let Some(size) = patch.code_font_size {
        code_font_size.set(size);
    }
    if let Some(family) = &patch.ui_font_family {
        ui_font_family.set(family.clone());
    }
    if let Some(family) = &patch.code_font_family {
        code_font_family.set(family.clone());
    }
    if let Some(enabled) = patch.selection_popup_enabled {
        selection_popup_enabled.set(enabled);
    }
    if let Some(enabled) = patch.send_with_modifier {
        send_with_modifier.set(enabled);
    }
    if let Some(css) = &patch.custom_css {
        custom_css.set(sanitize_custom_css(css));
    }
    if let Some(code) = &patch.locale {
        let loc = Locale::from_code(code);
        locale.set(loc);
        crate::i18n::set_document_lang(loc);
    }
    settings.update(|cfg| {
        if let Some(code) = &patch.locale {
            cfg.locale = Locale::from_code(code).code().into();
        }
        if let Some(value) = patch.max_iter {
            cfg.max_iter = value;
        }
        if let Some(value) = patch.auto_compact {
            cfg.auto_compact = value;
        }
        if let Some(value) = patch.semantic_compact_on_model_switch {
            cfg.semantic_compact_on_model_switch = value;
        }
        if let Some(value) = patch.semantic_compact_idle_hours {
            cfg.semantic_compact_idle_hours = value;
        }
        if let Some(value) = patch.auto_continue {
            cfg.auto_continue = value;
        }
        if let Some(value) = patch.auto_continue_limit {
            cfg.auto_continue_limit = value;
        }
        if let Some(value) = patch.follow_up_questions {
            cfg.follow_up_questions = value;
        }
        if let Some(value) = patch.resume_last_session {
            cfg.resume_last_session = value;
        }
        if let Some(value) = patch.notifications_enabled {
            cfg.notifications_enabled = value;
        }
    });
}

/// Session list timestamps below this are ranking stubs, not real activity.
pub(crate) const SEMANTIC_COMPACT_IDLE_TS_MIN_MS: i64 = 1_000_000_000_000;

pub(crate) fn should_prompt_semantic_compact_idle(
    idle_hours: u64,
    last_activity_ms: i64,
    now_ms: i64,
    has_transcript: bool,
) -> bool {
    idle_hours > 0
        && has_transcript
        && last_activity_ms >= SEMANTIC_COMPACT_IDLE_TS_MIN_MS
        && now_ms.saturating_sub(last_activity_ms) >= (idle_hours as i64).saturating_mul(3_600_000)
}

pub(crate) fn apply_font_prefs(ui_size: u16, code_size: u16, ui_family: &str, code_family: &str) {
    let ui_family = sanitize_font_family(ui_family);
    let code_family = sanitize_font_family(code_family);
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Some(root) = window.document().and_then(|d| d.document_element()) {
        let mut style = format!("--ui-font-size:{ui_size}px;--code-font-size:{code_size}px");
        if !ui_family.is_empty() {
            style.push_str(&format!(";--font-user-ui:{ui_family}"));
        }
        if !code_family.is_empty() {
            style.push_str(&format!(";--font-user-mono:{code_family}"));
        }
        let _ = root.set_attribute("style", &style);
    }
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item("wisp-ui-font-size", &ui_size.to_string());
        let _ = storage.set_item("wisp-code-font-size", &code_size.to_string());
        for (key, value) in [("wisp-font-ui", ui_family), ("wisp-font-mono", code_family)] {
            let _ = if value.is_empty() {
                storage.remove_item(key)
            } else {
                storage.set_item(key, &value)
            };
        }
    }
}

const CUSTOM_CSS_STYLE_ID: &str = "wisp-custom-theme";
const CUSTOM_CSS_STORAGE_KEY: &str = "wisp-custom-css";
const CUSTOM_CSS_MAX_BYTES: usize = 64_000;

pub(crate) fn load_custom_css() -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(CUSTOM_CSS_STORAGE_KEY).ok().flatten())
        .map(|value| sanitize_custom_css(&value))
        .unwrap_or_default()
}

pub(crate) fn apply_custom_css(css: &str) {
    let css = sanitize_custom_css(css);
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let style_el = match document.get_element_by_id(CUSTOM_CSS_STYLE_ID) {
        Some(el) => el,
        None => {
            let Ok(el) = document.create_element("style") else {
                return;
            };
            el.set_id(CUSTOM_CSS_STYLE_ID);
            if let Ok(Some(head)) = document.query_selector("head") {
                let _ = head.append_child(&el);
            } else if let Some(root) = document.document_element() {
                let _ = root.append_child(&el);
            }
            el
        }
    };
    style_el.set_text_content(Some(&css));
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = if css.is_empty() {
            storage.remove_item(CUSTOM_CSS_STORAGE_KEY)
        } else {
            storage.set_item(CUSTOM_CSS_STORAGE_KEY, &css)
        };
    }
}

pub(crate) fn import_custom_css_from_input(ev: &web_sys::Event, custom_css: RwSignal<String>) {
    let Some(input) = ev
        .target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
    else {
        return;
    };
    let Some(file) = input.files().and_then(|files| files.get(0)) else {
        return;
    };
    let blob: web_sys::Blob = file.unchecked_into();
    let promise = blob.text();
    let _ = input.set_value("");
    spawn_local(async move {
        if let Ok(value) = wasm_bindgen_futures::JsFuture::from(promise).await {
            if let Some(text) = value.as_string() {
                custom_css.set(sanitize_custom_css(&text));
            }
        }
    });
}

/// Keep in sync with `src-tauri/src/configure.rs`.
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

pub(crate) const COMPOSER_H_DEFAULT: f64 = 220.0;

pub(crate) const COMPOSER_H_MIN: f64 = 80.0;

pub(crate) const COMPOSER_H_MAX: f64 = 400.0;

pub(crate) const COMPOSER_H_KEY: &str = "composerHeight";

pub(crate) const COMPOSER_H_SAVED_KEY: &str = "composerHeightCustom";

pub(crate) const SIDEBAR_W_DEFAULT: f64 = 248.0;

pub(crate) const SIDEBAR_W_MIN: f64 = 200.0;

pub(crate) const SIDEBAR_W_MAX: f64 = 520.0;

pub(crate) const SIDEBAR_W_KEY: &str = "sidebarWidth";

pub(crate) fn load_composer_h() -> f64 {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(COMPOSER_H_KEY).ok().flatten())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(COMPOSER_H_DEFAULT)
        .clamp(COMPOSER_H_MIN, COMPOSER_H_MAX)
}

pub(crate) fn composer_h_custom() -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(COMPOSER_H_SAVED_KEY).ok().flatten())
        .is_some_and(|v| v == "1")
}

pub(crate) fn save_composer_h(h: f64) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(COMPOSER_H_KEY, &h.to_string());
        let _ = s.set_item(COMPOSER_H_SAVED_KEY, "1");
    }
}

pub(crate) fn load_sidebar_w() -> f64 {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(SIDEBAR_W_KEY).ok().flatten())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(SIDEBAR_W_DEFAULT)
        .clamp(SIDEBAR_W_MIN, SIDEBAR_W_MAX)
}

pub(crate) fn save_sidebar_w(w: f64) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(SIDEBAR_W_KEY, &w.to_string());
    }
}

/// Docked sits in the composer column. Floating is a free window the user
/// dragged off the dock. Last mode is in-memory only: a restart always
/// reopens docked, while saved geometry is reused the next time it undocks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ContextUsageMode {
    #[default]
    Docked,
    Floating,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ContextUsageGeom {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub(crate) const CONTEXT_USAGE_MIN_W: f64 = 320.0;

pub(crate) const CONTEXT_USAGE_MIN_H: f64 = 220.0;

pub(crate) const CONTEXT_USAGE_MARGIN: f64 = 8.0;

pub(crate) const CONTEXT_USAGE_DEFAULT_W: f64 = 420.0;

pub(crate) const CONTEXT_USAGE_DEFAULT_H: f64 = 360.0;

const CONTEXT_USAGE_X_KEY: &str = "wisp-context-usage-x";
const CONTEXT_USAGE_Y_KEY: &str = "wisp-context-usage-y";
const CONTEXT_USAGE_W_KEY: &str = "wisp-context-usage-w";
const CONTEXT_USAGE_H_KEY: &str = "wisp-context-usage-h";

pub(crate) fn clamp_context_usage_geom(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    viewport_w: f64,
    viewport_h: f64,
) -> ContextUsageGeom {
    let max_w = (viewport_w - 2.0 * CONTEXT_USAGE_MARGIN).max(1.0);
    let max_h = (viewport_h - 2.0 * CONTEXT_USAGE_MARGIN).max(1.0);
    let w = w.clamp(CONTEXT_USAGE_MIN_W.min(max_w), max_w);
    let h = h.clamp(CONTEXT_USAGE_MIN_H.min(max_h), max_h);
    let max_x = (viewport_w - w).max(0.0);
    let max_y = (viewport_h - h).max(0.0);
    ContextUsageGeom {
        x: x.clamp(0.0, max_x),
        y: y.clamp(0.0, max_y),
        w,
        h,
    }
}

pub(crate) fn viewport_size() -> (f64, f64) {
    web_sys::window()
        .map(|window| {
            let width = window
                .inner_width()
                .ok()
                .and_then(|value| value.as_f64())
                .unwrap_or(1280.0);
            let height = window
                .inner_height()
                .ok()
                .and_then(|value| value.as_f64())
                .unwrap_or(800.0);
            (width, height)
        })
        .unwrap_or((1280.0, 800.0))
}

pub(crate) fn load_context_usage_geom() -> Option<ContextUsageGeom> {
    let storage = web_sys::window()?.local_storage().ok().flatten()?;
    let x = storage
        .get_item(CONTEXT_USAGE_X_KEY)
        .ok()
        .flatten()?
        .parse()
        .ok()?;
    let y = storage
        .get_item(CONTEXT_USAGE_Y_KEY)
        .ok()
        .flatten()?
        .parse()
        .ok()?;
    let w = storage
        .get_item(CONTEXT_USAGE_W_KEY)
        .ok()
        .flatten()?
        .parse()
        .ok()?;
    let h = storage
        .get_item(CONTEXT_USAGE_H_KEY)
        .ok()
        .flatten()?
        .parse()
        .ok()?;
    let (viewport_w, viewport_h) = viewport_size();
    Some(clamp_context_usage_geom(x, y, w, h, viewport_w, viewport_h))
}

#[cfg(test)]
mod tests {
    use super::privacy_host_args;
    use std::collections::HashSet;

    #[test]
    fn privacy_host_args_match_the_stored_record() {
        let mut projects = HashSet::new();
        projects.insert("hidden".into());
        projects.insert("research-1".into());
        let args = privacy_host_args(true, &projects);
        assert_eq!(args["active"], true);
        assert_eq!(args["project_ids"][0], "hidden");
        assert_eq!(args["project_ids"][1], "research-1");
        let off = privacy_host_args(false, &projects);
        assert_eq!(off["active"], false);
        assert_eq!(off["project_ids"].as_array().unwrap().len(), 2);
        let empty = privacy_host_args(true, &HashSet::new());
        assert_eq!(empty["active"], false);
        assert!(empty["project_ids"].as_array().unwrap().is_empty());
    }
}

pub(crate) fn save_context_usage_geom(geom: ContextUsageGeom) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.set_item(CONTEXT_USAGE_X_KEY, &geom.x.to_string());
        let _ = storage.set_item(CONTEXT_USAGE_Y_KEY, &geom.y.to_string());
        let _ = storage.set_item(CONTEXT_USAGE_W_KEY, &geom.w.to_string());
        let _ = storage.set_item(CONTEXT_USAGE_H_KEY, &geom.h.to_string());
    }
}

#[cfg(test)]
mod context_usage_geom_tests {
    use super::{
        clamp_context_usage_geom, CONTEXT_USAGE_MARGIN, CONTEXT_USAGE_MIN_H, CONTEXT_USAGE_MIN_W,
    };

    #[test]
    fn clamp_keeps_the_panel_inside_the_viewport() {
        let geom = clamp_context_usage_geom(2000.0, 1500.0, 480.0, 300.0, 800.0, 600.0);
        assert_eq!(geom.w, 480.0);
        assert_eq!(geom.h, 300.0);
        assert_eq!(geom.x, 800.0 - 480.0);
        assert_eq!(geom.y, 600.0 - 300.0);
    }

    #[test]
    fn clamp_enforces_minimums_unless_the_window_is_smaller() {
        let geom = clamp_context_usage_geom(10.0, 10.0, 100.0, 80.0, 1280.0, 800.0);
        assert_eq!(geom.w, CONTEXT_USAGE_MIN_W);
        assert_eq!(geom.h, CONTEXT_USAGE_MIN_H);
        assert_eq!(geom.x, 10.0);
        assert_eq!(geom.y, 10.0);
    }

    #[test]
    fn clamp_shrinks_below_minimums_on_a_tiny_window() {
        let geom = clamp_context_usage_geom(-40.0, -20.0, 500.0, 400.0, 280.0, 200.0);
        let max_w = 280.0 - 2.0 * CONTEXT_USAGE_MARGIN;
        let max_h = 200.0 - 2.0 * CONTEXT_USAGE_MARGIN;
        assert_eq!(geom.w, max_w);
        assert_eq!(geom.h, max_h);
        assert_eq!(geom.x, 0.0);
        assert_eq!(geom.y, 0.0);
    }
}

#[cfg(test)]
mod app_prefs_payload_tests {
    use super::parse_app_prefs_payload;

    #[test]
    fn parses_font_and_theme_from_configure_payload() {
        let payload = serde_json::json!({
            "ui_font_size": 18,
            "theme": "dark",
            "locale": "zh",
            "auto_compact": false
        });
        let patch = parse_app_prefs_payload(&payload);
        assert_eq!(patch.ui_font_size, Some(18));
        assert_eq!(patch.theme.as_deref(), Some("dark"));
        assert_eq!(patch.locale.as_deref(), Some("zh"));
        assert_eq!(patch.auto_compact, Some(false));
        assert_eq!(patch.code_font_size, None);
        assert_eq!(patch.custom_css, None);
    }

    #[test]
    fn parses_custom_css_from_configure_payload() {
        let payload = serde_json::json!({
            "custom_css": ":root { --md-lead-bar-width: 0; }"
        });
        let patch = parse_app_prefs_payload(&payload);
        assert_eq!(
            patch.custom_css.as_deref(),
            Some(":root { --md-lead-bar-width: 0; }")
        );
    }
}

#[cfg(test)]
mod semantic_compact_idle_tests {
    use super::{should_prompt_semantic_compact_idle, SEMANTIC_COMPACT_IDLE_TS_MIN_MS};

    #[test]
    fn idle_prompt_requires_a_real_timestamp_and_transcript() {
        let now = 1_700_000_000_000;
        assert!(!should_prompt_semantic_compact_idle(24, 2000, now, true));
        assert!(!should_prompt_semantic_compact_idle(
            24,
            SEMANTIC_COMPACT_IDLE_TS_MIN_MS,
            now,
            false
        ));
        assert!(!should_prompt_semantic_compact_idle(
            0,
            now - 48 * 3_600_000,
            now,
            true
        ));
        assert!(!should_prompt_semantic_compact_idle(
            24,
            now - 12 * 3_600_000,
            now,
            true
        ));
        assert!(should_prompt_semantic_compact_idle(
            24,
            now - 25 * 3_600_000,
            now,
            true
        ));
    }
}
