use super::{
    HOME_SEARCH_ARTIFACT_LIMIT, HOME_SEARCH_PROJECT_LIMIT, HOME_SEARCH_SESSION_LIMIT,
    THEME_STORAGE_KEY,
};
use crate::bindings::{
    attach_cropped_region, crop_region_to_upload, invoke, invoke_checked, mount_preview,
    open_external_url, schedule_highlight, upload_files, upload_input_files, upload_pasted_images,
};
use crate::dto::*;
use crate::i18n::{localize_backend, t, tf, use_locale, Locale};
use crate::text::{
    code_lang, decode_href, dom_value, event_target_value, extract_href_from_tag, fasta_seq_count,
    fenced_blocks, file_kind, format_bytes, format_duration_ms, html_escape, ime_composing,
    is_external_href, is_separator, is_table_row, md_inline_to_html, md_to_html, next_artifact_id,
    normalize_path, opens_in_system_browser, parent_path, parse_csv_line, parse_notebook,
    pretty_json, provider_defaults, provider_value, split_row, tool_lang, unique_dom_id,
    user_message_presentation, NbOutput, Notebook,
};
use leptos::{ev, window_event_listener, *};
use serde_wasm_bindgen::to_value;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

const MODEL_SWITCH_WARNING_DISABLED_KEY: &str = "wisp-model-switch-warning-disabled";
const SSH_RETRY_STOPPED_MARKER: &str = "ssh automatic retry stopped";

thread_local! {
    static RUN_REFRESH_INITIALIZED: Cell<bool> = const { Cell::new(false) };
    static SSH_RETRY_TOASTED_RUNS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

pub(super) fn model_switch_warning_disabled() -> bool {
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

pub(super) fn disable_model_switch_warning() {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.set_item(MODEL_SWITCH_WARNING_DISABLED_KEY, "1");
    }
}

pub(super) fn load_theme_mode() -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(THEME_STORAGE_KEY).ok().flatten())
        .filter(|mode| matches!(mode.as_str(), "light" | "dark" | "system"))
        .unwrap_or_else(|| "system".into())
}

pub(super) fn apply_theme_mode(mode: &str) {
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

pub(super) fn load_light_palette() -> String {
    load_palette_mode(
        "wisp-light-palette",
        "paper",
        &["paper", "codex", "github", "catppuccin", "everforest"],
    )
}

pub(super) fn load_dark_palette() -> String {
    load_palette_mode(
        "wisp-dark-palette",
        "charcoal",
        &["charcoal", "codex", "github", "catppuccin", "gruvbox"],
    )
}

pub(super) fn apply_palette_modes(light: &str, dark: &str) {
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

fn load_font_size(key: &str, fallback: u16, min: u16, max: u16) -> u16 {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(key).ok().flatten())
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(fallback)
        .clamp(min, max)
}

pub(super) fn load_ui_font_size() -> u16 {
    load_font_size("wisp-ui-font-size", 14, 12, 18)
}

pub(super) fn load_code_font_size() -> u16 {
    load_font_size("wisp-code-font-size", 12, 10, 18)
}

pub(super) fn apply_font_sizes(ui_size: u16, code_size: u16) {
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Some(root) = window.document().and_then(|d| d.document_element()) {
        let style = format!("--ui-font-size:{ui_size}px;--code-font-size:{code_size}px",);
        let _ = root.set_attribute("style", &style);
    }
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item("wisp-ui-font-size", &ui_size.to_string());
        let _ = storage.set_item("wisp-code-font-size", &code_size.to_string());
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ComposerSendAction {
    Normal,
    BranchNew,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ComposerPickerMode {
    Artifact,
    Session,
    Skill,
}

#[derive(Clone, PartialEq)]
pub(super) enum ComposerPickerItem {
    Artifact(ArtifactInfo),
    Session(SessionSearchInfo),
    Skill(SkillRow),
    Context {
        id: String,
        label: String,
    },
    Runtime {
        context_id: String,
        context_label: String,
        language: String,
    },
}

#[derive(Clone)]
pub(super) enum ComposerReferenceChip {
    Artifact {
        id: String,
        name: String,
    },
    Session {
        id: String,
        title: String,
        project_name: String,
    },
    Skill {
        name: String,
    },
    Context {
        id: String,
        label: String,
    },
    Runtime {
        context_id: String,
        context_label: String,
        language: String,
    },
}

/// A passage attached from a preview. Unlike a plain blockquote, a source-aware
/// quote keeps the workspace path so the agent can act on "change this" instead
/// of treating the selection as an anonymous code sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ComposerQuote {
    pub(super) text: String,
    pub(super) source: Option<String>,
}

impl ComposerQuote {
    pub(super) fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            source: None,
        }
    }

    pub(super) fn from_selection(text: impl Into<String>, source: Option<String>) -> Self {
        Self {
            text: text.into(),
            source,
        }
    }

    pub(super) fn workspace_source(&self) -> Option<&str> {
        self.source.as_deref().filter(|source| {
            !source.starts_with("artifact:")
                && !source.starts_with("artifact-version:")
                && remote_file_path(source).is_none()
                && matches!(
                    file_kind(source),
                    Some(
                        "code" | "text" | "json" | "markdown" | "csv" | "html" | "fasta" | "smiles"
                    )
                )
        })
    }
}

#[derive(Clone, PartialEq)]
pub(super) enum CommandPaletteItem {
    Project(ProjectSummary),
    Artifact(ArtifactInfo),
    Session(SessionSearchInfo),
    Command(&'static str),
}

#[derive(Clone, PartialEq)]
pub(super) struct CommandAction {
    pub(super) id: &'static str,
    pub(super) icon: &'static str,
    pub(super) title: String,
    pub(super) group: String,
    pub(super) shortcut: &'static str,
}

impl ComposerReferenceChip {
    pub(super) fn key(&self) -> String {
        match self {
            Self::Artifact { id, .. } => format!("artifact:{id}"),
            Self::Session { id, .. } => format!("session:{id}"),
            Self::Skill { name } => format!("skill:{name}"),
            Self::Context { id, .. } => format!("context:{id}"),
            Self::Runtime {
                context_id,
                language,
                ..
            } => format!("runtime:{context_id}:{language}"),
        }
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Artifact { name, .. } | Self::Skill { name } => name.clone(),
            Self::Session {
                title,
                project_name,
                ..
            } => format!("{project_name} / {title}"),
            Self::Context { label, .. } => label.clone(),
            Self::Runtime {
                context_label,
                language,
                ..
            } => format!("{} · {context_label}", language_display(language)),
        }
    }

    pub(super) fn kind(&self) -> &'static str {
        match self {
            Self::Artifact { .. } => "artifact",
            Self::Session { .. } => "session",
            Self::Skill { .. } => "skill",
            Self::Context { .. } => "context",
            Self::Runtime { .. } => "runtime",
        }
    }

    pub(super) fn arg(&self) -> ComposerReferenceArg {
        match self {
            Self::Artifact { id, .. } => ComposerReferenceArg::Artifact { id: id.clone() },
            Self::Session { id, .. } => ComposerReferenceArg::Session { id: id.clone() },
            Self::Skill { name } => ComposerReferenceArg::Skill { name: name.clone() },
            Self::Context { id, .. } => ComposerReferenceArg::Context { id: id.clone() },
            Self::Runtime {
                context_id,
                language,
                ..
            } => ComposerReferenceArg::Runtime {
                context_id: context_id.clone(),
                language: language.clone(),
            },
        }
    }
}

#[derive(Clone)]
pub(super) enum FolderModal {
    Create,
    Rename(String),
}

#[derive(Clone)]
pub(super) enum FileEntryModal {
    CreateFile,
    CreateDirectory,
    Rename { path: String, is_dir: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionTransferMode {
    Copy,
    Move,
}

impl SessionTransferMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
        }
    }
}

#[derive(Clone)]
pub(super) struct SessionTransfer {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) mode: SessionTransferMode,
    pub(super) target_project_id: String,
}

#[derive(Clone)]
pub(super) enum UiConfirm {
    DeleteFolder(String),
    DeleteSession(String),
    DeleteFileEntry { path: String, is_dir: bool },
}

#[derive(Clone)]
pub(super) enum UpdateCheckModal {
    Checking,
    Available {
        version: String,
        release_url: String,
    },
    UpToDate {
        version: String,
    },
    Failed {
        message: String,
    },
}

/// First open vs after a failed probe (failed phase must not keep probing).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SshCheckPhase {
    /// Never probed (or status unknown): one intentional probe is allowed.
    NeedConfirm,
    /// Probe already failed: show diagnosis and fix actions, not re-probe as primary.
    Failed,
}

/// Modal asking the user to confirm SSH reachability before the agent can use a host.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct SshConnectivityModal {
    pub(super) context_id: String,
    pub(super) label: String,
    pub(super) detail: String,
    /// When true, a successful probe enables this context for the current session.
    pub(super) enable_after_probe: bool,
    pub(super) phase: SshCheckPhase,
}

impl SshConnectivityModal {
    pub(super) fn need_confirm(
        context_id: String,
        label: String,
        detail: String,
        enable_after_probe: bool,
    ) -> Self {
        Self {
            context_id,
            label,
            detail,
            enable_after_probe,
            phase: SshCheckPhase::NeedConfirm,
        }
    }

    pub(super) fn failed(
        context_id: String,
        label: String,
        detail: String,
        enable_after_probe: bool,
    ) -> Self {
        Self {
            context_id,
            label,
            detail,
            enable_after_probe,
            phase: SshCheckPhase::Failed,
        }
    }

    /// Prefer Failed when we already know the last probe error.
    pub(super) fn from_gap(
        context_id: String,
        label: String,
        detail: String,
        enable_after_probe: bool,
    ) -> Self {
        let phase = if detail == "not probed yet" {
            SshCheckPhase::NeedConfirm
        } else {
            SshCheckPhase::Failed
        };
        Self {
            context_id,
            label,
            detail,
            enable_after_probe,
            phase,
        }
    }
}

/// Classified SSH failure for diagnosis copy and fix guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SshFailKind {
    PasswordAuth,
    KeyAuth,
    Auth,
    IdentityMissing,
    Timeout,
    Resolve,
    HostKey,
    ProbeOutput,
    Other,
}

pub(super) fn classify_ssh_failure(detail: &str) -> SshFailKind {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("ssh password authentication failed") {
        SshFailKind::PasswordAuth
    } else if lower.contains("ssh key authentication failed") {
        SshFailKind::KeyAuth
    } else if lower.contains("ssh connection succeeded")
        || lower.contains("ssh authentication succeeded")
        || lower.contains("probe command returned no output")
    {
        SshFailKind::ProbeOutput
    } else if lower.contains("identity file")
        || lower.contains("not accessible")
        || lower.contains("no such identity")
    {
        SshFailKind::IdentityMissing
    } else if lower.contains("permission denied")
        || lower.contains("publickey")
        || lower.contains("too many authentication failures")
        || lower.contains("authentication failed")
    {
        SshFailKind::Auth
    } else if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("no route to host")
        || lower.contains("network is unreachable")
    {
        SshFailKind::Timeout
    } else if lower.contains("could not resolve")
        || lower.contains("name or service not known")
        || lower.contains("nodename nor servname")
    {
        SshFailKind::Resolve
    } else if lower.contains("host key verification failed")
        || lower.contains("remote host identification has changed")
    {
        SshFailKind::HostKey
    } else {
        SshFailKind::Other
    }
}

/// i18n keys for bullet causes under the failed-diagnosis phase.
pub(super) fn ssh_fail_cause_keys(kind: SshFailKind) -> &'static [&'static str] {
    match kind {
        SshFailKind::PasswordAuth => &[
            "ssh_check.cause.password.1",
            "ssh_check.cause.password.2",
            "ssh_check.cause.password.3",
        ],
        SshFailKind::KeyAuth => &[
            "ssh_check.cause.key.1",
            "ssh_check.cause.key.2",
            "ssh_check.cause.key.3",
        ],
        SshFailKind::Auth => &[
            "ssh_check.cause.auth.1",
            "ssh_check.cause.auth.2",
            "ssh_check.cause.auth.3",
            "ssh_check.cause.auth.4",
        ],
        SshFailKind::IdentityMissing => {
            &["ssh_check.cause.identity.1", "ssh_check.cause.identity.2"]
        }
        SshFailKind::Timeout => &[
            "ssh_check.cause.timeout.1",
            "ssh_check.cause.timeout.2",
            "ssh_check.cause.timeout.3",
        ],
        SshFailKind::Resolve => &["ssh_check.cause.resolve.1", "ssh_check.cause.resolve.2"],
        SshFailKind::HostKey => &["ssh_check.cause.hostkey.1", "ssh_check.cause.hostkey.2"],
        SshFailKind::ProbeOutput => &[
            "ssh_check.cause.probe_output.1",
            "ssh_check.cause.probe_output.2",
            "ssh_check.cause.probe_output.3",
        ],
        SshFailKind::Other => &[
            "ssh_check.cause.other.1",
            "ssh_check.cause.other.2",
            "ssh_check.cause.other.3",
        ],
    }
}

/// Returns a human detail when SSH connectivity is not known-good.
pub(super) fn ssh_connectivity_gap(ctx: &ExecutionContext) -> Option<String> {
    if ctx.kind != "ssh" {
        return None;
    }
    match ctx.last_probe_status.as_deref() {
        Some("ok") => None,
        Some("error") => Some(
            ctx.last_probe_error
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "probe failed".into()),
        ),
        _ => Some("not probed yet".into()),
    }
}

pub(super) fn ssh_context_known_good(ctx: &ExecutionContext) -> bool {
    ssh_connectivity_gap(ctx).is_none()
}

/// Errors that need host configuration / Probe, not a blind retry.
pub(super) fn is_ssh_setup_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("ssh connectivity is not confirmed")
        || lower.contains("ssh connectivity gate blocked")
        || lower.contains("identity file is not accessible")
        || lower.contains("no successful probe")
}

/// Prefer the active remote source; fall back to ``ssh:alias`` embedded in the error.
pub(super) fn ssh_setup_context_id(preferred: Option<&str>, error: &str) -> Option<String> {
    if let Some(id) = preferred.filter(|id| id.starts_with("ssh:")) {
        return Some(id.to_string());
    }
    // Messages use backticks: `ssh:host-alias`
    let Some(start) = error.find("`ssh:") else {
        return None;
    };
    let rest = &error[start + 1..];
    let end = rest.find('`').unwrap_or(rest.len());
    let id = &rest[..end];
    id.starts_with("ssh:").then(|| id.to_string())
}

pub(super) fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

pub(super) fn step_tool_meta(
    locale: Locale,
    duration_ms: Option<u64>,
    started_at_ms: Option<u64>,
    ok: Option<bool>,
    lines: usize,
    now: u64,
) -> Option<String> {
    let dur = duration_ms.map(format_duration_ms).or_else(|| {
        (ok.is_none())
            .then_some(started_at_ms?)
            .map(|start| format_duration_ms(now.saturating_sub(start)))
    });
    let line_label = (lines > 0 && ok != Some(false))
        .then(|| tf(locale, "chat.step_lines", &[("n", &lines.to_string())]));
    match (dur, line_label) {
        (Some(d), Some(l)) => Some(format!("{d} · {l}")),
        (Some(d), None) => Some(d),
        (None, Some(l)) => Some(l),
        (None, None) => None,
    }
}

pub(super) fn finalize_tool_duration(
    started_at_ms: &mut Option<u64>,
    store: &mut Option<u64>,
    event_ms: u64,
) {
    let elapsed = if event_ms > 0 {
        event_ms
    } else if let Some(start) = started_at_ms.take() {
        now_ms().saturating_sub(start)
    } else {
        0
    };
    if elapsed > 0 {
        *store = Some(elapsed);
    }
    started_at_ms.take();
}

pub(super) fn allow_drop(ev: &web_sys::DragEvent) {
    ev.prevent_default();
    ev.stop_propagation();
    if let Some(dt) = ev.data_transfer() {
        let _ = dt.set_drop_effect("move");
    }
}

pub(super) fn drag_session_id(ev: &web_sys::DragEvent, cached: Option<String>) -> Option<String> {
    cached.filter(|s| !s.is_empty()).or_else(|| {
        ev.data_transfer()
            .and_then(|dt| dt.get_data("text/plain").ok())
            .filter(|s| !s.is_empty())
    })
}

pub(super) fn start_session_drag(ev: &web_sys::DragEvent, id: &str) {
    ev.stop_propagation();
    if let Some(dt) = ev.data_transfer() {
        let _ = dt.set_effect_allowed("move");
        let _ = dt.set_data("text/plain", id);
    }
}

pub(super) fn composer_attachment_key(name: &str, idx: usize) -> String {
    format!("att-{idx}-{name}")
}

fn ready_attachment_key(path: &str) -> String {
    format!("path:{path}")
}

/// Attach an already-project-relative (or absolute native) path as a chip.
/// Returns false when the path was already attached.
pub(super) fn attach_ready_path(
    attachments: RwSignal<Vec<ComposerAttachment>>,
    path: impl Into<String>,
) -> bool {
    let path = path.into();
    if path.trim().is_empty() {
        return false;
    }
    if attachments.get_untracked().iter().any(|attachment| {
        matches!(attachment, ComposerAttachment::Ready { path: existing, .. } if existing == &path)
    }) {
        return false;
    }
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path.as_str())
        .to_string();
    let key = ready_attachment_key(&path);
    attachments.update(|items| {
        items.push(ComposerAttachment::Ready { key, name, path });
    });
    true
}

pub(super) const COMPOSER_H_DEFAULT: f64 = 220.0;
pub(super) const COMPOSER_H_MIN: f64 = 80.0;
pub(super) const COMPOSER_H_MAX: f64 = 400.0;
pub(super) const COMPOSER_H_KEY: &str = "composerHeight";
pub(super) const COMPOSER_H_SAVED_KEY: &str = "composerHeightCustom";
pub(super) const SIDEBAR_W_DEFAULT: f64 = 248.0;
pub(super) const SIDEBAR_W_MIN: f64 = 200.0;
pub(super) const SIDEBAR_W_MAX: f64 = 520.0;
pub(super) const SIDEBAR_W_KEY: &str = "sidebarWidth";

pub(super) fn load_composer_h() -> f64 {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(COMPOSER_H_KEY).ok().flatten())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(COMPOSER_H_DEFAULT)
        .clamp(COMPOSER_H_MIN, COMPOSER_H_MAX)
}

pub(super) fn composer_h_custom() -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(COMPOSER_H_SAVED_KEY).ok().flatten())
        .is_some_and(|v| v == "1")
}

pub(super) fn save_composer_h(h: f64) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(COMPOSER_H_KEY, &h.to_string());
        let _ = s.set_item(COMPOSER_H_SAVED_KEY, "1");
    }
}

pub(super) fn load_sidebar_w() -> f64 {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(SIDEBAR_W_KEY).ok().flatten())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(SIDEBAR_W_DEFAULT)
        .clamp(SIDEBAR_W_MIN, SIDEBAR_W_MAX)
}

pub(super) fn save_sidebar_w(w: f64) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(SIDEBAR_W_KEY, &w.to_string());
    }
}

pub(super) fn parse_upload_results(v: JsValue) -> Vec<UploadFileResult> {
    if v.is_null() || v.is_undefined() {
        return vec![];
    }
    serde_wasm_bindgen::from_value(v).unwrap_or_default()
}

pub(super) fn file_list_len(files: &JsValue) -> usize {
    js_sys::Reflect::get(files, &JsValue::from_str("length"))
        .ok()
        .and_then(|n| n.as_f64())
        .map(|n| n as usize)
        .unwrap_or(0)
}

pub(super) fn begin_uploads(
    attachments: RwSignal<Vec<ComposerAttachment>>,
    uploading: RwSignal<bool>,
    count: usize,
) {
    if count == 0 {
        return;
    }
    attachments.update(|items| {
        for i in 0..count {
            items.push(ComposerAttachment::Uploading {
                key: format!("up-{}-{i}", js_sys::Date::now()),
                name: String::new(),
            });
        }
    });
    uploading.set(true);
}

pub(super) fn finish_uploads(
    attachments: RwSignal<Vec<ComposerAttachment>>,
    uploading: RwSignal<bool>,
    results: Vec<UploadFileResult>,
) {
    uploading.set(false);
    attachments.update(|items| {
        merge_finished_uploads(items, results);
    });
}

fn merge_finished_uploads(items: &mut Vec<ComposerAttachment>, results: Vec<UploadFileResult>) {
    items.retain(|a| !matches!(a, ComposerAttachment::Uploading { .. }));
    let mut ready_paths = items
        .iter()
        .filter_map(|attachment| match attachment {
            ComposerAttachment::Ready { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();

    for result in results {
        let name = result
            .info
            .as_ref()
            .map(|i| i.name.clone())
            .or(result.filename.clone())
            .unwrap_or_else(|| "file".into());
        if result.ok {
            if let Some(info) = result.info {
                let path = info.path;
                if !ready_paths.insert(path.clone()) {
                    continue;
                }
                items.push(ComposerAttachment::Ready {
                    key: ready_attachment_key(&path),
                    name,
                    path,
                });
            }
        } else {
            items.push(ComposerAttachment::Error {
                key: composer_attachment_key(&name, items.len()),
                name,
                error: result.error.unwrap_or_else(|| "Upload failed".into()),
            });
        }
    }
}

// Closes the `<details class="settings-add-menu">` a menu button lives in,
// mirroring native `<select>`-style auto-close so the menu doesn't linger
// open after the user picks an option.
pub(super) fn close_details_ancestor(ev: &web_sys::MouseEvent) {
    let el = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    if let Some(details) = el.and_then(|e| e.closest("details").ok().flatten()) {
        details.remove_attribute("open").ok();
    }
}

pub(super) fn queue_uploads(
    attachments: RwSignal<Vec<ComposerAttachment>>,
    uploading: RwSignal<bool>,
    files: JsValue,
) {
    let count = file_list_len(&files);
    begin_uploads(attachments, uploading, count);
    spawn_local(async move {
        finish_uploads(
            attachments,
            uploading,
            parse_upload_results(upload_files(files).await),
        );
    });
}

pub(super) fn upload_from_input(
    attachments: RwSignal<Vec<ComposerAttachment>>,
    uploading: RwSignal<bool>,
    input_id: &'static str,
) {
    uploading.set(true);
    spawn_local(async move {
        let v = upload_input_files(input_id).await;
        finish_uploads(attachments, uploading, parse_upload_results(v));
    });
}

pub(super) fn upload_from_paste(
    attachments: RwSignal<Vec<ComposerAttachment>>,
    uploading: RwSignal<bool>,
    event: JsValue,
    count: usize,
) {
    begin_uploads(attachments, uploading, count);
    spawn_local(async move {
        let v = upload_pasted_images(event).await;
        finish_uploads(attachments, uploading, parse_upload_results(v));
    });
}

pub(super) fn attachment_paths(items: &[ComposerAttachment]) -> Vec<String> {
    items
        .iter()
        .filter_map(|a| match a {
            ComposerAttachment::Ready { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn refresh_execution_contexts(into: RwSignal<Vec<ExecutionContext>>) {
    spawn_local(async move {
        let v = invoke("list_execution_contexts", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ExecutionContext>>(v) {
            into.set(list);
        }
    });
}

pub(super) fn refresh_session_execution_contexts(
    into: RwSignal<HashSet<String>>,
    active_session: RwSignal<Option<String>>,
    session_id: String,
) {
    spawn_local(async move {
        let args = to_value(&serde_json::json!({ "sessionId": session_id.clone() })).unwrap();
        let Ok(value) = invoke_checked("list_session_execution_context_ids", args).await else {
            return;
        };
        let Ok(ids) = serde_wasm_bindgen::from_value::<Vec<String>>(value) else {
            return;
        };
        if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
            into.set(ids.into_iter().collect());
        }
    });
}

pub(super) fn refresh_runtimes(into: RwSignal<Vec<RuntimeInfo>>) {
    spawn_local(async move {
        let value = invoke("list_runtimes", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<RuntimeInfo>>(value) {
            into.set(list);
        }
    });
}

pub(super) fn refresh_runs(into: RwSignal<Vec<RunRecord>>, locale: RwSignal<Locale>) {
    spawn_local(async move {
        let v = invoke("list_runs", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<RunRecord>>(v) {
            let initialized = RUN_REFRESH_INITIALIZED.with(Cell::get);
            let stopped_runs = list
                .iter()
                .filter(|run| {
                    matches!(run.status.as_str(), "failed" | "lost")
                        && run.last_poll_error.as_deref().is_some_and(|error| {
                            error
                                .to_ascii_lowercase()
                                .contains(SSH_RETRY_STOPPED_MARKER)
                        })
                })
                .map(|run| run.id.clone())
                .collect::<Vec<_>>();
            let should_toast = SSH_RETRY_TOASTED_RUNS.with(|seen| {
                let mut seen = seen.borrow_mut();
                let mut added = false;
                for run_id in stopped_runs {
                    if seen.insert(run_id) {
                        added = true;
                    }
                }
                initialized && added
            });
            into.set(list);
            RUN_REFRESH_INITIALIZED.with(|ready| ready.set(true));
            if should_toast {
                show_warning_toast(&t(locale.get_untracked(), "runs.ssh_retry_stopped"));
            }
        }
    });
}

pub(super) fn show_probe_stopped_toast(value: &JsValue, locale: RwSignal<Locale>) {
    let Ok(context) = serde_wasm_bindgen::from_value::<ExecutionContext>(value.clone()) else {
        return;
    };
    if context.last_probe_status.as_deref() == Some("error") {
        let key = if context
            .last_probe_error
            .as_deref()
            .is_some_and(|detail| classify_ssh_failure(detail) == SshFailKind::ProbeOutput)
        {
            "contexts.probe_incomplete"
        } else {
            "contexts.probe_stopped"
        };
        show_warning_toast(&t(locale.get_untracked(), key));
    }
}

pub(super) fn context_capability_summary(ctx: &ExecutionContext) -> String {
    let parsed = serde_json::from_str::<serde_json::Value>(&ctx.capabilities_json).ok();
    let mut parts = Vec::new();
    if let Some(v) = parsed.as_ref() {
        let os = v.get("os").and_then(|x| x.as_str()).unwrap_or_default();
        let arch = v.get("arch").and_then(|x| x.as_str()).unwrap_or_default();
        match (os.is_empty(), arch.is_empty()) {
            (false, false) => parts.push(format!("{os}/{arch}")),
            (false, true) => parts.push(os.to_string()),
            (true, false) => parts.push(arch.to_string()),
            (true, true) => {}
        }
        for key in ["gpu_summary", "scheduler", "python", "r_version"] {
            if let Some(s) = v
                .get(key)
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty())
            {
                parts.push(s.to_string());
            }
        }
        if v.get("probe_skill").and_then(|x| x.as_str()).is_some()
            && v.get("gpu_summary").is_none_or(serde_json::Value::is_null)
        {
            parts.push("No GPU".into());
        }
        if let Some(privilege) = v.get("privilege").and_then(|x| x.as_str()) {
            parts.push(privilege.to_string());
        }
    }
    if parts.is_empty() {
        ctx.last_probe_status
            .clone()
            .unwrap_or_else(|| "not probed".into())
    } else {
        parts.join(" · ")
    }
}

pub(super) fn language_display(language: &str) -> &str {
    match language {
        "r" => "R",
        "python" => "Python",
        other => other,
    }
}

/// Compute-context entries for the composer `@` menu: every execution context
/// as a server target, plus a runtime entry per available language on it.
/// Query tokens (split on non-alphanumerics, so `runtime_R` works) must all
/// match the entry's descriptive haystack.
fn context_display_label(ctx: &ExecutionContext) -> String {
    if ctx.label.trim().is_empty() {
        ctx.id.clone()
    } else {
        ctx.label.clone()
    }
}

/// Contexts a source file can bind its runtime to, as (id, label) pairs for the
/// preview picker. Same availability rule as the composer's `@` runtime entries,
/// so a file cannot bind to a runtime `@` would not offer. Empty means nothing
/// on this machine can run the language and there is no binding to make.
pub(super) fn runtime_binding_options(
    contexts: &[ExecutionContext],
    language: &str,
) -> Vec<(String, String)> {
    contexts
        .iter()
        .filter(|ctx| context_runtime_available(ctx, language))
        .map(|ctx| (ctx.id.clone(), context_display_label(ctx)))
        .collect()
}

/// Resolve which context a script is actually bound to. A stored (or default)
/// binding that cannot host the language is not a binding — falling back to the
/// first context that can keeps the picker's displayed value and the context a
/// run is sent to from ever disagreeing.
pub(super) fn resolve_runtime_binding(
    options: &[(String, String)],
    stored: Option<&str>,
) -> Option<String> {
    let hosted = |id: &str| options.iter().any(|(option, _)| option == id);
    stored
        .filter(|id| hosted(id))
        .map(str::to_string)
        .or_else(|| {
            hosted(LOCAL_CONTEXT_ID)
                .then(|| LOCAL_CONTEXT_ID.to_string())
                .or_else(|| options.first().map(|(id, _)| id.clone()))
        })
}

pub(super) fn mention_compute_entries(
    query: &str,
    contexts: &[ExecutionContext],
) -> Vec<ComposerPickerItem> {
    let query = query.to_lowercase();
    let tokens: Vec<&str> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let matches = |haystack: String| tokens.iter().all(|t| haystack.to_lowercase().contains(t));
    let mut items = Vec::new();
    for ctx in contexts {
        let label = context_display_label(ctx);
        if matches(format!("server {} {} {label}", ctx.kind, ctx.id)) {
            items.push(ComposerPickerItem::Context {
                id: ctx.id.clone(),
                label: label.clone(),
            });
        }
        for language in ["python", "r"] {
            if context_runtime_available(ctx, language)
                && matches(format!(
                    "runtime {language} {} {} {label}",
                    ctx.kind, ctx.id
                ))
            {
                items.push(ComposerPickerItem::Runtime {
                    context_id: ctx.id.clone(),
                    context_label: label.clone(),
                    language: language.to_string(),
                });
            }
        }
    }
    items
}

fn context_runtime_available(ctx: &ExecutionContext, language: &str) -> bool {
    if ctx.kind == "local" && language == "python" {
        return true;
    }
    let config = serde_json::from_str::<serde_json::Value>(&ctx.config_json).unwrap_or_default();
    let capabilities =
        serde_json::from_str::<serde_json::Value>(&ctx.capabilities_json).unwrap_or_default();
    let has_value = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.trim().is_empty())
    };
    match language {
        "python" => {
            ["python_executable", "python_path"]
                .iter()
                .any(|key| has_value(&config, key))
                || has_value(&capabilities, "python_executable")
        }
        "r" => {
            if ["rscript_executable", "rscript_path"]
                .iter()
                .any(|key| has_value(&config, key))
            {
                return true;
            }
            if has_value(&capabilities, "rscript_executable") {
                return capabilities
                    .get("r_jsonlite")
                    .and_then(|value| value.as_bool())
                    != Some(false);
            }
            ctx.kind == "local" && ctx.last_probe_status.as_deref() != Some("ok")
        }
        _ => false,
    }
}

#[cfg(test)]
mod runtime_slot_tests {
    use super::{
        classify_ssh_failure, context_runtime_available, is_ssh_setup_error,
        mention_compute_entries, ssh_connectivity_gap, ssh_fail_cause_keys, ssh_setup_context_id,
        ComposerPickerItem, SshFailKind,
    };
    use crate::dto::ExecutionContext;
    use crate::i18n::Locale;

    fn context(
        kind: &str,
        capabilities_json: &str,
        probe_status: Option<&str>,
    ) -> ExecutionContext {
        ExecutionContext {
            id: if kind == "local" { "local" } else { "ssh:test" }.into(),
            kind: kind.into(),
            label: "Test".into(),
            config_json: "{}".into(),
            capabilities_json: capabilities_json.into(),
            last_probe_at: None,
            last_probe_status: probe_status.map(str::to_string),
            last_probe_error: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn ssh_connectivity_gap_requires_successful_probe() {
        assert!(ssh_connectivity_gap(&context("local", "{}", None)).is_none());
        assert_eq!(
            ssh_connectivity_gap(&context("ssh", "{}", None)).as_deref(),
            Some("not probed yet")
        );
        assert_eq!(
            ssh_connectivity_gap(&context("ssh", "{}", Some("error"))).as_deref(),
            Some("probe failed")
        );
        assert!(ssh_connectivity_gap(&context("ssh", "{}", Some("ok"))).is_none());
    }

    #[test]
    fn ssh_setup_error_helpers_detect_and_parse_context() {
        let err = "SSH connectivity is not confirmed for `ssh:insertsbio_public`: no successful probe yet.";
        assert!(is_ssh_setup_error(err));
        assert_eq!(
            ssh_setup_context_id(None, err).as_deref(),
            Some("ssh:insertsbio_public")
        );
        assert_eq!(
            ssh_setup_context_id(Some("ssh:other"), err).as_deref(),
            Some("ssh:other")
        );
        assert!(!is_ssh_setup_error("Remote directory empty"));
    }

    #[test]
    fn classify_ssh_failure_maps_permission_denied_to_auth() {
        let detail =
            "SSH probe failed with exit 255: user@host: Permission denied (publickey,password).";
        assert_eq!(classify_ssh_failure(detail), SshFailKind::Auth);
        assert!(ssh_fail_cause_keys(SshFailKind::Auth).len() >= 3);
        assert_eq!(
            classify_ssh_failure("SSH password authentication failed for `gpu-box`"),
            SshFailKind::PasswordAuth
        );
        assert_eq!(
            classify_ssh_failure("SSH key authentication failed for `gpu-box`"),
            SshFailKind::KeyAuth
        );
        assert_eq!(
            classify_ssh_failure("Connection timed out"),
            SshFailKind::Timeout
        );
        assert_eq!(
            classify_ssh_failure("identity file is not accessible"),
            SshFailKind::IdentityMissing
        );
        assert_eq!(
            classify_ssh_failure(
                "SSH connection succeeded, but the environment probe could not read operating system information"
            ),
            SshFailKind::ProbeOutput
        );
    }

    #[test]
    fn optional_r_slot_distinguishes_unknown_available_and_missing() {
        assert!(context_runtime_available(
            &context("local", "{}", None),
            "r"
        ));
        assert!(!context_runtime_available(
            &context("local", r#"{"rscript_executable":null}"#, Some("ok")),
            "r"
        ));
        assert!(context_runtime_available(
            &context(
                "ssh",
                r#"{"rscript_executable":"/usr/bin/Rscript","r_jsonlite":true}"#,
                Some("ok")
            ),
            "r"
        ));
        assert!(!context_runtime_available(
            &context(
                "ssh",
                r#"{"rscript_executable":"/usr/bin/Rscript","r_jsonlite":false}"#,
                Some("ok")
            ),
            "r"
        ));
    }

    #[test]
    fn binding_options_offer_only_contexts_that_can_host_the_language() {
        let contexts = vec![
            context("local", "{}", None),
            context(
                "ssh",
                r#"{"rscript_executable":"/usr/bin/Rscript","r_jsonlite":false}"#,
                Some("ok"),
            ),
        ];
        // Local always hosts Python; the SSH host has R but no jsonlite, so it
        // cannot host an R runtime and must not be offered as a binding.
        let r = super::runtime_binding_options(&contexts, "r");
        assert_eq!(r, vec![("local".to_string(), "Test".to_string())]);
        let python = super::runtime_binding_options(&contexts, "python");
        assert_eq!(
            python.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            vec!["local"]
        );
    }

    #[test]
    fn binding_resolves_to_a_context_that_can_actually_host_the_language() {
        let options = vec![
            ("local".to_string(), "Local".to_string()),
            ("ssh:gpu".to_string(), "GPU".to_string()),
        ];
        // A stored binding is honoured...
        assert_eq!(
            super::resolve_runtime_binding(&options, Some("ssh:gpu")),
            Some("ssh:gpu".to_string())
        );
        // ...unless that context cannot host the language, in which case the
        // picker would show one context while runs went to another.
        assert_eq!(
            super::resolve_runtime_binding(&options, Some("ssh:gone")),
            Some("local".to_string())
        );
        assert_eq!(
            super::resolve_runtime_binding(&options, None),
            Some("local".to_string())
        );
        // Local cannot host R without Rscript; fall back to one that can.
        let r_only = vec![("ssh:gpu".to_string(), "GPU".to_string())];
        assert_eq!(
            super::resolve_runtime_binding(&r_only, None),
            Some("ssh:gpu".to_string())
        );
        // Nothing can host it: no binding, so no run controls.
        assert_eq!(super::resolve_runtime_binding(&[], Some("local")), None);
    }

    #[test]
    fn console_echo_prefixes_every_submitted_line() {
        assert_eq!(
            super::console_echo("library(Seurat)\nlibrary(dplyr)", Locale::En),
            "> library(Seurat)\n> library(dplyr)"
        );
    }

    #[test]
    fn console_echo_bounds_long_script_previews() {
        let code = (1..=40)
            .map(|line| format!("line_{line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let echo = super::console_echo(&code, Locale::En);
        assert_eq!(echo.lines().count(), 11);
        assert!(echo.contains("> line_1"));
        assert!(echo.contains("> … 30 submitted lines omitted …"));
        assert!(echo.contains("> line_40"));
        assert!(!echo.contains("> line_20"));

        let zh_echo = super::console_echo(&code, Locale::Zh);
        assert!(zh_echo.contains("> … 已省略 30 行提交代码 …"));
    }

    #[test]
    fn closed_worker_error_is_user_facing() {
        assert_eq!(
            crate::i18n::localize_backend(Locale::Zh, "kernel worker closed protocol stdout"),
            "Runtime 进程意外退出，请重启后再运行代码。"
        );
    }

    #[test]
    fn mention_entries_match_servers_and_runtimes() {
        let contexts = vec![context(
            "ssh",
            r#"{"rscript_executable":"/usr/bin/Rscript","r_jsonlite":true}"#,
            Some("ok"),
        )];
        // Empty query lists the server plus its available runtime.
        let all = mention_compute_entries("", &contexts);
        assert!(all.iter().any(
            |item| matches!(item, ComposerPickerItem::Context { id, .. } if id == "ssh:test")
        ));
        assert!(all.iter().any(|item| matches!(
            item,
            ComposerPickerItem::Runtime { language, .. } if language == "r"
        )));
        // `runtime_R` style queries tokenize on the underscore and drop the
        // server entry (its haystack has no "runtime" token).
        let runtimes = mention_compute_entries("runtime_R", &contexts);
        assert!(runtimes.iter().any(|item| matches!(
            item,
            ComposerPickerItem::Runtime { language, .. } if language == "r"
        )));
        assert!(!runtimes
            .iter()
            .any(|item| matches!(item, ComposerPickerItem::Context { .. })));
        // Server label match.
        assert!(mention_compute_entries("test", &contexts)
            .iter()
            .any(|item| matches!(item, ComposerPickerItem::Context { .. })));
        assert!(mention_compute_entries("nomatch", &contexts).is_empty());
    }
}

pub(super) fn runtime_slots(
    runtimes: Vec<RuntimeInfo>,
    contexts: &[ExecutionContext],
    active_project: Option<ProjectInfo>,
    projects: &[ProjectSummary],
) -> Vec<RuntimeSlot> {
    let project_label = |id: &str| {
        active_project
            .as_ref()
            .filter(|project| project.id == id)
            .map(|project| project.name.clone())
            .or_else(|| {
                projects
                    .iter()
                    .find(|project| project.id == id)
                    .map(|project| project.name.clone())
            })
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| id.to_string())
    };
    let context_label = |id: &str| {
        contexts
            .iter()
            .find(|context| context.id == id)
            .map(|context| {
                if context.label.trim().is_empty() {
                    context.id.clone()
                } else {
                    context.label.clone()
                }
            })
            .unwrap_or_else(|| id.to_string())
    };

    let mut present = HashSet::new();
    let mut slots = runtimes
        .into_iter()
        .map(|info| {
            present.insert((
                info.key.project_id.clone(),
                info.key.context_id.clone(),
                info.key.language.clone(),
            ));
            RuntimeSlot {
                project_id: info.key.project_id.clone(),
                project_label: project_label(&info.key.project_id),
                context_id: info.key.context_id.clone(),
                context_label: context_label(&info.key.context_id),
                language: info.key.language.clone(),
                available: true,
                info: Some(info),
            }
        })
        .collect::<Vec<_>>();

    if let Some(project) = active_project.as_ref() {
        for context in contexts {
            for language in ["python", "r"] {
                let key = (project.id.clone(), context.id.clone(), language.to_string());
                if present.insert(key) {
                    slots.push(RuntimeSlot {
                        project_id: project.id.clone(),
                        project_label: project_label(&project.id),
                        context_id: context.id.clone(),
                        context_label: context_label(&context.id),
                        language: language.to_string(),
                        available: context_runtime_available(context, language),
                        info: None,
                    });
                }
            }
        }
    }
    slots.sort_by(|left, right| {
        left.project_id
            .cmp(&right.project_id)
            .then_with(|| left.context_id.cmp(&right.context_id))
            .then_with(|| left.language.cmp(&right.language))
    });
    slots
}

fn invoke_runtime_control(
    command: &'static str,
    args: serde_json::Value,
    locale: RwSignal<Locale>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
) {
    spawn_local(async move {
        let args = to_value(&args).unwrap();
        match invoke_checked(command, args).await {
            Ok(_) => refresh_runtimes(runtimes),
            Err(error) => {
                let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                show_toast(&message);
                refresh_runtimes(runtimes);
            }
        }
    });
}

pub(super) fn runtime_status_label(locale: Locale, status: &str) -> String {
    let key = match status {
        "starting" => "runtime.starting",
        "ready" => "runtime.ready",
        "busy" => "runtime.busy",
        "stopping" => "runtime.stopping",
        "dead" => "runtime.dead",
        "unavailable" => "runtime.unavailable",
        _ => "runtime.missing",
    };
    t(locale, key).into()
}

pub(super) fn inspect_runtime_objects(
    state_key: String,
    project_id: String,
    context_id: String,
    language: String,
    locale: RwSignal<Locale>,
    states: RwSignal<HashMap<String, RuntimeObjectState>>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
) {
    states.update(|states| {
        let state = states.entry(state_key.clone()).or_default();
        state.loading = true;
        state.error = None;
    });
    spawn_local(async move {
        let args = to_value(&serde_json::json!({
            "projectId": project_id,
            "contextId": context_id,
            "language": language,
        }))
        .unwrap();
        let result = match invoke_checked("inspect_runtime", args).await {
            Ok(value) => serde_wasm_bindgen::from_value::<RuntimeObjectList>(value)
                .map_err(|error| error.to_string()),
            Err(error) => Err(localize_backend(
                locale.get_untracked(),
                &js_error_text(error),
            )),
        };
        states.update(|states| {
            let state = states.entry(state_key).or_default();
            state.loading = false;
            match result {
                Ok(snapshot) => {
                    state.snapshot = Some(snapshot);
                    state.error = None;
                }
                Err(error) => state.error = Some(error),
            }
        });
        refresh_runtimes(runtimes);
    });
}

/// Mirrors `wisp_runtime::LOCAL_CONTEXT_ID`. `ui/` is a separate workspace and
/// cannot depend on the runtime crate, so the default binding is spelled here.
pub(super) const LOCAL_CONTEXT_ID: &str = "local";

/// Stable object-inspection key for the runtime bound to a center source file.
/// Unlike the process runtime id, this survives the lazy first start and lets
/// the inspector publish variables immediately after selected code runs.
pub(super) fn runtime_binding_state_key(
    project_id: &str,
    context_id: &str,
    language: &str,
) -> String {
    format!("binding:{project_id}:{context_id}:{language}")
}

/// Console log per previewed file path. Ephemeral like the runtime it mirrors:
/// a log that outlived its process would describe variables that no longer
/// exist. Use "add to chat" to hand a result to the agent.
pub(super) type RuntimeConsoles = HashMap<String, String>;

/// R and Python consoles echo submitted code behind a prompt. Keeping that here
/// is what lets one flat log stay readable as alternating input and output.
fn console_echo(code: &str, locale: Locale) -> String {
    const MAX_LINES: usize = 12;
    const HEAD_LINES: usize = 7;
    const TAIL_LINES: usize = 3;

    let lines = code.lines().collect::<Vec<_>>();
    let visible = if lines.len() <= MAX_LINES {
        lines.into_iter().map(str::to_string).collect::<Vec<_>>()
    } else {
        let omitted = lines.len() - HEAD_LINES - TAIL_LINES;
        lines[..HEAD_LINES]
            .iter()
            .map(|line| (*line).to_string())
            .chain(std::iter::once(tf(
                locale,
                "runtime.console_omitted",
                &[("n", &omitted.to_string())],
            )))
            .chain(
                lines[lines.len() - TAIL_LINES..]
                    .iter()
                    .map(|line| (*line).to_string()),
            )
            .collect::<Vec<_>>()
    };
    visible
        .into_iter()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_console(consoles: RwSignal<RuntimeConsoles>, path: &str, text: &str) {
    consoles.update(|logs| {
        let log = logs.entry(path.to_string()).or_default();
        if !log.is_empty() {
            log.push('\n');
        }
        log.push_str(text);
    });
}

/// The signals a file preview needs to run code against its bound runtime. They
/// always travel together; passing them as one keeps the run helpers readable.
#[derive(Clone, Copy)]
pub(super) struct RuntimeRunCtx {
    pub(super) consoles: RwSignal<RuntimeConsoles>,
    pub(super) busy: RwSignal<Option<String>>,
    pub(super) runtimes: RwSignal<Vec<RuntimeInfo>>,
    pub(super) project: RwSignal<Option<ProjectInfo>>,
    pub(super) object_states: RwSignal<HashMap<String, RuntimeObjectState>>,
    pub(super) inspector_open: RwSignal<bool>,
    pub(super) locale: RwSignal<Locale>,
}

/// Run `code` from the `path` preview against its bound runtime and append the
/// result to that file's console. The runtime starts lazily, so a not-yet-running
/// binding needs no separate Start click.
pub(super) fn run_in_runtime(
    path: String,
    context_id: String,
    language: String,
    code: String,
    locale: Locale,
    ctx: RuntimeRunCtx,
) {
    let code = code.trim().to_string();
    // ponytail: one run at a time across all files. Key `busy` by path if two
    // runtimes on different contexts ever need to run concurrently.
    if code.is_empty() || ctx.busy.get_untracked().is_some() {
        return;
    }
    ctx.inspector_open.set(true);
    append_console(ctx.consoles, &path, &console_echo(&code, locale));
    ctx.busy.set(Some(path.clone()));
    spawn_local(async move {
        let args = to_value(&serde_json::json!({
            "contextId": context_id,
            "language": language,
            "code": code,
        }))
        .unwrap();
        let output = match invoke_checked("execute_runtime", args).await {
            Ok(value) => value.as_string().unwrap_or_default(),
            Err(error) => localize_backend(locale, &js_error_text(error)),
        };
        append_console(ctx.consoles, &path, &output);
        ctx.busy.set(None);
        // Execution may have lazily created the process. Inspect by its stable
        // binding key so variables appear without waiting for list_runtimes to
        // reveal a new process id first.
        if let Some(project) = ctx.project.get_untracked() {
            inspect_runtime_objects(
                runtime_binding_state_key(&project.id, &context_id, &language),
                project.id,
                context_id,
                language,
                ctx.locale,
                ctx.object_states,
                ctx.runtimes,
            );
        } else {
            refresh_runtimes(ctx.runtimes);
        }
    });
}

/// One-line quote the selection popup actions (add to chat / explain) send for
/// a runtime variable. `summary`/`size` arrive display-normalized ("—" = none).
pub(super) fn runtime_object_quote(
    language: &str,
    name: &str,
    type_name: &str,
    summary: &str,
    size: &str,
) -> String {
    let mut quote = format!("[{language} runtime] {name}: {type_name}");
    if summary != "—" {
        quote.push_str(" = ");
        quote.push_str(summary);
    }
    if size != "—" {
        quote.push_str(&format!(" ({size})"));
    }
    quote
}

fn runtime_environment_viewport() -> (i32, i32) {
    web_sys::window()
        .map(|window| {
            let width = window
                .inner_width()
                .ok()
                .and_then(|value| value.as_f64())
                .unwrap_or(1280.0) as i32;
            let height = window
                .inner_height()
                .ok()
                .and_then(|value| value.as_f64())
                .unwrap_or(720.0) as i32;
            (width, height)
        })
        .unwrap_or((1280, 720))
}

fn clamp_runtime_environment_position(x: i32, y: i32) -> (i32, i32) {
    const MARGIN: i32 = 16;
    const PANEL_WIDTH: i32 = 620;
    const PANEL_HEIGHT: i32 = 560;
    let (viewport_width, viewport_height) = runtime_environment_viewport();
    let width = PANEL_WIDTH.min((viewport_width - MARGIN * 2).max(0));
    let height = PANEL_HEIGHT.min((viewport_height - MARGIN * 2).max(0));
    (
        x.clamp(MARGIN, (viewport_width - width - MARGIN).max(MARGIN)),
        y.clamp(MARGIN, (viewport_height - height - MARGIN).max(MARGIN)),
    )
}

fn default_runtime_environment_position() -> (i32, i32) {
    let (viewport_width, viewport_height) = runtime_environment_viewport();
    clamp_runtime_environment_position(viewport_width - 636, (viewport_height - 560) / 2)
}

#[component]
pub(super) fn RuntimeEnvironmentPanel(
    selected: RwSignal<Option<RuntimeSlot>>,
    pinned: RwSignal<bool>,
    position: RwSignal<(i32, i32)>,
    context_modal: RwSignal<Option<(String, ContextModalKind)>>,
    locale: RwSignal<Locale>,
    states: RwSignal<HashMap<String, RuntimeObjectState>>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
    selection_popup: RwSignal<Option<(String, Option<String>, i32, i32)>>,
) -> impl IntoView {
    let drag_start = Rc::new(Cell::new(None::<(i32, i32, i32, i32, i32)>));
    let dragging = create_rw_signal(false);

    move || {
        selected.get().map(|mut slot| {
        let drag_start_down = drag_start.clone();
        let drag_start_move = drag_start.clone();
        let drag_start_up = drag_start.clone();
        let drag_start_cancel = drag_start.clone();
        slot.info = runtimes.get().into_iter().find(|runtime| {
            runtime.key.project_id == slot.project_id
                && runtime.key.context_id == slot.context_id
                && runtime.key.language == slot.language
        });
        let language_label = if slot.language == "r" { "R" } else { "Python" };
        let status = slot.info.as_ref()
            .map(|info| info.status.clone())
            .unwrap_or_else(|| if slot.available { "missing".into() } else { "unavailable".into() });
        let status_class = format!("runtime-status {status}");
        let runtime_id = slot.info.as_ref().map(|info| info.runtime_id.clone()).unwrap_or_default();
        let can_refresh = status == "ready";
        let refresh_runtime_id = runtime_id.clone();
        let loading_runtime_id = runtime_id.clone();
        let content_runtime_id = runtime_id;
        let refresh_project = slot.project_id.clone();
        let refresh_context = slot.context_id.clone();
        let refresh_language = slot.language.clone();

        view! {
            <section class="runtime-environment-panel" role="region"
                class:is-pinned=move || pinned.get()
                class:is-dragging=move || dragging.get()
                style=move || {
                    let (x, y) = position.get();
                    format!("--runtime-environment-x:{x}px;--runtime-environment-y:{y}px")
                }
                aria-label=tf(locale.get(), "runtime.environment_title", &[("language", language_label)])>
                <div class="runtime-environment-head">
                    <div class="runtime-environment-title"
                        on:pointerdown=move |event: web_sys::PointerEvent| {
                            if !pinned.get_untracked() || event.button() != 0 {
                                return;
                            }
                            event.prevent_default();
                            let Some(target) = event.target()
                                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                            else {
                                return;
                            };
                            let _ = target.set_pointer_capture(event.pointer_id());
                            let (x, y) = position.get_untracked();
                            drag_start_down.set(Some((
                                event.client_x(),
                                event.client_y(),
                                x,
                                y,
                                event.pointer_id(),
                            )));
                            dragging.set(true);
                        }
                        on:pointermove=move |event: web_sys::PointerEvent| {
                            let Some((start_x, start_y, origin_x, origin_y, _)) = drag_start_move.get() else {
                                return;
                            };
                            event.prevent_default();
                            position.set(clamp_runtime_environment_position(
                                origin_x + event.client_x() - start_x,
                                origin_y + event.client_y() - start_y,
                            ));
                        }
                        on:pointerup=move |event: web_sys::PointerEvent| {
                            if let Some((_, _, _, _, pointer_id)) = drag_start_up.take() {
                                if let Some(target) = event.target()
                                    .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                                {
                                    let _ = target.release_pointer_capture(pointer_id);
                                }
                            }
                            dragging.set(false);
                        }
                        on:pointercancel=move |_| {
                            drag_start_cancel.set(None);
                            dragging.set(false);
                        }>
                        <h3>{tf(locale.get(), "runtime.environment_title", &[("language", language_label)])}</h3>
                        <span>{format!("{} · {}", slot.project_label, slot.context_label)}</span>
                    </div>
                    <button type="button" class="runtime-environment-pin"
                        class:active=move || pinned.get()
                        aria-pressed=move || pinned.get().to_string()
                        title=move || if pinned.get() {
                            t(locale.get(), "runtime.unpin_environment")
                        } else {
                            t(locale.get(), "runtime.pin_environment")
                        }
                        aria-label=move || if pinned.get() {
                            t(locale.get(), "runtime.unpin_environment")
                        } else {
                            t(locale.get(), "runtime.pin_environment")
                        }
                        on:click=move |_| {
                            if pinned.get_untracked() {
                                pinned.set(false);
                                if context_modal.get_untracked().is_none() {
                                    selected.set(None);
                                }
                            } else {
                                position.set(default_runtime_environment_position());
                                pinned.set(true);
                                context_modal.set(None);
                            }
                        }>{compose_icon("pin")}</button>
                    <span class=status_class>{runtime_status_label(locale.get(), &status)}</span>
                    <button type="button" class="runtime-environment-refresh"
                        title=t(locale.get(), "runtime.inspect_objects")
                        aria-label=t(locale.get(), "runtime.inspect_objects")
                        disabled=move || !can_refresh || states.with(|states| {
                            states.get(&loading_runtime_id).is_some_and(|state| state.loading)
                        })
                        on:click=move |_| inspect_runtime_objects(
                            refresh_runtime_id.clone(),
                            refresh_project.clone(),
                            refresh_context.clone(),
                            refresh_language.clone(),
                            locale,
                            states,
                            runtimes,
                        )>{compose_icon("sync")}</button>
                    <button type="button" class="runtime-environment-close"
                        title=t(locale.get(), "runtime.close_environment")
                        aria-label=t(locale.get(), "runtime.close_environment")
                        on:click=move |_| {
                            selected.set(None);
                            pinned.set(false);
                            dragging.set(false);
                        }>{compose_icon("close")}</button>
                </div>
                <div class="runtime-environment-table-head" aria-hidden="true">
                    <span>{t(locale.get(), "runtime.object_name")}</span>
                    <span>{t(locale.get(), "runtime.object_type")}</span>
                    <span>{t(locale.get(), "runtime.object_value")}</span>
                    <span>{t(locale.get(), "runtime.object_size")}</span>
                </div>
                <div class="runtime-environment-body">
                    {move || {
                        if content_runtime_id.is_empty() {
                            return view! {
                                <div class="runtime-environment-empty">{t(locale.get(), "runtime.environment_unavailable")}</div>
                            }.into_view();
                        }
                        let state = states.with(|states| {
                            states.get(&content_runtime_id).cloned().unwrap_or_default()
                        });
                        if state.loading && state.snapshot.is_none() {
                            return view! {
                                <div class="runtime-environment-empty">{t(locale.get(), "runtime.objects_loading")}</div>
                            }.into_view();
                        }
                        if let Some(error) = state.error {
                            return view! { <div class="context-error">{error}</div> }.into_view();
                        }
                        let Some(snapshot) = state.snapshot else {
                            return view! {
                                <div class="runtime-environment-empty">{t(locale.get(), "runtime.objects_hint")}</div>
                            }.into_view();
                        };
                        if snapshot.objects.is_empty() {
                            return view! {
                                <div class="runtime-environment-empty">{t(locale.get(), "runtime.objects_empty")}</div>
                            }.into_view();
                        }
                        let shown = snapshot.objects.len();
                        let total = snapshot.total_count;
                        view! {
                            <div class="runtime-environment-rows">
                                {snapshot.objects.into_iter().map(|object| {
                                    let size = object.size_bytes.map(format_bytes).unwrap_or_else(|| "—".into());
                                    let summary = if object.summary.is_empty() { "—".into() } else { object.summary };
                                    let quote = runtime_object_quote(
                                        language_label, &object.name, &object.type_name, &summary, &size,
                                    );
                                    let key_quote = quote.clone();
                                    view! {
                                        <div class="runtime-environment-row" role="button" tabindex="0"
                                            title=t(locale.get(), "runtime.quote_object")
                                            on:click=move |ev: web_sys::MouseEvent| {
                                                selection_popup.set(Some((
                                                    quote.clone(), None, ev.client_x(), ev.client_y(),
                                                )));
                                            }
                                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                if ev.key() != "Enter" && ev.key() != " " {
                                                    return;
                                                }
                                                ev.prevent_default();
                                                let Some(rect) = ev.target()
                                                    .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                                                    .map(|el| el.get_bounding_client_rect())
                                                else {
                                                    return;
                                                };
                                                selection_popup.set(Some((
                                                    key_quote.clone(), None,
                                                    (rect.left() + rect.width() / 2.0) as i32,
                                                    rect.bottom() as i32,
                                                )));
                                            }>
                                            <span class="runtime-object-name" title=object.name.clone()>{object.name}</span>
                                            <span class="runtime-object-type" title=object.type_name.clone()>{object.type_name}</span>
                                            <span class="runtime-object-value" title=summary.clone()>{summary}</span>
                                            <span class="runtime-object-size">{size}</span>
                                        </div>
                                    }
                                }).collect_view()}
                            </div>
                            {(shown < total).then(|| view! {
                                <div class="runtime-objects-limit">{
                                    tf(locale.get(), "runtime.objects_showing", &[
                                        ("shown", &shown.to_string()),
                                        ("total", &total.to_string()),
                                    ])
                                }</div>
                            })}
                        }.into_view()
                    }}
                </div>
            </section>
        }
    })
    }
}

#[component]
pub(super) fn RuntimeCard(
    runtime_slot: RuntimeSlot,
    interpreter_form: Option<RuntimeInterpreterForm>,
    runtime_interpreter_form: RwSignal<Option<RuntimeInterpreterForm>>,
    runtime_environment: RwSignal<Option<RuntimeSlot>>,
    locale: RwSignal<Locale>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
    object_states: RwSignal<HashMap<String, RuntimeObjectState>>,
) -> impl IntoView {
    let slot = runtime_slot;
    let status = slot
        .info
        .as_ref()
        .map(|info| info.status.clone())
        .unwrap_or_else(|| {
            if slot.available {
                "missing".into()
            } else {
                "unavailable".into()
            }
        });
    let status_class = format!("runtime-status {status}");
    let language_label = if slot.language == "r" { "R" } else { "Python" };
    let identity = format!("{} · {}", slot.project_label, slot.context_label);
    let metadata = slot.info.as_ref().map(|info| {
        let mut parts = Vec::new();
        if let Some(interpreter) = info.interpreter.as_deref() {
            parts.push(interpreter.to_string());
        }
        if let Some(version) = info.version.as_deref() {
            parts.push(version.to_string());
        }
        if let Some(pid) = info.process_id {
            parts.push(format!("PID {pid}"));
        }
        parts.join(" · ")
    });
    let details = slot.info.as_ref().map(|info| {
        let activity =
            format_relative_time(info.last_activity_at_ms as i64, locale.get_untracked());
        let started = format_relative_time(info.started_at_ms as i64, locale.get_untracked());
        let memory = info
            .resident_memory_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "—".into());
        format!(
            "{} {} · {} {} · {} {} · {} {}",
            t(locale.get_untracked(), "runtime.generation"),
            info.generation,
            t(locale.get_untracked(), "runtime.memory"),
            memory,
            t(locale.get_untracked(), "runtime.started"),
            started,
            t(locale.get_untracked(), "runtime.last_activity"),
            activity
        )
    });
    let runtime_id = slot
        .info
        .as_ref()
        .map(|info| info.runtime_id.clone())
        .unwrap_or_default();
    let last_error = slot.info.as_ref().and_then(|info| info.last_error.clone());

    let start_context = slot.context_id.clone();
    let start_language = slot.language.clone();
    let stop_project = slot.project_id.clone();
    let stop_context = slot.context_id.clone();
    let stop_language = slot.language.clone();
    let restart_project = slot.project_id.clone();
    let restart_context = slot.context_id.clone();
    let restart_language = slot.language.clone();
    let environment_slot = slot.clone();
    let selected_project = slot.project_id.clone();
    let selected_context = slot.context_id.clone();
    let selected_language = slot.language.clone();
    let inspect_project = slot.project_id.clone();
    let inspect_context = slot.context_id.clone();
    let inspect_language = slot.language.clone();
    let inspect_runtime_id = runtime_id.clone();
    let can_stop = matches!(status.as_str(), "starting" | "ready" | "busy");
    let can_restart = matches!(status.as_str(), "ready" | "busy" | "dead");
    let can_start = status == "missing";
    let can_inspect = status == "ready";

    view! {
        <div class="runtime-card" data-runtime-language=slot.language.clone()
            class:environment-active=move || runtime_environment.with(|selected| {
                selected.as_ref().is_some_and(|selected| {
                    selected.project_id == selected_project
                        && selected.context_id == selected_context
                        && selected.language == selected_language
                })
            })
            data-runtime-context=slot.context_id.clone() data-runtime-id=runtime_id.clone()>
            <div class="runtime-card-head">
                <button type="button" class="runtime-language"
                    aria-label=tf(locale.get_untracked(), "runtime.open_environment", &[("language", language_label)])
                    on:click=move |_| {
                        runtime_environment.set(Some(environment_slot.clone()));
                        if can_inspect {
                            inspect_runtime_objects(
                                inspect_runtime_id.clone(),
                                inspect_project.clone(),
                                inspect_context.clone(),
                                inspect_language.clone(),
                                locale,
                                object_states,
                                runtimes,
                            );
                        }
                    }>
                    <span>{language_label}</span>
                    <span class="runtime-language-open" aria-hidden="true">{compose_icon("chevron-right")}</span>
                </button>
                <span class=status_class>{runtime_status_label(locale.get_untracked(), &status)}</span>
            </div>
            <div class="runtime-identity">{identity}</div>
            <div class="runtime-context">{format!("{} · {}", slot.project_id, slot.context_id)}</div>
            {metadata.filter(|value| !value.is_empty()).map(|value| view! {
                <div class="runtime-meta">{value}</div>
            })}
            {details.map(|value| view! { <div class="runtime-details">{value}</div> })}
            {(status == "unavailable").then(|| view! {
                <div class="runtime-unavailable">{t(locale.get_untracked(), "runtime.unavailable_hint")}</div>
            })}
            {last_error.map(|error| view! { <div class="context-error">{error}</div> })}
            <div class="runtime-actions">
                {interpreter_form.map(|form| view! {
                    <button type="button" class="runtime-config"
                        on:click=move |_| runtime_interpreter_form.set(Some(form.clone()))>
                        {move || t(locale.get(), "runtime.configure")}
                    </button>
                })}
                {can_start.then(|| view! {
                    <button type="button" class="runtime-start" on:click=move |_| {
                        invoke_runtime_control(
                            "start_runtime",
                            serde_json::json!({
                                "contextId": start_context.clone(),
                                "language": start_language.clone(),
                            }),
                            locale,
                            runtimes,
                        );
                    }>{move || t(locale.get(), "runtime.start")}</button>
                })}
                {can_stop.then(|| view! {
                    <button type="button" class="runtime-stop" on:click=move |_| {
                        invoke_runtime_control(
                            "stop_runtime",
                            serde_json::json!({
                                "projectId": stop_project.clone(),
                                "contextId": stop_context.clone(),
                                "language": stop_language.clone(),
                            }),
                            locale,
                            runtimes,
                        );
                    }>{move || t(locale.get(), "runtime.stop")}</button>
                })}
                {can_restart.then(|| view! {
                    <button type="button" class="runtime-restart" on:click=move |_| {
                        invoke_runtime_control(
                            "restart_runtime",
                            serde_json::json!({
                                "projectId": restart_project.clone(),
                                "contextId": restart_context.clone(),
                                "language": restart_language.clone(),
                            }),
                            locale,
                            runtimes,
                        );
                    }>{move || t(locale.get(), "runtime.restart")}</button>
                })}
            </div>
        </div>
    }
}

pub(super) fn run_title(run: &RunRecord) -> String {
    if !run.title.trim().is_empty() {
        run.title.clone()
    } else {
        run.command.clone().unwrap_or_else(|| run.id.clone())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ContextModalKind {
    Machine,
    Runtimes,
    Runs,
}

#[component]
pub(super) fn ContextDetailsOverlay(
    modal: RwSignal<Option<(String, ContextModalKind)>>,
    runtime_environment: RwSignal<Option<RuntimeSlot>>,
    runtime_environment_pinned: RwSignal<bool>,
    runtime_environment_position: RwSignal<(i32, i32)>,
    contexts: RwSignal<Vec<ExecutionContext>>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
    runs: RwSignal<Vec<RunRecord>>,
    active_project: RwSignal<Option<ProjectInfo>>,
    projects: RwSignal<Vec<ProjectSummary>>,
    runtime_interpreter_form: RwSignal<Option<RuntimeInterpreterForm>>,
    object_states: RwSignal<HashMap<String, RuntimeObjectState>>,
    locale: RwSignal<Locale>,
    selection_popup: RwSignal<Option<(String, Option<String>, i32, i32)>>,
) -> impl IntoView {
    create_effect(move |_| {
        let active = modal.get();
        let pinned = runtime_environment_pinned.get();
        let should_close = runtime_environment.with_untracked(|selected| {
            !pinned && selected.as_ref().is_some_and(|slot| {
                !matches!(&active, Some((context_id, ContextModalKind::Runtimes)) if context_id == &slot.context_id)
            })
        });
        if should_close {
            runtime_environment.set(None);
        }
    });

    move || {
        let Some((context_id, kind)) = modal.get() else {
            return ().into_view();
        };
        let Some(context) = contexts
            .get()
            .into_iter()
            .find(|context| context.id == context_id)
        else {
            modal.set(None);
            return ().into_view();
        };
        let context_label = if context.label.trim().is_empty() {
            context.id.clone()
        } else {
            context.label.clone()
        };
        let title = match kind {
            ContextModalKind::Machine => t(locale.get(), "contexts.machine_info"),
            ContextModalKind::Runtimes => t(locale.get(), "contexts.runtimes"),
            ContextModalKind::Runs => t(locale.get(), "contexts.runs"),
        };
        let body_context_id = context.id.clone();

        view! {
            <div class="overlay context-details-overlay" role="presentation">
                <div class="modal context-details-modal" class:runtime-details=kind == ContextModalKind::Runtimes
                    class:environment-open=move || {
                        runtime_environment.get().is_some() && !runtime_environment_pinned.get()
                    }
                    role="dialog" aria-modal="true" aria-label=title.clone()>
                    <div class="ps-head">
                        <div class="context-modal-title">
                            <h2>{title}</h2>
                            <span>{context_label}</span>
                        </div>
                        <button type="button" class="ps-close"
                            title=t(locale.get(), "contexts.close_details")
                            aria-label=t(locale.get(), "contexts.close_details")
                            on:click=move |_| modal.set(None)>{compose_icon("close")}</button>
                    </div>
                    {match kind {
                        ContextModalKind::Machine => {
                            let status = context.last_probe_status.clone().unwrap_or_else(|| "unknown".into());
                            let status_class = format!("context-status {status}");
                            let error = context.last_probe_error.clone();
                            view! {
                                <div class="context-machine-summary" data-context-id=context.id.clone()>
                                    <div class="context-machine-heading">
                                        <span class="context-id">{context.id.clone()}</span>
                                        <span class=status_class>{status}</span>
                                    </div>
                                    <dl class="context-machine-fields">
                                        <div><dt>{t(locale.get(), "contexts.kind")}</dt><dd>{context.kind.clone()}</dd></div>
                                        <div><dt>{t(locale.get(), "contexts.capabilities")}</dt><dd>{context_capability_summary(&context)}</dd></div>
                                    </dl>
                                    {error.map(|error| view! { <div class="context-error">{error}</div> })}
                                </div>
                            }.into_view()
                        }
                        ContextModalKind::Runtimes => {
                            let section_context_id = body_context_id.clone();
                            view! {
                                <div class="runtime-modal-body">
                                    {move || {
                                        let all_contexts = contexts.get();
                                        let rows = runtime_slots(
                                            runtimes.get(),
                                            &all_contexts,
                                            active_project.get(),
                                            &projects.get(),
                                        ).into_iter()
                                            .filter(|slot| slot.context_id == section_context_id)
                                            .collect::<Vec<_>>();
                                        view! {
                                            <section class="control-section context-modal-section" data-context-id=section_context_id.clone()>
                                                <div class="control-section-head">
                                                    <span>{t(locale.get(), "contexts.runtimes")}</span>
                                                    <div class="control-head-actions">
                                                        <span class="control-count">{rows.len().to_string()}</span>
                                                        <button type="button" class="icon-btn control-refresh"
                                                            title=t(locale.get(), "runtime.refresh")
                                                            aria-label=t(locale.get(), "runtime.refresh")
                                                            on:click=move |_| refresh_runtimes(runtimes)>{compose_icon("sync")}</button>
                                                    </div>
                                                </div>
                                                <div class="runtime-warning">{t(locale.get(), "runtime.state_warning")}</div>
                                                {if rows.is_empty() {
                                                    view! { <div class="control-empty">{t(locale.get(), "runtime.empty")}</div> }.into_view()
                                                } else {
                                                    rows.into_iter().map(|slot| {
                                                        let interpreter_form = all_contexts.iter()
                                                            .find(|context| context.id == slot.context_id)
                                                            .map(RuntimeInterpreterForm::from_context);
                                                        view! {
                                                            <RuntimeCard runtime_slot=slot interpreter_form=interpreter_form
                                                                runtime_interpreter_form=runtime_interpreter_form
                                                                runtime_environment=runtime_environment locale=locale runtimes=runtimes
                                                                object_states=object_states />
                                                        }
                                                    }).collect_view()
                                                }}
                                            </section>
                                        }
                                    }}
                                    {move || (!runtime_environment_pinned.get()).then(|| view! {
                                        <RuntimeEnvironmentPanel selected=runtime_environment
                                            pinned=runtime_environment_pinned
                                            position=runtime_environment_position context_modal=modal
                                            locale=locale states=object_states runtimes=runtimes
                                            selection_popup=selection_popup />
                                    })}
                                </div>
                            }.into_view()
                        }
                        ContextModalKind::Runs => {
                            let section_context_id = body_context_id.clone();
                            view! {
                                {move || {
                                    let rows = runs.get().into_iter()
                                        .filter(|run| run.context_id == section_context_id)
                                        .collect::<Vec<_>>();
                                    view! {
                                        <section class="control-section context-modal-section" data-context-id=section_context_id.clone()>
                                            <div class="control-section-head">
                                                <span>{t(locale.get(), "contexts.runs")}</span>
                                                <div class="control-head-actions">
                                                    <span class="control-count">{rows.len().to_string()}</span>
                                                    <button type="button" class="icon-btn control-refresh"
                                                        title=t(locale.get(), "runs.refresh")
                                                        aria-label=t(locale.get(), "runs.refresh")
                                                        on:click=move |_| refresh_runs(runs, locale)>{compose_icon("sync")}</button>
                                                </div>
                                            </div>
                                            {if rows.is_empty() {
                                                view! { <div class="control-empty">{t(locale.get(), "runs.empty")}</div> }.into_view()
                                            } else {
                                                rows.into_iter().map(|run| {
                                            let title = run_title(&run);
                                            let status_class = format!("run-status {}", run.status);
                                            let cancel_id = run.id.clone();
                                            let cancellable = matches!(run.status.as_str(), "submitted" | "running");
                                            let remote_workdir = run.remote_workdir.clone();
                                            let poll_error = run.last_poll_error.clone();
                                            let stdout_tail = run.stdout_tail.clone().unwrap_or_default();
                                            let stderr_tail = run.stderr_tail.clone().unwrap_or_default();
                                            let output = match (stdout_tail.is_empty(), stderr_tail.is_empty()) {
                                                (false, false) => format!("{stdout_tail}\n\n[stderr]\n{stderr_tail}"),
                                                (false, true) => stdout_tail,
                                                (true, false) => format!("[stderr]\n{stderr_tail}"),
                                                (true, true) => String::new(),
                                            };
                                            let meta = match run.exit_code {
                                                Some(code) => format!("{} · {} · exit {code}", run.context_id, run.kind),
                                                None => format!("{} · {}", run.context_id, run.kind),
                                            };
                                            view! {
                                                <div class="run-card">
                                                    <div class="run-card-head">
                                                        <span class="run-title">{title}</span>
                                                        <span class=status_class>{run.status.clone()}</span>
                                                        {cancellable.then(|| {
                                                            let run_id = cancel_id.clone();
                                                            view! {
                                                                <button type="button" class="icon-btn run-cancel"
                                                                    title=t(locale.get(), "runs.cancel")
                                                                    aria-label=t(locale.get(), "runs.cancel")
                                                                    on:click=move |_| {
                                                                        let run_id = run_id.clone();
                                                                        spawn_local(async move {
                                                                            let arg = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
                                                                            let _ = invoke("cancel_run", arg).await;
                                                                            refresh_runs(runs, locale);
                                                                        });
                                                                    }>{compose_icon("close")}</button>
                                                            }
                                                        })}
                                                    </div>
                                                    <div class="run-meta">{meta}</div>
                                                    {run.command.clone().filter(|command| !command.trim().is_empty()).map(|command| view! {
                                                        <div class="run-command">{command}</div>
                                                    })}
                                                    {remote_workdir.map(|workdir| view! {
                                                        <div class="run-remote">
                                                            <span>{t(locale.get(), "runs.remote_workdir")}</span>
                                                            <code>{workdir}</code>
                                                        </div>
                                                    })}
                                                    {poll_error.filter(|error| !error.trim().is_empty()).map(|error| view! {
                                                        <div class="context-error">{error}</div>
                                                    })}
                                                    {(!output.is_empty()).then(|| view! {
                                                        <details class="run-output">
                                                            <summary>{t(locale.get(), "runs.output")}</summary>
                                                            <pre>{output}</pre>
                                                        </details>
                                                    })}
                                                </div>
                                            }
                                                }).collect_view()
                                            }}
                                        </section>
                                    }
                                }}
                            }.into_view()
                        }
                    }}
                </div>
            </div>
        }.into_view()
    }
}

pub(super) fn message_with_attachments(text: &str, paths: &[String]) -> String {
    let body = text.trim();
    if paths.is_empty() {
        return body.to_string();
    }
    let files = paths.join(", ");
    if body.is_empty() {
        format!("Uploaded files: {files}")
    } else {
        format!("{body}\n\nUploaded files: {files}")
    }
}

fn prompt_safe_source(source: &str) -> String {
    source
        .replace(['\r', '\n'], " ")
        .replace('`', "\\`")
        .trim()
        .to_string()
}

/// Prefix the message body with quoted-selection snippets as markdown
/// blockquotes. Workspace selections retain their target path and carry a
/// stable action hint, so change requests lead to a real tool edit rather than
/// a suggested replacement code block.
pub(super) fn message_with_quotes(text: &str, quotes: &[ComposerQuote]) -> String {
    if quotes.is_empty() {
        return text.trim().to_string();
    }
    let mut out = String::new();
    for quote in quotes {
        if let Some(source) = quote.source.as_deref() {
            let source = prompt_safe_source(source);
            if quote.workspace_source().is_some() {
                out.push_str("Selected excerpt from workspace file `");
            } else {
                out.push_str("Selected excerpt from reference `");
            }
            out.push_str(&source);
            out.push_str("`:\n");
        }
        for line in quote.text.trim().lines() {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(text.trim());
    let mut editable_sources = quotes
        .iter()
        .filter_map(ComposerQuote::workspace_source)
        .map(prompt_safe_source)
        .collect::<Vec<_>>();
    editable_sources.sort();
    editable_sources.dedup();
    if !editable_sources.is_empty() {
        out.push_str("\n\nAI source-edit instruction: If the user requests a change, read the selected workspace file first, modify it directly with the edit tool for a focused in-place change (use write only for a whole-file replacement), and verify the saved result. Do not only return a replacement code block. Target file");
        out.push_str(if editable_sources.len() == 1 {
            ": `"
        } else {
            "s: `"
        });
        out.push_str(&editable_sources.join("`, `"));
        out.push('`');
    }
    out.trim_end().to_string()
}

/// Chip label for a quoted selection: first line, capped at 40 chars.
pub(super) fn quote_label(text: &str) -> String {
    let line = text.trim().lines().next().unwrap_or_default();
    let mut label: String = line.chars().take(40).collect();
    if line.chars().count() > 40 {
        label.push('…');
    }
    label
}

/// Build the persisted user-facing turn. Reference labels are deliberately
/// kept in the message alongside upload paths: the backend still receives the
/// typed reference ids separately, while a reloaded transcript retains enough
/// information for the UI to rebuild its attachment cards.
pub(super) fn message_with_composer_context(
    text: &str,
    paths: &[String],
    references: &[ComposerReferenceChip],
    quotes: &[ComposerQuote],
) -> String {
    let mut message = message_with_attachments(&message_with_quotes(text, quotes), paths);
    let mut artifacts = Vec::new();
    let mut sessions = Vec::new();
    let mut skills = Vec::new();
    let mut contexts = Vec::new();
    let mut runtimes = Vec::new();
    for reference in references {
        match reference {
            ComposerReferenceChip::Artifact { name, .. } => artifacts.push(name.clone()),
            ComposerReferenceChip::Session {
                title,
                project_name,
                ..
            } => sessions.push(format!("{project_name} / {title}")),
            ComposerReferenceChip::Skill { name } => skills.push(name.clone()),
            ComposerReferenceChip::Context { label, .. } => contexts.push(label.clone()),
            ComposerReferenceChip::Runtime { .. } => runtimes.push(reference.label()),
        }
    }
    for (label, values) in [
        ("Attached artifacts", artifacts),
        ("Attached sessions", sessions),
        ("Selected skills", skills),
        ("Target environments", contexts),
        ("Target runtimes", runtimes),
    ] {
        if values.is_empty() {
            continue;
        }
        if !message.is_empty() {
            message.push_str("\n\n");
        }
        message.push_str(label);
        message.push_str(": ");
        message.push_str(&values.join(", "));
    }
    message
}

/// If the composer text ends in an `@`, `#`, or `/` token, return its byte
/// offset, picker mode, and query. ponytail: this is end-of-text only; upgrade
/// to caret-index scanning when editing mentions in the middle matters.
pub(super) fn active_composer_trigger(text: &str) -> Option<(usize, ComposerPickerMode, String)> {
    let (at, trigger) = text
        .char_indices()
        .rev()
        .find(|(_, c)| matches!(c, '@' | '#' | '/'))?;
    if at > 0 && !text[..at].chars().next_back()?.is_whitespace() {
        return None;
    }
    let query = &text[at + 1..];
    if query.chars().any(char::is_whitespace) {
        return None;
    }
    let mode = match trigger {
        '@' => ComposerPickerMode::Artifact,
        '#' => ComposerPickerMode::Session,
        '/' => ComposerPickerMode::Skill,
        _ => return None,
    };
    Some((at, mode, query.to_string()))
}

pub(super) fn scroll_picker_item(selector: &str, index: usize) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(items) = document.query_selector_all(selector) else {
        return;
    };
    if let Some(item) = items.item(index as u32) {
        item.unchecked_into::<web_sys::Element>().scroll_into_view();
    }
}

pub(super) fn scroll_to_transcript(index: usize) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if let Ok(Some(row)) = document.query_selector(&format!("[data-ui-index=\"{index}\"]")) {
        row.scroll_into_view();
    }
}

#[cfg(test)]
mod mention_tests {
    use super::{active_composer_trigger, ComposerPickerMode};

    #[test]
    fn detects_mention_at_end() {
        assert!(
            matches!(active_composer_trigger("look at @qc"), Some((8, ComposerPickerMode::Artifact, q)) if q == "qc")
        );
        assert!(
            matches!(active_composer_trigger("#old"), Some((0, ComposerPickerMode::Session, q)) if q == "old")
        );
        assert!(
            matches!(active_composer_trigger("/boltz"), Some((0, ComposerPickerMode::Skill, q)) if q == "boltz")
        );
    }

    #[test]
    fn ignores_non_mentions() {
        assert_eq!(active_composer_trigger("no trigger"), None);
        assert_eq!(active_composer_trigger("email a@b.com"), None);
        assert_eq!(active_composer_trigger("@qc then more"), None);
    }
}

pub(super) fn js_error_text(err: JsValue) -> String {
    err.as_string()
        .or_else(|| {
            js_sys::Reflect::get(&err, &JsValue::from_str("message"))
                .ok()
                .and_then(|v| v.as_string())
        })
        .unwrap_or_else(|| t(Locale::En, "err.unknown").into())
}

pub(super) fn show_copy_toast() {
    let is_zh = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
        .and_then(|element| element.get_attribute("lang"))
        .as_deref()
        == Some("zh");
    show_toast(if is_zh { "已复制" } else { "Copied" });
}

pub(super) fn show_toast(message: &str) {
    show_toast_with_class(message, "copy-toast");
}

pub(super) fn show_warning_toast(message: &str) {
    show_toast_with_class(message, "copy-toast copy-toast-warning");
}

fn show_toast_with_class(message: &str, class_name: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    if let Some(old) = document.get_element_by_id("copy-toast") {
        old.remove();
    }
    let Ok(toast) = document.create_element("div") else {
        return;
    };
    toast.set_id("copy-toast");
    toast.set_class_name(class_name);
    toast.set_text_content(Some(message));
    let Some(body) = document.body() else {
        return;
    };
    if body.append_child(&toast).is_err() {
        return;
    }
    let remove = Closure::once(move || toast.remove());
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        remove.as_ref().unchecked_ref(),
        1_600,
    );
    remove.forget();
}

pub(super) fn copy_text(text: String) {
    if text.is_empty() {
        return;
    }
    spawn_local(async move {
        let Some(window) = web_sys::window() else {
            return;
        };
        let promise = window.navigator().clipboard().write_text(&text);
        if wasm_bindgen_futures::JsFuture::from(promise).await.is_ok() {
            show_copy_toast();
        }
    });
}

fn normalize_table_copy_cell(text: &str) -> String {
    let mut out = String::new();
    for part in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

fn table_data_to_tsv(t: &TableData) -> String {
    let mut lines = Vec::with_capacity(t.rows.len() + usize::from(!t.headers.is_empty()));
    if !t.headers.is_empty() {
        lines.push(
            t.headers
                .iter()
                .map(|cell| normalize_table_copy_cell(cell))
                .collect::<Vec<_>>()
                .join("\t"),
        );
    }
    lines.extend(t.rows.iter().map(|row| {
        row.iter()
            .map(|cell| normalize_table_copy_cell(cell))
            .collect::<Vec<_>>()
            .join("\t")
    }));
    lines.join("\n")
}

fn html_table_to_tsv(table: &web_sys::HtmlTableElement) -> String {
    let rows = table.rows();
    let mut lines = Vec::with_capacity(rows.length() as usize);
    for i in 0..rows.length() {
        let Some(row) = rows.item(i) else { continue };
        let Ok(row) = row.dyn_into::<web_sys::HtmlTableRowElement>() else {
            continue;
        };
        let cells = row.cells();
        let mut vals = Vec::with_capacity(cells.length() as usize);
        for j in 0..cells.length() {
            let Some(cell) = cells.item(j) else { continue };
            vals.push(normalize_table_copy_cell(
                &cell.text_content().unwrap_or_default(),
            ));
        }
        if !vals.is_empty() {
            lines.push(vals.join("\t"));
        }
    }
    lines.join("\n")
}

fn wrap_markdown_tables_with_copy_controls(html: String, locale: Locale) -> String {
    let copy_label = html_escape(&t(locale, "table.copy"));
    let mut out = String::with_capacity(html.len());
    let mut rest = html.as_str();
    while let Some(start) = rest.find("<table") {
        out.push_str(&rest[..start]);
        let table_rest = &rest[start..];
        let Some(end) = table_rest.find("</table>") else {
            out.push_str(table_rest);
            return out;
        };
        let table_html = &table_rest[..end + "</table>".len()];
        out.push_str(&format!(
            r#"<div class="md-table-card"><div class="tbl-head"><button type="button" class="tbl-copy md-table-copy" title="{copy_label}" aria-label="{copy_label}">{copy_label}</button></div><div class="tbl-wrap">{table_html}</div></div>"#
        ));
        rest = &table_rest[end + "</table>".len()..];
    }
    out.push_str(rest);
    out
}

pub(super) fn normalize_settings_mut(cfg: &mut Settings) {
    cfg.provider = provider_value(&cfg.provider).into();
    cfg.api_url = cfg.api_url.trim().into();
    cfg.model = cfg.model.trim().into();
    cfg.sync_backend = if cfg.sync_backend == "folder" {
        "folder".into()
    } else {
        "relay".into()
    };
    cfg.sync_relay_url = cfg.sync_relay_url.trim().into();
    cfg.sync_folder = cfg.sync_folder.trim().into();
}

pub(super) fn normalized_settings(mut cfg: Settings) -> Settings {
    normalize_settings_mut(&mut cfg);
    cfg
}

pub(super) fn settings_required_error_key(cfg: &Settings, key: &str) -> Option<&'static str> {
    if cfg.api_url.trim().is_empty() {
        return Some("err.api_url_required");
    }
    if cfg.model.trim().is_empty() {
        return Some("err.model_required");
    }
    let has_new_key = !key.trim().is_empty();
    if !cfg.has_api_key && !has_new_key {
        return Some("err.api_key_required");
    }
    None
}

/// Single source of truth for `invoke` argument payloads.
///
/// Tauri v2 deserializes command arguments from JS **camelCase** keys onto the
/// Rust **snake_case** parameters. A snake_case key (`session_id`) never binds:
/// an `Option` param silently becomes `None`, which once made every send fork a
/// brand-new conversation instead of continuing the active one. Keep every
/// multi-word key camelCase here; `tauri_args_tests` pins them.
pub(super) mod tauri_args {
    use serde_json::{json, Value};

    pub fn stop_agent(session_id: &Option<String>) -> Value {
        json!({ "sessionId": session_id })
    }
    pub fn review_session(session_id: &Option<String>) -> Value {
        json!({ "sessionId": session_id })
    }
    pub fn branch_session(
        session_id: &Option<String>,
        title: Option<&str>,
        user_index: Option<usize>,
    ) -> Value {
        let mut payload = json!({ "sessionId": session_id });
        if let Some(title) = title.map(str::trim).filter(|s| !s.is_empty()) {
            payload["title"] = json!(title);
        }
        if let Some(user_index) = user_index {
            payload["userIndex"] = json!(user_index);
        }
        payload
    }
    pub fn rewind_session(session_id: &Option<String>, user_index: usize) -> Value {
        json!({ "sessionId": session_id, "userIndex": user_index })
    }
    pub fn confirm_response(
        session_id: &str,
        approved: bool,
        feedback: Option<&str>,
        scope: Option<&str>,
    ) -> Value {
        let mut payload = json!({ "sessionId": session_id, "approved": approved });
        if let Some(feedback) = feedback.map(str::trim).filter(|s| !s.is_empty()) {
            payload["feedback"] = json!(feedback);
        }
        if let Some(scope) = scope.map(str::trim).filter(|s| !s.is_empty()) {
            payload["scope"] = json!(scope);
        }
        payload
    }
    pub fn read_file(path: &str, max_bytes: Option<u64>) -> Value {
        match max_bytes {
            Some(n) => json!({ "path": path, "maxBytes": n }),
            None => json!({ "path": path }),
        }
    }
}

#[cfg(test)]
mod tauri_args_tests {
    use super::*;

    // Guard the exact regression: `session_id` must reach the backend as the
    // camelCase `sessionId`, or `send_message` starts a new conversation.
    #[test]
    fn send_message_args_serialize_camel_case() {
        let v = serde_json::to_value(SendMessageArgs {
            session_id: Some("frame-1".into()),
            message: "hi".into(),
            attachments: vec!["a.png".into()],
            references: vec![],
            resume: false,
            acp_agent_id: None,
        })
        .unwrap();
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["message"], "hi");
        assert_eq!(v["attachments"][0], "a.png");
        assert!(
            v.get("session_id").is_none(),
            "snake_case key would bind to None on the backend"
        );
    }

    #[test]
    fn session_command_args_use_camel_case_keys() {
        let sid = Some("frame-1".to_string());

        let v = tauri_args::stop_agent(&sid);
        assert_eq!(v["sessionId"], "frame-1");
        assert!(v.get("session_id").is_none());

        let v = tauri_args::review_session(&sid);
        assert_eq!(v["sessionId"], "frame-1");

        let v = tauri_args::branch_session(&sid, Some("fork here"), Some(2));
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["title"], "fork here");
        assert_eq!(v["userIndex"], 2);
        assert!(v.get("session_id").is_none());
        assert!(v.get("user_index").is_none());

        let v = tauri_args::rewind_session(&sid, 3);
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["userIndex"], 3);
        assert!(v.get("user_index").is_none());

        let v = tauri_args::confirm_response("frame-1", true, None, Some("once"));
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["approved"], true);
        assert_eq!(v["scope"], "once");
        assert!(v.get("feedback").is_none());

        let v = tauri_args::confirm_response("frame-1", false, Some("split the plan"), None);
        assert_eq!(v["feedback"], "split the plan");
        assert!(v.get("scope").is_none());

        let v = tauri_args::read_file("a.txt", Some(1024));
        assert_eq!(v["path"], "a.txt");
        assert_eq!(v["maxBytes"], 1024);
        assert!(v.get("max_bytes").is_none());
    }

    // The agent is told to emit absolute paths, so a clicked file link must reach
    // the backend intact. Stripping the leading slash turned `/Users/…/fig.png`
    // into a bad root-relative path that 404'd on click (#12).
    #[test]
    fn normalize_path_keeps_absolute_paths() {
        assert_eq!(
            normalize_path("/Users/x/proj/results/fig.png"),
            "/Users/x/proj/results/fig.png"
        );
        assert_eq!(normalize_path("C:\\proj\\out.csv"), "C:\\proj\\out.csv");
        // Redundant current-dir prefixes are still trimmed; relative stays relative.
        assert_eq!(normalize_path("./results/fig.png"), "results/fig.png");
        assert_eq!(normalize_path(".\\results\\fig.png"), "results\\fig.png");
        assert_eq!(normalize_path("results/fig.png"), "results/fig.png");
        assert_eq!(normalize_path("  /a/b.txt  "), "/a/b.txt");
        assert_eq!(
            normalize_path("figures/panel_I_heatmap_4genes_median.png/.pdf"),
            "figures/panel_I_heatmap_4genes_median.png"
        );
        assert_eq!(
            normalize_path("./figures/plot.JPG/.PDF"),
            "figures/plot.JPG"
        );
        assert_eq!(
            normalize_path("C:\\proj\\fig.png\\.pdf"),
            "C:\\proj\\fig.png"
        );
    }

    #[test]
    fn collect_artifacts_normalizes_image_pdf_shorthand() {
        let items = vec![ChatItem::Assistant {
            text: "`figures/panel_I_heatmap_4genes_median.png/.pdf`".into(),
            model: None,
            resources: Vec::new(),
        }];
        let arts = collect_artifacts(&items, Locale::En, &mut ProtoCache::new());
        let a = arts
            .iter()
            .find(|a| a.name == "panel_I_heatmap_4genes_median.png")
            .unwrap();
        assert_eq!(a.kind, "image");
        match &a.data {
            PreviewData::File { path, kind } => {
                assert_eq!(path, "figures/panel_I_heatmap_4genes_median.png");
                assert_eq!(kind, "image");
            }
            _ => panic!("expected file artifact"),
        }
    }
}

pub(super) fn split_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn join_tags(tags: &[String]) -> String {
    tags.join(", ")
}

pub(super) fn skill_matches_filter(skill: &SkillRow, tag: &str, query: &str) -> bool {
    let tag_match = match tag {
        "" => true,
        "__untagged" => skill.tags.is_empty(),
        "__enabled" => skill.enabled,
        "__disabled" => !skill.enabled,
        t => skill.tags.iter().any(|s| s == t),
    };
    let q = query.trim().to_ascii_lowercase();
    tag_match
        && (q.is_empty()
            || skill.name.to_ascii_lowercase().contains(&q)
            || skill.description.to_ascii_lowercase().contains(&q))
}

#[cfg(test)]
mod skill_filter_tests {
    use super::*;

    fn skill(name: &str, enabled: bool, tags: &[&str]) -> SkillRow {
        SkillRow {
            name: name.into(),
            description: "Scientific workflow".into(),
            tags: tags.iter().map(|tag| (*tag).into()).collect(),
            enabled,
            builtin: true,
            dir: String::new(),
        }
    }

    #[test]
    fn skill_status_filters_compose_with_search() {
        let enabled = skill("remote-compute", true, &["compute"]);
        let disabled = skill("literature-review", false, &[]);

        assert!(skill_matches_filter(&enabled, "__enabled", "remote"));
        assert!(!skill_matches_filter(&enabled, "__disabled", ""));
        assert!(skill_matches_filter(&disabled, "__disabled", "workflow"));
        assert!(!skill_matches_filter(&disabled, "__enabled", ""));
    }
}

pub(super) fn refresh_capabilities(caps: RwSignal<Option<Capabilities>>) {
    spawn_local(async move {
        let v = invoke("get_capabilities", JsValue::UNDEFINED).await;
        if let Ok(data) = serde_wasm_bindgen::from_value::<Capabilities>(v) {
            caps.set(Some(data));
        }
    });
}

pub(super) fn begin_pending_turn(
    pending: RwSignal<HashMap<String, usize>>,
    running: RwSignal<HashSet<String>>,
    id: &str,
) {
    pending.update(|m| {
        *m.entry(id.to_string()).or_insert(0) += 1;
    });
    running.update(|r| {
        r.insert(id.to_string());
    });
}

/// Decide how `get_acp_session_agent` should update the picker selection.
///
/// Returning `None` means "leave the current selection alone" — needed when the
/// first ACP turn is still binding the session and the backend still reports
/// `None` (otherwise the picker snaps back to the HTTP model mid-send).
pub(super) fn acp_agent_selection_after_fetch(
    fetched: Option<String>,
    session_id: &str,
    pending: &HashMap<String, usize>,
    running: &HashSet<String>,
    provisional: Option<&(String, String)>,
) -> Option<Option<String>> {
    match fetched {
        Some(id) => Some(Some(id)),
        None if provisional.is_some_and(|(frame_id, _)| frame_id == session_id) => {
            Some(provisional.map(|(_, agent_id)| agent_id.clone()))
        }
        None if pending.contains_key(session_id) || running.contains(session_id) => None,
        None => Some(None),
    }
}

/// Fold a `CurrentModeUpdate` payload into the stored SessionModeState.
///
/// The update only carries `currentModeId`, so when we already hold the initial
/// `SessionModeState` we keep its `availableModes` (which the mode picker needs)
/// and only swap the current id. With no prior object, the payload stands alone.
pub(super) fn merge_current_mode(
    existing: Option<&serde_json::Value>,
    payload: serde_json::Value,
) -> serde_json::Value {
    if let (Some(serde_json::Value::Object(existing)), Some(id)) =
        (existing, payload.get("currentModeId"))
    {
        let mut merged = existing.clone();
        merged.insert("currentModeId".into(), id.clone());
        return serde_json::Value::Object(merged);
    }
    payload
}

#[cfg(test)]
mod merge_current_mode_tests {
    use super::merge_current_mode;
    use serde_json::json;

    #[test]
    fn preserves_available_modes_on_update() {
        let existing = json!({
            "currentModeId": "full-access",
            "availableModes": [{"id": "agent", "name": "Agent"}, {"id": "full-access", "name": "Full Access"}],
        });
        let merged = merge_current_mode(Some(&existing), json!({ "currentModeId": "agent" }));
        assert_eq!(merged["currentModeId"], json!("agent"));
        assert_eq!(merged["availableModes"], existing["availableModes"]);
    }

    #[test]
    fn falls_back_to_payload_without_prior_state() {
        let merged = merge_current_mode(None, json!({ "currentModeId": "agent" }));
        assert_eq!(merged, json!({ "currentModeId": "agent" }));
    }
}

#[cfg(test)]
mod acp_agent_selection_tests {
    use super::acp_agent_selection_after_fetch;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn applies_bound_agent() {
        let pending = HashMap::new();
        let running = HashSet::new();
        assert_eq!(
            acp_agent_selection_after_fetch(Some("agent-1".into()), "s1", &pending, &running, None),
            Some(Some("agent-1".into()))
        );
    }

    #[test]
    fn preserves_selection_while_first_turn_pending() {
        let mut pending = HashMap::new();
        pending.insert("s1".into(), 1);
        let running = HashSet::new();
        assert_eq!(
            acp_agent_selection_after_fetch(None, "s1", &pending, &running, None),
            None
        );
    }

    #[test]
    fn preserves_provisional_agent_on_a_fresh_session() {
        let pending = HashMap::new();
        let running = HashSet::new();
        let provisional = ("s1".into(), "agent-1".into());
        assert_eq!(
            acp_agent_selection_after_fetch(None, "s1", &pending, &running, Some(&provisional)),
            Some(Some("agent-1".into()))
        );
    }

    #[test]
    fn clears_when_session_has_no_binding() {
        let pending = HashMap::new();
        let running = HashSet::new();
        assert_eq!(
            acp_agent_selection_after_fetch(None, "s1", &pending, &running, None),
            Some(None)
        );
    }
}

pub(super) fn finish_pending_turn(
    pending: RwSignal<HashMap<String, usize>>,
    running: RwSignal<HashSet<String>>,
    id: &str,
) {
    let remaining = pending.with(|m| m.get(id).copied().unwrap_or(0));
    if remaining > 1 {
        pending.update(|m| {
            if let Some(n) = m.get_mut(id) {
                *n -= 1;
            }
        });
        return;
    }
    pending.update(|m| {
        m.remove(id);
    });
    running.update(|r| {
        r.remove(id);
    });
}

pub(super) fn clear_running_if_idle(
    pending: RwSignal<HashMap<String, usize>>,
    running: RwSignal<HashSet<String>>,
    id: &str,
) {
    if pending.with(|m| m.get(id).copied().unwrap_or(0)) == 0 {
        running.update(|r| {
            r.remove(id);
        });
    }
}

pub(super) fn strip_approval_pending(items: &mut Vec<ChatItem>) {
    items.retain(|i| {
        !matches!(
            i,
            ChatItem::ApprovalPending { .. } | ChatItem::AcpPermission { .. }
        )
    });
}

pub(super) fn upsert_review(items: &mut Vec<ChatItem>, report: ReviewReport) {
    if let Some(existing) = items
        .iter_mut()
        .find(|item| matches!(item, ChatItem::Review(current) if current.id == report.id))
    {
        *existing = ChatItem::Review(report);
    } else {
        let index = trailing_queue_start(items);
        items.insert(index, ChatItem::Review(report));
    }
}

pub(super) fn reviewer_backend_key(reviewer: &Specialist) -> String {
    match &reviewer.review_backend {
        Some(ReviewBackendConfig::FollowSession) => "follow_session".into(),
        Some(ReviewBackendConfig::AcpAgent { profile_id }) => format!("acp:{profile_id}"),
        Some(ReviewBackendConfig::HttpModel { profile_id }) => format!("http:{profile_id}"),
        None => format!("http:{}", reviewer.model_id),
    }
}

pub(super) fn set_reviewer_backend(reviewer: &mut Specialist, key: &str) {
    if key == "follow_session" {
        reviewer.review_backend = Some(ReviewBackendConfig::follow_session());
    } else if let Some(profile_id) = key.strip_prefix("acp:") {
        reviewer.review_backend = Some(ReviewBackendConfig::acp(profile_id));
    } else {
        let profile_id = key.strip_prefix("http:").unwrap_or(key);
        reviewer.model_id = profile_id.to_string();
        reviewer.review_backend = Some(ReviewBackendConfig::http(profile_id));
    }
}

pub(super) fn reviewer_backend_label(
    reviewer: &Specialist,
    models: &[ModelProfile],
    acp_agents: &[AcpAgentProfile],
    follow_session_label: &str,
    missing_acp_label: &str,
) -> Option<String> {
    match &reviewer.review_backend {
        Some(ReviewBackendConfig::FollowSession) => Some(follow_session_label.into()),
        Some(ReviewBackendConfig::AcpAgent { profile_id }) => Some(
            acp_agents
                .iter()
                .find(|profile| profile.id == *profile_id)
                .map(|profile| format!("{} · ACP", profile.label))
                .unwrap_or_else(|| format!("{missing_acp_label} · {profile_id}")),
        ),
        Some(ReviewBackendConfig::HttpModel { profile_id }) => {
            if profile_id.is_empty() {
                None
            } else {
                models
                    .iter()
                    .find(|profile| profile.id == *profile_id)
                    .map(|profile| profile.label.clone())
            }
        }
        None => {
            if reviewer.model_id.is_empty() {
                None
            } else {
                models
                    .iter()
                    .find(|profile| profile.id == reviewer.model_id)
                    .map(|profile| profile.label.clone())
            }
        }
    }
}

pub(super) fn reviewer_missing_acp_profile_id(
    reviewer: &Specialist,
    acp_agents: &[AcpAgentProfile],
) -> Option<String> {
    let Some(ReviewBackendConfig::AcpAgent { profile_id }) = &reviewer.review_backend else {
        return None;
    };
    (!acp_agents.iter().any(|profile| profile.id == *profile_id)).then(|| profile_id.clone())
}

#[cfg(test)]
mod review_tests {
    use super::{
        reviewer_backend_key, reviewer_backend_label, reviewer_missing_acp_profile_id,
        set_reviewer_backend, upsert_review,
    };
    use crate::dto::{AcpAgentProfile, ChatItem, ReviewBackendConfig, ReviewReport, Specialist};

    fn report(id: &str, summary: &str) -> ReviewReport {
        ReviewReport {
            id: id.into(),
            summary: summary.into(),
            findings: vec![],
            reviewer_model: "review-model".into(),
            reviewer_effort: String::new(),
            reviewer_backend: "http_model".into(),
            review_status: "passed".into(),
            evidence_coverage: 100,
            coverage_gaps: vec![],
        }
    }

    #[test]
    fn follow_up_review_replaces_the_original_card() {
        let mut items = vec![ChatItem::Assistant {
            text: "answer".into(),
            model: None,
            resources: Vec::new(),
        }];
        upsert_review(&mut items, report("r1", "first"));
        upsert_review(&mut items, report("r1", "verified"));

        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[1],
            ChatItem::Review(report) if report.summary == "verified"
        ));
    }

    #[test]
    fn reviewer_backend_keys_roundtrip_http_acp_and_follow_session() {
        let mut reviewer = Specialist {
            id: "reviewer".into(),
            name: "Reviewer".into(),
            icon: String::new(),
            color: String::new(),
            description: String::new(),
            instructions: String::new(),
            model_id: String::new(),
            review_backend: None,
            skills: Some(vec![]),
            connectors: Some(vec![]),
            builtin: true,
        };

        set_reviewer_backend(&mut reviewer, "acp:codex");
        assert_eq!(reviewer_backend_key(&reviewer), "acp:codex");
        assert_eq!(
            reviewer.review_backend,
            Some(ReviewBackendConfig::acp("codex"))
        );

        set_reviewer_backend(&mut reviewer, "http:review-model");
        assert_eq!(reviewer_backend_key(&reviewer), "http:review-model");
        assert_eq!(reviewer.model_id, "review-model");

        set_reviewer_backend(&mut reviewer, "follow_session");
        assert_eq!(reviewer_backend_key(&reviewer), "follow_session");
    }

    #[test]
    fn missing_acp_reviewer_stays_visible_instead_of_looking_like_http() {
        let mut reviewer = Specialist {
            id: "reviewer".into(),
            name: "Reviewer".into(),
            icon: String::new(),
            color: String::new(),
            description: String::new(),
            instructions: String::new(),
            model_id: String::new(),
            review_backend: None,
            skills: Some(vec![]),
            connectors: Some(vec![]),
            builtin: true,
        };
        set_reviewer_backend(&mut reviewer, "acp:deleted-profile");
        let agents = vec![AcpAgentProfile {
            id: "other".into(),
            label: "Other ACP".into(),
            command: "other-acp".into(),
            args: vec![],
        }];

        assert_eq!(
            reviewer_missing_acp_profile_id(&reviewer, &agents).as_deref(),
            Some("deleted-profile")
        );
        assert_eq!(
            reviewer_backend_label(
                &reviewer,
                &[],
                &agents,
                "Follow session backend",
                "Missing ACP Agent",
            )
            .as_deref(),
            Some("Missing ACP Agent · deleted-profile")
        );
    }
}

pub(super) fn is_error_assistant(item: &ChatItem) -> bool {
    matches!(item, ChatItem::Assistant { text, .. } if text.starts_with("Error: "))
}

pub(super) fn strip_error_at(items: &mut Vec<ChatItem>, idx: usize) {
    if idx < items.len() && is_error_assistant(&items[idx]) {
        items.remove(idx);
    }
}

pub(super) fn ensure_streaming_assistant(items: &mut Vec<ChatItem>, model: Option<String>) {
    let queue_start = trailing_queue_start(items);
    let has_blank = items[..queue_start]
        .iter()
        .rev()
        .any(|i| matches!(i, ChatItem::Assistant { text, .. } if text.trim().is_empty()));
    if !has_blank {
        items.insert(
            queue_start,
            ChatItem::Assistant {
                text: String::new(),
                model,
                resources: Vec::new(),
            },
        );
    }
}

pub(super) fn last_tool_input(items: &[ChatItem], tool: &str) -> String {
    items
        .iter()
        .rev()
        .find_map(|i| match i {
            ChatItem::Tool {
                name,
                input,
                ok: None,
                ..
            } if name == tool => Some(input.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

pub(super) fn trailing_queue_start(items: &[ChatItem]) -> usize {
    items
        .iter()
        .rposition(|item| !matches!(item, ChatItem::QueuedUser(_)))
        .map(|i| i + 1)
        .unwrap_or(0)
}

/// Insert ACP tool rows above the turn's assistant answer (and above the empty
/// streaming placeholder), so they coalesce into the same "Ran N steps" panel
/// as native Wisp tools instead of dangling under the finished reply.
pub(super) fn acp_tool_insert_index(items: &[ChatItem]) -> usize {
    let Some(user_idx) = items
        .iter()
        .rposition(|item| matches!(item, ChatItem::User(_)))
    else {
        return items.len();
    };
    for (offset, item) in items[user_idx + 1..].iter().enumerate() {
        match item {
            ChatItem::Reasoning(_) | ChatItem::AcpTool { .. } => {}
            ChatItem::Tool { name, .. } if name != "attempt_completion" => {}
            _ => return user_idx + 1 + offset,
        }
    }
    items.len()
}

#[cfg(test)]
mod acp_tool_insert_tests {
    use super::acp_tool_insert_index;
    use crate::dto::ChatItem;

    #[test]
    fn inserts_before_streaming_assistant_placeholder() {
        let items = vec![
            ChatItem::User("hi".into()),
            ChatItem::Assistant {
                text: String::new(),
                model: None,
                resources: Vec::new(),
            },
        ];
        assert_eq!(acp_tool_insert_index(&items), 1);
    }

    #[test]
    fn stacks_with_existing_acp_tools() {
        let items = vec![
            ChatItem::User("hi".into()),
            ChatItem::AcpTool {
                call_id: "1".into(),
                title: "ls".into(),
                kind: "execute".into(),
                status: "completed".into(),
                content: String::new(),
                locations: String::new(),
            },
            ChatItem::Assistant {
                text: "done".into(),
                model: Some("Codex".into()),
                resources: Vec::new(),
            },
        ];
        assert_eq!(acp_tool_insert_index(&items), 2);
    }
}

pub(super) fn start_user_turn(items: &mut Vec<ChatItem>, text: String, model: Option<String>) {
    let incoming_body = composer_text_from_user_message(&text);
    // ponytail: text-keyed promotion; upgrade to a backend intent_id if
    // display/echo texts ever diverge beyond the attachment suffix.
    if let Some((idx, queued)) = items.iter().enumerate().find_map(|(i, item)| match item {
        ChatItem::QueuedUser(queued)
            if queued == &text || composer_text_from_user_message(queued) == incoming_body =>
        {
            Some((i, queued.clone()))
        }
        _ => None,
    }) {
        // Prefer the longer display form, mirroring the ack path below.
        let display = if queued.len() > text.len() {
            queued
        } else {
            text
        };
        items.splice(
            idx..=idx,
            [
                ChatItem::User(display),
                ChatItem::Assistant {
                    text: String::new(),
                    model,
                    resources: Vec::new(),
                },
            ],
        );
    } else if let Some(idx) = items.windows(2).position(|pair| {
        matches!(
            &pair[0],
            ChatItem::User(s)
                if s == &text || composer_text_from_user_message(s) == incoming_body
        ) && matches!(&pair[1], ChatItem::Assistant { text: assistant, .. } if assistant.is_empty())
    }) {
        // Normal sends are rendered optimistically. The backend User event is
        // only an acknowledgement in that case, so do not append a duplicate.
        // Prefer the longer display form when one side still lacks the
        // "Uploaded files:" (or reference) suffix.
        if let ChatItem::User(existing) = &mut items[idx] {
            if text.len() > existing.len() {
                *existing = text;
            }
        }
    } else {
        items.push(ChatItem::User(text));
        items.push(ChatItem::Assistant {
            text: String::new(),
            model,
            resources: Vec::new(),
        });
    }
}

#[cfg(test)]
mod start_user_turn_tests {
    use super::{
        append_assistant_delta, append_reasoning_delta, composer_text_from_user_message,
        message_with_attachments, message_with_composer_context, message_with_quotes,
        runtime_object_quote, start_user_turn, trailing_queue_start, ComposerQuote,
        ComposerReferenceChip,
    };
    use crate::dto::ChatItem;

    #[test]
    fn message_with_attachments_appends_suffix() {
        assert_eq!(
            message_with_attachments("描述下图片", &["uploads/a.png".into()]),
            "描述下图片\n\nUploaded files: uploads/a.png"
        );
        assert_eq!(
            message_with_attachments("  ", &["uploads/a.png".into()]),
            "Uploaded files: uploads/a.png"
        );
    }

    #[test]
    fn message_with_context_keeps_reference_labels_for_transcript_ui() {
        let refs = vec![
            ComposerReferenceChip::Artifact {
                id: "a1".into(),
                name: "counts.csv".into(),
            },
            ComposerReferenceChip::Session {
                id: "s1".into(),
                title: "QC review".into(),
                project_name: "Atlas".into(),
            },
            ComposerReferenceChip::Skill {
                name: "bear-review".into(),
            },
            ComposerReferenceChip::Context {
                id: "ssh:cpu1".into(),
                label: "CPU1".into(),
            },
            ComposerReferenceChip::Runtime {
                context_id: "local".into(),
                context_label: "Local".into(),
                language: "r".into(),
            },
        ];
        assert_eq!(
            message_with_composer_context(
                "Compare these",
                &["uploads/plot.png".into()],
                &refs,
                &[]
            ),
            "Compare these\n\nUploaded files: uploads/plot.png\n\nAttached artifacts: counts.csv\n\nAttached sessions: Atlas / QC review\n\nSelected skills: bear-review\n\nTarget environments: CPU1\n\nTarget runtimes: R · Local"
        );
    }

    #[test]
    fn runtime_object_quote_skips_placeholder_fields() {
        assert_eq!(
            runtime_object_quote("Python", "df", "DataFrame", "(100, 3)", "2.3 MB"),
            "[Python runtime] df: DataFrame = (100, 3) (2.3 MB)"
        );
        assert_eq!(
            runtime_object_quote("R", "fit", "lm", "—", "—"),
            "[R runtime] fit: lm"
        );
    }

    #[test]
    fn message_with_quotes_prefixes_blockquotes() {
        assert_eq!(
            message_with_quotes(
                "这是什么意思?",
                &[ComposerQuote::plain("line one\nline two")]
            ),
            "> line one\n> line two\n\n这是什么意思?"
        );
        assert_eq!(message_with_quotes("plain", &[]), "plain");
        assert_eq!(
            message_with_quotes("", &[ComposerQuote::plain("ctx")]),
            "> ctx"
        );
    }

    #[test]
    fn workspace_quote_carries_an_actionable_edit_target() {
        let message = message_with_quotes(
            "改成散点图",
            &[ComposerQuote::from_selection(
                "plot(1:3)",
                Some("analysis.R".into()),
            )],
        );
        assert!(message.starts_with(
            "Selected excerpt from workspace file `analysis.R`:\n> plot(1:3)\n\n改成散点图"
        ));
        assert!(message.contains("read the selected workspace file first"));
        assert!(message.contains("edit tool"));
        assert!(message.ends_with("Target file: `analysis.R`"));
    }

    #[test]
    fn immutable_reference_quote_does_not_request_a_file_edit() {
        let message = message_with_quotes(
            "解释一下",
            &[ComposerQuote::from_selection(
                "result",
                Some("artifact:report".into()),
            )],
        );
        assert!(message.starts_with("Selected excerpt from reference `artifact:report`:"));
        assert!(!message.contains("AI source-edit instruction"));

        let binary = message_with_quotes(
            "改一下",
            &[ComposerQuote::from_selection(
                "rendered text",
                Some("manuscript.docx".into()),
            )],
        );
        assert!(binary.starts_with("Selected excerpt from reference `manuscript.docx`:"));
        assert!(!binary.contains("AI source-edit instruction"));
    }

    #[test]
    fn does_not_duplicate_when_backend_acks_bare_body() {
        let display = message_with_attachments("图片里有啥文字?", &["uploads/img.png".into()]);
        let mut items = vec![
            ChatItem::User(display.clone()),
            ChatItem::Assistant {
                text: String::new(),
                model: Some("gpt".into()),
                resources: Vec::new(),
            },
        ];
        start_user_turn(&mut items, "图片里有啥文字?".into(), Some("gpt".into()));
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], ChatItem::User(s) if s == &display));
    }

    #[test]
    fn upgrades_optimistic_row_when_ack_has_suffix() {
        let display = message_with_attachments("描述下图片", &["uploads/img.png".into()]);
        let mut items = vec![
            ChatItem::User("描述下图片".into()),
            ChatItem::Assistant {
                text: String::new(),
                model: None,
                resources: Vec::new(),
            },
        ];
        start_user_turn(&mut items, display.clone(), None);
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], ChatItem::User(s) if s == &display));
        assert_eq!(composer_text_from_user_message(&display), "描述下图片");
    }

    #[test]
    fn acp_thinking_joins_tool_steps_above_a_started_reply() {
        // Codex-style order: a short reply streams first, then thinking, then
        // ACP tools (which are hoisted above the reply). Thinking must land in
        // the same process region so it folds into the steps panel instead of
        // dangling under the reply (issue: ACP run/thinking rendered apart).
        let mut items = vec![
            ChatItem::User("查下文献".into()),
            ChatItem::Assistant {
                text: "我先查近年文献".into(),
                model: Some("Codex".into()),
                resources: Vec::new(),
            },
        ];

        append_reasoning_delta(&mut items, "Searching for literature.".into());
        // A subsequent ACP tool inserts at the same process-region anchor.
        let idx = super::acp_tool_insert_index(&items);
        items.insert(
            idx,
            ChatItem::AcpTool {
                call_id: "1".into(),
                title: "web_search".into(),
                kind: "search".into(),
                status: "in_progress".into(),
                content: String::new(),
                locations: String::new(),
            },
        );

        // Reasoning + tool sit consecutively before the reply → one panel.
        assert!(matches!(&items[0], ChatItem::User(_)));
        assert!(matches!(&items[1], ChatItem::Reasoning(t) if t == "Searching for literature."));
        assert!(matches!(&items[2], ChatItem::AcpTool { .. }));
        assert!(matches!(&items[3], ChatItem::Assistant { text, .. } if text == "我先查近年文献"));
    }

    #[test]
    fn native_thinking_precedes_reply_and_stays_at_the_tail() {
        // Native reasoning models emit thinking before any reply, so the hoist
        // must not fire: thinking appends at the tail next to the placeholder.
        let mut items = vec![
            ChatItem::User("hi".into()),
            ChatItem::Assistant {
                text: String::new(),
                model: None,
                resources: Vec::new(),
            },
        ];

        append_reasoning_delta(&mut items, "Let me think.".into());

        assert!(matches!(&items[1], ChatItem::Assistant { text, .. } if text.is_empty()));
        assert!(matches!(&items[2], ChatItem::Reasoning(t) if t == "Let me think."));
    }

    #[test]
    fn active_deltas_stay_before_queued_turns() {
        let mut items = vec![
            ChatItem::User("alpha".into()),
            ChatItem::Assistant {
                text: "echo:alpha".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::QueuedUser("queued".into()),
        ];

        assert_eq!(trailing_queue_start(&items), 2);
        append_assistant_delta(&mut items, ":tail".into(), None);

        assert!(matches!(
            &items[1],
            ChatItem::Assistant { text, .. } if text == "echo:alpha:tail"
        ));
        assert!(matches!(&items[2], ChatItem::QueuedUser(text) if text == "queued"));
    }

    #[test]
    fn promotes_queued_turn_when_backend_acks_bare_body() {
        let display = message_with_attachments("图片里有啥文字?", &["uploads/img.png".into()]);
        let mut items = vec![
            ChatItem::User("alpha".into()),
            ChatItem::Assistant {
                text: "done".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::QueuedUser(display.clone()),
        ];

        start_user_turn(&mut items, "图片里有啥文字?".into(), None);

        assert_eq!(items.len(), 4);
        assert!(matches!(&items[2], ChatItem::User(s) if s == &display));
        assert!(matches!(
            &items[3],
            ChatItem::Assistant { text, .. } if text.is_empty()
        ));
    }

    #[test]
    fn backend_user_event_promotes_the_matching_queued_turn() {
        let mut items = vec![
            ChatItem::User("alpha".into()),
            ChatItem::Assistant {
                text: "done".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::QueuedUser("queued".into()),
            ChatItem::QueuedUser("later".into()),
        ];

        start_user_turn(&mut items, "queued".into(), Some("model".into()));

        assert!(matches!(&items[2], ChatItem::User(text) if text == "queued"));
        assert!(matches!(
            &items[3],
            ChatItem::Assistant { text, model, .. } if text.is_empty() && model.as_deref() == Some("model")
        ));
        assert!(matches!(&items[4], ChatItem::QueuedUser(text) if text == "later"));
    }
}

#[cfg(test)]
mod upload_attachment_tests {
    use super::{merge_finished_uploads, ready_attachment_key};
    use crate::dto::{ArtifactInfo, ComposerAttachment, UploadFileResult};

    fn ok_result(path: &str) -> UploadFileResult {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
        UploadFileResult {
            ok: true,
            info: Some(ArtifactInfo {
                id: "artifact-1".into(),
                name: name.clone(),
                kind: "file".into(),
                path: path.into(),
                ts: 0,
                project_id: None,
                project_name: None,
                session_id: None,
                session_title: None,
                size_bytes: None,
                origin: None,
            }),
            filename: Some(name),
            error: None,
        }
    }

    #[test]
    fn merge_finished_uploads_dedupes_duplicate_ready_paths() {
        let mut items = vec![ComposerAttachment::Uploading {
            key: "up-1".into(),
            name: String::new(),
        }];

        merge_finished_uploads(
            &mut items,
            vec![
                ok_result("uploads/s41467-026-73270-2_reference.pdf"),
                ok_result("uploads/s41467-026-73270-2_reference.pdf"),
            ],
        );

        assert_eq!(items.len(), 1);
        assert!(matches!(
            &items[0],
            ComposerAttachment::Ready { path, .. }
                if path == "uploads/s41467-026-73270-2_reference.pdf"
        ));
    }

    #[test]
    fn merge_finished_uploads_keeps_existing_ready_path_unique() {
        let path = "uploads/existing.pdf";
        let mut items = vec![ComposerAttachment::Ready {
            key: ready_attachment_key(path),
            name: "existing.pdf".into(),
            path: path.into(),
        }];

        merge_finished_uploads(&mut items, vec![ok_result(path)]);

        assert_eq!(items.len(), 1);
        assert!(matches!(
            &items[0],
            ComposerAttachment::Ready { path, .. } if path == "uploads/existing.pdf"
        ));
    }
}

pub(super) fn append_assistant_delta(
    items: &mut Vec<ChatItem>,
    delta: String,
    model: Option<String>,
) {
    let queue_start = trailing_queue_start(items);
    if let Some(idx) = items[..queue_start]
        .iter()
        .rposition(|item| matches!(item, ChatItem::Assistant { .. }))
    {
        if let ChatItem::Assistant { text, .. } = &mut items[idx] {
            text.push_str(&delta);
            return;
        }
    }
    items.insert(
        queue_start,
        ChatItem::Assistant {
            text: delta,
            model,
            resources: Vec::new(),
        },
    );
}

pub(super) fn append_reasoning_delta(items: &mut Vec<ChatItem>, delta: String) {
    let queue_start = trailing_queue_start(items);
    if let Some(idx) = items[..queue_start]
        .iter()
        .rposition(|item| matches!(item, ChatItem::Reasoning(_)))
    {
        if let ChatItem::Reasoning(text) = &mut items[idx] {
            text.push_str(&delta);
            return;
        }
    }
    // ACP agents (e.g. Codex) stream a short reply first, THEN thinking, THEN
    // tools. Tool rows are hoisted above that reply (acp_tool_insert_index) so
    // they fold into one "Ran N steps" panel; thinking must join them there
    // instead of dangling under the reply. A non-empty reply already present in
    // the turn is that signature — native reasoning always precedes the reply,
    // so this never fires for native turns.
    // ponytail: only covers reply-before-thinking; an ACP agent that emits
    // thinking as its very first event still lands at the tail until a tool
    // arrives — revisit if such an agent shows up.
    let insert_at = if turn_has_started_reply(&items[..queue_start]) {
        acp_tool_insert_index(items)
    } else {
        queue_start
    };
    items.insert(insert_at, ChatItem::Reasoning(delta));
}

/// True when the active turn (after the last user message) already carries a
/// non-empty assistant reply — i.e. the agent spoke before it started thinking.
fn turn_has_started_reply(turn: &[ChatItem]) -> bool {
    let start = turn
        .iter()
        .rposition(|item| matches!(item, ChatItem::User(_)))
        .map(|u| u + 1)
        .unwrap_or(0);
    turn[start..]
        .iter()
        .any(|item| matches!(item, ChatItem::Assistant { text, .. } if !text.trim().is_empty()))
}

pub(super) fn append_stdout_chunk(items: &mut Vec<ChatItem>, chunk: String) {
    let queue_start = trailing_queue_start(items);
    if let Some(idx) = items[..queue_start]
        .iter()
        .rposition(|item| matches!(item, ChatItem::Tool { .. }))
    {
        if let ChatItem::Tool { output, .. } = &mut items[idx] {
            output.push_str(&chunk);
            return;
        }
    }
    items.insert(
        queue_start,
        ChatItem::Tool {
            name: "stdout".into(),
            ok: None,
            input: String::new(),
            output: chunk,
            started_at_ms: None,
            duration_ms: None,
        },
    );
}

// --- Streaming delta batching (#65) ------------------------------------------
//
// The backend emits one `agent` event per LLM/stdout chunk. Applying each one
// writes the `items` signal, and every write re-runs the thread view and the
// artifact scan — O(conversation length) work per token, which freezes long
// conversations. Buffer the append-only deltas and flush them on a short timer
// so the signal is written at most ~20×/s regardless of token rate.

pub(super) enum PendingDelta {
    Text(String),
    Reasoning(String),
    Stdout(String),
}

pub(super) type DeltaBuf = Rc<RefCell<HashMap<String, Vec<PendingDelta>>>>;

/// Append a delta to a session's queue, coalescing consecutive same-kind chunks.
pub(super) fn queue_delta(buf: &DeltaBuf, fid: String, d: PendingDelta) {
    let mut map = buf.borrow_mut();
    let q = map.entry(fid).or_default();
    match (q.last_mut(), d) {
        (Some(PendingDelta::Text(s)), PendingDelta::Text(n)) => s.push_str(&n),
        (Some(PendingDelta::Reasoning(s)), PendingDelta::Reasoning(n)) => s.push_str(&n),
        (Some(PendingDelta::Stdout(s)), PendingDelta::Stdout(n)) => s.push_str(&n),
        (_, d) => q.push(d),
    }
}

/// Apply all buffered deltas to their sessions in arrival order.
pub(super) fn flush_delta_buf(
    buf: &DeltaBuf,
    active: RwSignal<Option<String>>,
    items: RwSignal<Vec<ChatItem>>,
    transcripts: RwSignal<HashMap<String, Vec<ChatItem>>>,
    models: RwSignal<Vec<ModelProfile>>,
) {
    let drained: Vec<_> = buf.borrow_mut().drain().collect();
    if drained.is_empty() {
        return;
    }
    let fallback_model = active_model_label(&models.get_untracked());
    for (fid, deltas) in drained {
        let model = fallback_model.clone();
        route_items(active, items, transcripts, &fid, move |v| {
            for d in deltas {
                match d {
                    PendingDelta::Text(s) => append_assistant_delta(v, s, model.clone()),
                    PendingDelta::Reasoning(s) => append_reasoning_delta(v, s),
                    PendingDelta::Stdout(s) => append_stdout_chunk(v, s),
                }
            }
        });
    }
}

pub(super) fn schedule_delta_flush(
    buf: &DeltaBuf,
    scheduled: &Rc<Cell<bool>>,
    active: RwSignal<Option<String>>,
    items: RwSignal<Vec<ChatItem>>,
    transcripts: RwSignal<HashMap<String, Vec<ChatItem>>>,
    models: RwSignal<Vec<ModelProfile>>,
) {
    if scheduled.get() {
        return;
    }
    scheduled.set(true);
    let buf = buf.clone();
    let scheduled = scheduled.clone();
    set_timeout(
        move || {
            scheduled.set(false);
            flush_delta_buf(&buf, active, items, transcripts, models);
        },
        std::time::Duration::from_millis(50),
    );
}

pub(super) fn format_relative_time(ts: i64, locale: Locale) -> String {
    if ts <= 0 {
        return String::new();
    }
    let now_ms = js_sys::Date::now();
    let ts_ms = if ts > 1_000_000_000_000 {
        ts as f64
    } else {
        ts as f64 * 1000.0
    };
    let secs = ((now_ms - ts_ms) / 1000.0).max(0.0) as i64;
    if secs < 45 {
        return t(locale, "time.just_now").into();
    }
    if secs < 3600 {
        return tf(
            locale,
            "time.minutes",
            &[("n", &(secs / 60).max(1).to_string())],
        );
    }
    if secs < 86_400 {
        return tf(locale, "time.hours", &[("n", &(secs / 3600).to_string())]);
    }
    tf(locale, "time.days", &[("n", &(secs / 86_400).to_string())])
}

#[component]
pub(super) fn SessionStatusBadge(
    status: SessionStatusKind,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let key = status.i18n_key();
    let class = status.css();
    view! {
        <span class=format!("sess-status sess-status-{class}")>
            {move || t(locale.get(), key)}
        </span>
    }
}

pub(super) fn profile_to_form(m: &ModelProfile) -> ModelForm {
    ModelForm {
        id: Some(m.id.clone()),
        label: m.label.clone(),
        provider: m.provider.clone(),
        api_url: m.api_url.clone(),
        model: m.model.clone(),
        max_tokens: if m.max_tokens >= 16 {
            m.max_tokens
        } else {
            8192
        },
        reasoning_effort: m.reasoning_effort.clone(),
        supports_vision: m.supports_vision,
        use_for_vision: m.use_for_vision,
    }
}

pub(super) fn new_model_form() -> ModelForm {
    let (api_url, model) = provider_defaults("openai");
    ModelForm {
        provider: "openai".into(),
        api_url: api_url.into(),
        model: model.into(),
        max_tokens: 8192,
        ..Default::default()
    }
}

pub(super) fn new_acp_form() -> AcpAgentProfile {
    AcpAgentProfile {
        id: String::new(),
        label: String::new(),
        command: String::new(),
        args: Vec::new(),
    }
}

pub(super) fn model_form_to_settings(form: &ModelForm, has_api_key: bool) -> Settings {
    let mut cfg = Settings::default();
    cfg.provider = provider_value(&form.provider).into();
    cfg.api_url = form.api_url.trim().into();
    cfg.model = form.model.trim().into();
    cfg.label = form.label.trim().into();
    cfg.has_api_key = has_api_key;
    cfg.max_tokens = form.max_tokens;
    cfg.reasoning_effort = form.reasoning_effort.clone();
    cfg.supports_vision = form.supports_vision;
    cfg
}

pub(super) fn settings_section_label(loc: Locale, section: &str) -> String {
    match section {
        "appearance" => t(loc, "settings.nav.appearance"),
        "pet" => t(loc, "settings.nav.pet"),
        "environments" => t(loc, "settings.nav.environments"),
        "models" => t(loc, "settings.nav.models"),
        "specialists" => t(loc, "settings.nav.specialists"),
        "memory" => t(loc, "settings.nav.memory"),
        "skills" => t(loc, "settings.nav.skills"),
        "connections" => t(loc, "settings.nav.connections"),
        "channels" => t(loc, "settings.nav.channels"),
        "credentials" => t(loc, "settings.nav.credentials"),
        "permissions" => t(loc, "settings.nav.permissions"),
        _ => t(loc, "settings.title"),
    }
    .into()
}

/// A field within a credential service group: (credential id, i18n label key,
/// whether to mask the value like a password).
pub(super) struct CredField {
    pub(super) id: &'static str,
    pub(super) label_key: &'static str,
    pub(super) secret: bool,
}

/// A credential service shown in Settings → Credentials: display name, help
/// text, and its fields. Mirrors the backend `CREDENTIALS` registry in
/// models.rs — keep ids in sync.
pub(super) struct CredGroup {
    pub(super) name_key: &'static str,
    pub(super) hint_key: &'static str,
    pub(super) fields: &'static [CredField],
}

pub(super) const CRED_GROUPS: &[CredGroup] = &[
    CredGroup {
        name_key: "cred.openalex.name",
        hint_key: "cred.openalex.hint",
        fields: &[CredField {
            id: "openalex_api_key",
            label_key: "cred.openalex_api_key.label",
            secret: true,
        }],
    },
    CredGroup {
        name_key: "cred.infinisynapse.name",
        hint_key: "cred.infinisynapse.hint",
        fields: &[CredField {
            id: "infinisynapse_api_key",
            label_key: "cred.infinisynapse_api_key.label",
            secret: true,
        }],
    },
    CredGroup {
        name_key: "cred.scimaster.name",
        hint_key: "cred.scimaster.hint",
        fields: &[CredField {
            id: "scimaster_api_key",
            label_key: "cred.scimaster_api_key.label",
            secret: true,
        }],
    },
    CredGroup {
        name_key: "cred.ncbi.name",
        hint_key: "cred.ncbi.hint",
        fields: &[
            CredField {
                id: "ncbi_api_key",
                label_key: "cred.ncbi_api_key.label",
                secret: true,
            },
            CredField {
                id: "ncbi_email",
                label_key: "cred.ncbi_email.label",
                secret: false,
            },
        ],
    },
];

pub(super) fn settings_subpage_label(
    loc: Locale,
    section: &str,
    model_form: Option<&ModelForm>,
    conn_form: Option<&ConnForm>,
    open_conn: Option<&str>,
    memory_selected: Option<&str>,
    specialist_form: Option<&Specialist>,
    acp_form: Option<&AcpAgentProfile>,
    channels_open: Option<&str>,
) -> Option<String> {
    match section {
        "models" => acp_form
            .map(|f| {
                if f.id.is_empty() {
                    t(loc, "models.add_acp").into()
                } else {
                    t(loc, "models.edit_acp").into()
                }
            })
            .or_else(|| {
                model_form.map(|f| {
                    if f.id.is_some() {
                        t(loc, "models.edit").into()
                    } else {
                        t(loc, "models.add").into()
                    }
                })
            }),
        "specialists" => specialist_form.map(|s| {
            if s.id.is_empty() {
                t(loc, "specialists.add")
            } else {
                s.name.clone()
            }
        }),
        "connections" => conn_form
            .map(|f| {
                if f.id.is_some() {
                    t(loc, "conn.edit").into()
                } else {
                    t(loc, "conn.add").into()
                }
            })
            .or_else(|| open_conn.map(|s| s.to_string())),
        "memory" => memory_selected.map(|s| s.to_string()),
        "channels" => channels_open.map(|key| match key {
            "feishu" => t(loc, "channels.feishu.title").into(),
            "weixin" => t(loc, "channels.weixin.title").into(),
            other => other.to_string(),
        }),
        _ => None,
    }
}

pub(super) fn build_conn_json(f: &ConnForm, assign_id: bool) -> serde_json::Value {
    let id = f.id.clone().unwrap_or_else(|| {
        if assign_id {
            format!("conn-{}", (js_sys::Math::random() * 1e9) as u64)
        } else {
            "test".into()
        }
    });
    let transport = if f.kind == "http" {
        let headers: Vec<(String, String)> = f
            .headers
            .lines()
            .filter_map(|l| {
                l.split_once(':')
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        let auth = if f.auth == "oauth" { "oauth" } else { "none" };
        serde_json::json!({ "kind": "http", "url": f.url.trim(), "headers": headers, "auth": auth })
    } else {
        let args: Vec<String> = f.args.split_whitespace().map(|s| s.to_string()).collect();
        serde_json::json!({ "kind": "stdio", "command": f.command.trim(), "args": args, "env": [], "cwd": null })
    };
    serde_json::json!({ "id": id, "name": f.name.trim(), "enabled": f.enabled, "transport": transport })
}

pub(super) fn conn_form_from_row(row: &ConnRow) -> ConnForm {
    match &row.transport {
        ConnTransport::Stdio { command, args, .. } => ConnForm {
            id: Some(row.id.clone()),
            name: row.name.clone(),
            kind: "stdio".into(),
            command: command.clone(),
            args: args.join(" "),
            url: String::new(),
            headers: String::new(),
            auth: "none".into(),
            enabled: row.enabled,
        },
        ConnTransport::Http { url, headers, auth } => ConnForm {
            id: Some(row.id.clone()),
            name: row.name.clone(),
            kind: "http".into(),
            command: String::new(),
            args: String::new(),
            url: url.clone(),
            headers: headers
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n"),
            auth: if auth == "oauth" {
                "oauth".into()
            } else {
                "none".into()
            },
            enabled: row.enabled,
        },
    }
}

pub(super) fn refresh_dir(cwd: RwSignal<String>, entries: RwSignal<Vec<DirEntry>>) {
    spawn_local(async move {
        let path = cwd.get();
        let v = invoke(
            "list_dir",
            to_value(&serde_json::json!({ "path": path })).unwrap(),
        )
        .await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DirEntry>>(v) {
            entries.set(list);
        }
    });
}

pub(super) fn refresh_remote_dir(
    context_id: String,
    cwd: RwSignal<String>,
    entries: RwSignal<Vec<DirEntry>>,
    loading: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    active_source: RwSignal<String>,
) {
    let requested_path = cwd.get_untracked();
    entries.set(vec![]);
    error.set(None);
    loading.set(true);
    spawn_local(async move {
        let result = invoke_checked(
            "list_remote_dir",
            to_value(&serde_json::json!({
                "contextId": context_id.clone(),
                "path": requested_path.clone(),
            }))
            .unwrap(),
        )
        .await;
        if active_source.get_untracked() != context_id || cwd.get_untracked() != requested_path {
            return;
        }
        loading.set(false);
        match result {
            Ok(value) => match serde_wasm_bindgen::from_value::<DirectoryListing>(value) {
                Ok(listing) => {
                    cwd.set(listing.path);
                    entries.set(listing.entries);
                }
                Err(parse_error) => error.set(Some(parse_error.to_string())),
            },
            Err(invoke_error) => error.set(Some(js_error_text(invoke_error))),
        }
    });
}

pub(super) fn refresh_active_file_dir(
    source: RwSignal<String>,
    local_cwd: RwSignal<String>,
    local_entries: RwSignal<Vec<DirEntry>>,
    remote_cwd: RwSignal<String>,
    remote_entries: RwSignal<Vec<DirEntry>>,
    remote_loading: RwSignal<bool>,
    remote_error: RwSignal<Option<String>>,
) {
    let context_id = source.get_untracked();
    if context_id == "local" {
        refresh_dir(local_cwd, local_entries);
    } else {
        refresh_remote_dir(
            context_id,
            remote_cwd,
            remote_entries,
            remote_loading,
            remote_error,
            source,
        );
    }
}

pub(super) fn refresh_file_search(query: RwSignal<String>, hits: RwSignal<Vec<FileSearchHit>>) {
    spawn_local(async move {
        let q = query.get().trim().to_string();
        if q.is_empty() {
            hits.set(vec![]);
            return;
        }
        let v = invoke(
            "search_files",
            to_value(&serde_json::json!({ "query": q, "limit": 200 })).unwrap(),
        )
        .await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<FileSearchHit>>(v) {
            hits.set(list);
        }
    });
}

pub(super) fn refresh_artifact_search(query: RwSignal<String>, hits: RwSignal<Vec<ArtifactInfo>>) {
    spawn_local(async move {
        let q = query.get().trim().to_string();
        let v = invoke(
            "search_artifacts",
            to_value(&serde_json::json!({ "query": q, "limit": 12 })).unwrap(),
        )
        .await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ArtifactInfo>>(v) {
            hits.set(list);
        }
    });
}

pub(super) fn artifact_badge(kind: &str, name: &str) -> String {
    let raw = name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| !ext.is_empty() && ext.len() <= 10)
        .or_else(|| kind.rsplit('/').next())
        .unwrap_or("file");
    raw.to_uppercase()
}

pub(super) fn stored_artifact_path(path: &str) -> String {
    path.strip_prefix("file://").unwrap_or(path).to_string()
}

pub(super) fn contains_search(q: &str, parts: &[&str]) -> bool {
    q.is_empty() || parts.iter().any(|s| s.to_lowercase().contains(q))
}

pub(super) type ModalArtifact = (String, String, String);

#[derive(Clone, PartialEq, Eq)]
pub(super) struct CenterFileTab {
    pub(super) path: String,
    pub(super) name: String,
    pub(super) kind: String,
}

impl CenterFileTab {
    pub(super) fn new(path: String, name: String, kind: String) -> Self {
        Self { path, name, kind }
    }

    pub(super) fn from_path(path: String) -> Self {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
        let kind = file_kind(&path).unwrap_or("text").to_string();
        Self { path, name, kind }
    }
}

/// Keys a live file-change event can use to address an open center tab. Tools
/// may report either the relative argument the model supplied or the resolved
/// absolute path; normalize both POSIX and Windows separators for matching.
pub(super) fn file_change_refresh_keys(path: &str, project_root: Option<&str>) -> Vec<String> {
    let mut keys = Vec::new();
    let mut push = |value: String| {
        if !value.is_empty() && !keys.contains(&value) {
            keys.push(value);
        }
    };
    push(path.to_string());
    let normalized = path.replace('\\', "/");
    push(normalized.clone());
    if let Some(relative) = normalized.strip_prefix("./") {
        push(relative.to_string());
        push(relative.replace('/', "\\"));
    }
    if let Some(root) = project_root {
        let normalized_root = root.replace('\\', "/");
        let normalized_root = normalized_root.trim_end_matches('/');
        if let Some(relative) = normalized.strip_prefix(normalized_root).and_then(|tail| {
            tail.strip_prefix('/')
                .filter(|relative| !relative.is_empty())
        }) {
            push(relative.to_string());
            push(relative.replace('/', "\\"));
        }
    }
    keys
}

#[cfg(test)]
mod file_change_refresh_keys_tests {
    use super::file_change_refresh_keys;

    #[test]
    fn matches_relative_and_absolute_workspace_paths() {
        assert_eq!(
            file_change_refresh_keys("analysis.R", Some("/work/project")),
            ["analysis.R"]
        );
        assert!(
            file_change_refresh_keys("./analysis.R", Some("/work/project"))
                .contains(&"analysis.R".to_string())
        );
        let unix = file_change_refresh_keys("/work/project/src/analysis.R", Some("/work/project"));
        assert!(unix.contains(&"src/analysis.R".to_string()));

        let windows =
            file_change_refresh_keys(r"C:\work\project\src\analysis.R", Some(r"C:\work\project"));
        assert!(windows.contains(&"src/analysis.R".to_string()));
        assert!(windows.contains(&r"src\analysis.R".to_string()));
    }
}

pub(super) fn open_workspace_file(path: String, modal_artifact: RwSignal<Option<ModalArtifact>>) {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
    let kind = file_kind(&path).unwrap_or("text").to_string();
    modal_artifact.set(Some((path, name, kind)));
}

pub(super) fn modal_image_nav_targets(
    artifacts: &[Artifact],
    current_path: &str,
    current_kind: &str,
) -> (Option<ModalArtifact>, Option<ModalArtifact>) {
    if current_kind != "image" {
        return (None, None);
    }
    let images = artifacts
        .iter()
        .filter_map(|artifact| match &artifact.data {
            PreviewData::File { path, kind } if kind == "image" => {
                Some((path.clone(), artifact.name.clone(), kind.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let Some(index) = images.iter().position(|(path, _, _)| path == current_path) else {
        return (None, None);
    };
    let prev = index
        .checked_sub(1)
        .and_then(|idx| images.get(idx).cloned());
    let next = images.get(index + 1).cloned();
    (prev, next)
}

pub(super) const ALL_RIGHT_TABS: [RightTab; 7] = [
    RightTab::Artifacts,
    RightTab::Notebook,
    RightTab::Highlights,
    RightTab::File,
    RightTab::Provenance,
    RightTab::Hosts,
    RightTab::SideChat,
];

pub(super) const DEFAULT_RIGHT_TABS: [RightTab; 3] =
    [RightTab::Artifacts, RightTab::File, RightTab::Hosts];

pub(super) fn ensure_right_tab(
    tab: RightTab,
    show_right: RwSignal<bool>,
    open_right_tabs: RwSignal<Vec<RightTab>>,
    right_tab: RwSignal<RightTab>,
) {
    show_right.set(true);
    open_right_tabs.update(|tabs| {
        if !tabs.iter().any(|t| *t == tab) {
            tabs.push(tab);
        }
    });
    right_tab.set(tab);
}

pub(super) fn close_right_tab(
    tab: RightTab,
    show_right: RwSignal<bool>,
    open_right_tabs: RwSignal<Vec<RightTab>>,
    right_tab: RwSignal<RightTab>,
) {
    let was_active = right_tab.get_untracked() == tab;
    let prev_idx = open_right_tabs
        .get_untracked()
        .iter()
        .position(|t| *t == tab);
    open_right_tabs.update(|tabs| tabs.retain(|t| *t != tab));
    let remaining = open_right_tabs.get_untracked();
    if remaining.is_empty() {
        show_right.set(false);
        return;
    }
    if was_active {
        let pick = prev_idx
            .map(|i| if i > 0 { i - 1 } else { 0 })
            .unwrap_or(0)
            .min(remaining.len() - 1);
        right_tab.set(remaining[pick]);
    }
}

pub(super) fn reveal_in_files(
    path: &str,
    file_source: RwSignal<String>,
    file_cwd: RwSignal<String>,
    file_query: RwSignal<String>,
    file_entries: RwSignal<Vec<DirEntry>>,
    show_right: RwSignal<bool>,
    open_right_tabs: RwSignal<Vec<RightTab>>,
    right_tab: RwSignal<RightTab>,
) {
    file_source.set("local".into());
    file_query.set(String::new());
    file_cwd.set(parent_path(path));
    refresh_dir(file_cwd, file_entries);
    ensure_right_tab(RightTab::File, show_right, open_right_tabs, right_tab);
}

pub(super) fn file_dir_label(path: &str) -> String {
    let p = parent_path(path);
    if p == "." {
        String::new()
    } else {
        format!("{p}/")
    }
}

pub(super) fn art_label(a: &Artifact) -> String {
    if a.name.len() <= 28 {
        a.name.clone()
    } else {
        format!("artifact-{}", &a.id[..8.min(a.id.len())])
    }
}

pub(super) fn art_chip(idx: usize, a: &Artifact) -> String {
    let label = html_escape(&art_label(a));
    let title = html_escape(&a.name);
    format!(
        r#"<button type="button" class="art-ref" data-art-idx="{idx}" title="{title}">{label}</button>"#
    )
}

pub(super) fn artifact_file_paths(a: &Artifact) -> Vec<String> {
    match &a.data {
        PreviewData::File { path, .. } => {
            let mut out = vec![normalize_path(path)];
            if let Some(name) = path.rsplit(['/', '\\']).next() {
                let name = normalize_path(name);
                if !out.contains(&name) {
                    out.push(name);
                }
            }
            out
        }
        _ => vec![normalize_path(&a.name)],
    }
}

pub(super) fn href_matches_artifact(href: &str, a: &Artifact) -> bool {
    let h = normalize_path(href);
    artifact_file_paths(a).iter().any(|p| *p == h)
}

pub(super) fn artifact_index_for_href(arts: &[Artifact], href: &str) -> Option<usize> {
    arts.iter().position(|a| href_matches_artifact(href, a))
}

pub(super) fn replace_file_links(html: String, arts: &[Artifact]) -> String {
    let mut out = String::new();
    let mut rest = html.as_str();
    while let Some(ai) = rest.find("<a ") {
        out.push_str(&rest[..ai]);
        rest = &rest[ai..];
        let Some(gt) = rest.find('>') else {
            out.push_str(rest);
            break;
        };
        let tag = &rest[..=gt];
        let after = &rest[gt + 1..];
        let Some(end) = after.find("</a>") else {
            out.push_str(rest);
            break;
        };
        let inner = &after[..end];
        rest = &after[end + 4..];

        if let Some(href) = extract_href_from_tag(tag).map(|h| decode_href(&h)) {
            if !is_external_href(&href) {
                if let Some(idx) = artifact_index_for_href(arts, &href) {
                    out.push_str(&art_chip(idx, &arts[idx]));
                    continue;
                }
            }
        }
        out.push_str(tag);
        out.push_str(inner);
        out.push_str("</a>");
    }
    out.push_str(rest);
    out
}

pub(super) fn artifact_matches_token(token: &str, id: &str) -> bool {
    let t = token.trim();
    t == id
        || t.starts_with(id)
        || id.starts_with(&t[..t.len().min(8)])
        || t.starts_with(&id[..id.len().min(8)])
}

pub(super) fn replace_artifact_tokens(mut html: String, arts: &[Artifact]) -> String {
    while let Some(start) = html.find("{{artifact:") {
        let (head, rest) = html.split_at(start);
        let rest = &rest["{{artifact:".len()..];
        let Some(end) = rest.find("}}") else {
            break;
        };
        let token = rest[..end].trim();
        let tail = &rest[end + 2..];
        let chip = arts
            .iter()
            .enumerate()
            .find_map(|(i, a)| {
                if artifact_matches_token(token, &a.id) {
                    Some(art_chip(i, a))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                let short = &token[..token.len().min(8)];
                format!(r#"<span class="art-ref dead" title="{token}">artifact-{short}</span>"#)
            });
        html = format!("{head}{chip}{tail}");
    }
    html
}

/// Promote bare `<code>filename</code>` to artifact chips, without nesting
/// inside an existing `.art-ref` (browsers auto-split nested `<button>`s into
/// an empty outer chip + a filled sibling — the dashed pills in lists).
pub(super) fn wrap_code_filenames_as_art_refs(html: String, arts: &[Artifact]) -> String {
    let mut html = html;
    for (i, a) in arts.iter().enumerate() {
        let fname = html_escape(&a.name);
        if fname.is_empty() {
            continue;
        }
        let needle = format!("<code>{fname}</code>");
        let replacement = format!(
            r#"<button type="button" class="art-ref" data-art-idx="{i}" title="{fname}"><code>{fname}</code></button>"#
        );
        html = replace_code_outside_art_refs(&html, &needle, &replacement);
    }
    html
}

fn replace_code_outside_art_refs(html: &str, needle: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(idx) = rest.find(needle) {
        let before = &rest[..idx];
        out.push_str(before);
        if code_is_inside_art_ref(before) {
            out.push_str(needle);
        } else {
            out.push_str(replacement);
        }
        rest = &rest[idx + needle.len()..];
    }
    out.push_str(rest);
    out
}

fn code_is_inside_art_ref(before: &str) -> bool {
    let open_btn = before.rfind(r#"class="art-ref""#);
    let close_btn = before.rfind("</button>");
    let close_span = before.rfind("</span>");
    let last_close = match (close_btn, close_span) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    match (open_btn, last_close) {
        (Some(o), Some(c)) => o > c,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Drop stray list markers left in front of artifact chips.
/// Models often write `- \`file\`` inside table cells; after chip promotion the
/// leading `- ` remains as a dash beside the pill.
pub(super) fn strip_list_markers_before_art_refs(html: &str) -> String {
    const CHIPS: &[&str] = &[
        r#"<button type="button" class="art-ref""#,
        r#"<span class="art-ref"#,
    ];
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some((idx, needle)) = CHIPS
        .iter()
        .filter_map(|n| rest.find(n).map(|i| (i, *n)))
        .min_by_key(|(i, _)| *i)
    {
        let (before, after) = rest.split_at(idx);
        out.push_str(strip_trailing_list_marker(before));
        out.push_str(needle);
        rest = &after[needle.len()..];
    }
    out.push_str(rest);
    out
}

fn strip_trailing_list_marker(before: &str) -> &str {
    let trimmed = before.trim_end_matches([' ', '\t']);
    let Some(without) = trimmed.strip_suffix(['-', '*', '•', '–', '—']) else {
        return before;
    };
    let boundary = without.trim_end_matches([' ', '\t']);
    if boundary.is_empty() || boundary.ends_with('>') || boundary.ends_with('\n') {
        boundary
    } else {
        before
    }
}

fn html_attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!(r#"{name}=""#);
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

fn resource_reference_matches(rendered: &str, original: &str) -> bool {
    fn decode_html_attribute(value: &str) -> String {
        value
            .replace("&#x27;", "'")
            .replace("&#39;", "'")
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
    }
    normalize_path(&decode_href(&decode_html_attribute(rendered)))
        == normalize_path(&decode_href(original))
}

/// Replace path-bearing Markdown tags with durable resource identities. Only
/// bindings persisted with this exact message are considered; old unbound
/// messages intentionally retain their original behavior.
fn replace_bound_resource_tags(html: String, resources: &[MessageResource]) -> String {
    if resources.is_empty() {
        return html;
    }
    let mut out = String::with_capacity(html.len() + resources.len() * 48);
    let mut rest = html.as_str();
    let mut consumed = vec![false; resources.len()];
    loop {
        let next = [(rest.find("<a "), "href"), (rest.find("<img "), "src")]
            .into_iter()
            .filter_map(|(position, attribute)| position.map(|position| (position, attribute)))
            .min_by_key(|(position, _)| *position);
        let Some((position, attribute)) = next else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..position]);
        rest = &rest[position..];
        let Some(end) = rest.find('>') else {
            out.push_str(rest);
            break;
        };
        let tag = &rest[..=end];
        rest = &rest[end + 1..];
        let Some(reference) = html_attr(tag, attribute) else {
            out.push_str(tag);
            continue;
        };
        let Some(resource_index) = resources.iter().enumerate().position(|(index, resource)| {
            !consumed[index] && resource_reference_matches(&reference, &resource.original_reference)
        }) else {
            out.push_str(tag);
            continue;
        };
        consumed[resource_index] = true;
        let resource = &resources[resource_index];
        let old = format!(r#"{attribute}="{}""#, reference);
        let title = html_escape(
            resource
                .error
                .as_deref()
                .unwrap_or(resource.display_name.as_str()),
        );
        if attribute == "src" && resource.status != "ready" {
            out.push_str(&format!(
                r#"<span class="resource-unresolved" data-resource-id="{}" data-resource-status="unresolved" title="{title}">{}</span>"#,
                html_escape(&resource.id),
                html_escape(&format!("{} — {title}", resource.display_name)),
            ));
            continue;
        }
        let replacement = if resource.status == "ready" && resource.artifact_version_id.is_some() {
            let value = if attribute == "src" { "" } else { "#" };
            format!(
                r#"{attribute}="{value}" data-resource-id="{}" data-resource-kind="{}" data-resource-status="ready" title="{title}""#,
                html_escape(&resource.id),
                html_escape(&resource.kind),
            )
        } else {
            let value = if attribute == "src" { "" } else { "#" };
            format!(
                r#"{attribute}="{value}" data-resource-id="{}" data-resource-kind="{}" data-resource-status="unresolved" title="{title}""#,
                html_escape(&resource.id),
                html_escape(&resource.kind),
            )
        };
        out.push_str(&tag.replacen(&old, &replacement, 1));
    }
    out
}

/// Post-process rendered Markdown: durable resources, artifact chips, code
/// wrappers, and filename links.
pub(super) fn enrich_md_html(
    mut html: String,
    arts: &[Artifact],
    resources: &[MessageResource],
    locale: Locale,
) -> String {
    html = replace_bound_resource_tags(html, resources);
    html = replace_artifact_tokens(html, arts);
    html = replace_file_links(html, arts);
    for (i, a) in arts.iter().enumerate() {
        let chip = art_chip(i, a);
        let marker = format!("{{{{artifact:{}}}}}", a.id);
        html = html.replace(&marker, &chip);
    }
    html = wrap_code_filenames_as_art_refs(html, arts);
    html = strip_list_markers_before_art_refs(&html);
    html = html.replace("<pre><code", "<pre class=\"md-code\"><code");
    html = wrap_markdown_tables_with_copy_controls(html, locale);
    html
}

#[cfg(test)]
mod art_ref_marker_tests {
    use super::*;

    fn message_resource(reference: &str, kind: &str, ready: bool) -> MessageResource {
        MessageResource {
            id: "resource-link".into(),
            ordinal: 0,
            original_reference: reference.into(),
            artifact_id: ready.then(|| "artifact-id".into()),
            artifact_version_id: ready.then(|| "version-id".into()),
            display_name: "plot.png".into(),
            kind: kind.into(),
            mime_type: "image/png".into(),
            status: if ready { "ready" } else { "unresolved" }.into(),
            error: (!ready).then(|| "not found".into()),
        }
    }

    #[test]
    fn replaces_bound_links_and_images_with_resource_identity() {
        let html = r#"<p><a href="D:/work/report.md">report</a><img src="figures/plot.png" alt="plot" /></p>"#;
        let resources = vec![
            message_resource("D:/work/report.md", "markdown", true),
            message_resource("figures/plot.png", "image", true),
        ];
        let out = replace_bound_resource_tags(html.into(), &resources);
        assert_eq!(out.matches(r#"data-resource-status="ready""#).count(), 2);
        assert!(out.contains(r##"href="#" data-resource-id="resource-link""##));
        assert!(out.contains(r#"src="" data-resource-id="resource-link""#));
        assert!(!out.contains("D:/work/report.md"));
    }

    #[test]
    fn unresolved_binding_is_visible_and_never_keeps_the_raw_path() {
        let html = r#"<p><a href="figures/missing.md">missing</a></p>"#;
        let out = replace_bound_resource_tags(
            html.into(),
            &[message_resource("figures/missing.md", "markdown", false)],
        );
        assert!(out.contains(r#"data-resource-status="unresolved""#));
        assert!(out.contains(r#"title="not found""#));
        assert!(!out.contains("figures/missing.md"));
    }

    #[test]
    fn unresolved_image_becomes_an_error_placeholder() {
        let html = r#"<p><img src="figures/missing.png" alt="plot" /></p>"#;
        let out = replace_bound_resource_tags(
            html.into(),
            &[message_resource("figures/missing.png", "image", false)],
        );
        assert!(out.contains(r#"class="resource-unresolved""#));
        assert!(out.contains("plot.png — not found"));
        assert!(!out.contains("<img"));
        assert!(!out.contains("figures/missing.png"));
    }

    #[test]
    fn matches_html_escaped_quotes_in_rendered_destinations() {
        let markdown = "[report](D:/work/report.md')";
        let out = enrich_md_html(
            md_to_html(markdown),
            &[],
            &[message_resource("D:/work/report.md'", "markdown", true)],
            Locale::En,
        );
        assert!(out.contains(r#"data-resource-status="ready""#));
        assert!(!out.contains("D:/work/report.md"));
    }

    #[test]
    fn strips_list_dashes_before_chips_in_table_cells() {
        let html = r#"<td> - <button type="button" class="art-ref" data-art-idx="0">a.fasta</button> - <button type="button" class="art-ref" data-art-idx="1">b.fasta</button></td>"#;
        let out = strip_list_markers_before_art_refs(html);
        assert_eq!(
            out,
            r#"<td><button type="button" class="art-ref" data-art-idx="0">a.fasta</button><button type="button" class="art-ref" data-art-idx="1">b.fasta</button></td>"#
        );
    }

    #[test]
    fn keeps_dashes_that_are_part_of_prose() {
        let html = r#"see range 1 - <button type="button" class="art-ref" data-art-idx="0">x.csv</button>"#;
        assert_eq!(strip_list_markers_before_art_refs(html), html);
    }

    #[test]
    fn does_not_nest_art_refs_for_duplicate_filenames() {
        let arts = vec![
            Artifact {
                id: "aaa".into(),
                name: "denovo_design_worklist.csv".into(),
                kind: "csv",
                data: PreviewData::File {
                    path: "a/denovo_design_worklist.csv".into(),
                    kind: "csv".into(),
                },
                source_item: 0,
                superseded: false,
            },
            Artifact {
                id: "bbb".into(),
                name: "denovo_design_worklist.csv".into(),
                kind: "csv",
                data: PreviewData::File {
                    path: "b/denovo_design_worklist.csv".into(),
                    kind: "csv".into(),
                },
                source_item: 0,
                superseded: false,
            },
        ];
        let html = r#"<ul><li><code>denovo_design_worklist.csv</code></li></ul>"#;
        let out = wrap_code_filenames_as_art_refs(html.into(), &arts);
        assert_eq!(out.matches(r#"class="art-ref""#).count(), 1);
        assert!(out.contains(r#"data-art-idx="0""#));
        assert!(!out.contains(r#"data-art-idx="1""#));
        assert!(!out.contains("</button></button>"));
    }

    #[test]
    fn skips_code_already_inside_art_ref_chip() {
        let html = r#"<button type="button" class="art-ref" data-art-idx="0" title="x.csv"><code>x.csv</code></button>"#;
        let out = replace_code_outside_art_refs(
            html,
            "<code>x.csv</code>",
            r#"<button type="button" class="art-ref" data-art-idx="1" title="x.csv"><code>x.csv</code></button>"#,
        );
        assert_eq!(out, html);
    }

    #[test]
    fn wraps_markdown_tables_with_copy_controls() {
        let html = "<p>Summary</p><table><thead><tr><th>a</th></tr></thead><tbody><tr><td>1</td></tr></tbody></table>";
        let out = wrap_markdown_tables_with_copy_controls(html.into(), Locale::En);
        assert!(out.contains("md-table-card"));
        assert!(out.contains("md-table-copy"));
        assert!(out.contains("Copy table"));
    }

    #[test]
    fn table_data_to_tsv_uses_tabs_and_newlines() {
        let t = TableData {
            headers: vec!["Gene".into(), "TPM".into()],
            rows: vec![
                vec!["A".into(), "2.62".into()],
                vec!["B".into(), "1.81".into()],
            ],
        };
        assert_eq!(table_data_to_tsv(&t), "Gene\tTPM\nA\t2.62\nB\t1.81");
    }
}

pub(super) fn handle_md_click(
    ev: &web_sys::MouseEvent,
    arts: &[Artifact],
    resources: &[MessageResource],
    on_artifact: &Callback<usize>,
    on_file: &Callback<ModalArtifact>,
) {
    use wasm_bindgen::JsCast;
    let mut el = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    while let Some(n) = el {
        if n.class_list().contains("md-table-copy") {
            if let Ok(Some(card)) = n.closest(".md-table-card") {
                if let Ok(Some(table)) = card.query_selector("table") {
                    if let Ok(table) = table.dyn_into::<web_sys::HtmlTableElement>() {
                        ev.prevent_default();
                        ev.stop_propagation();
                        copy_text(html_table_to_tsv(&table));
                    }
                }
            }
            return;
        }
        if n.class_list().contains("art-ref") {
            if let Ok(idx) = n
                .get_attribute("data-art-idx")
                .unwrap_or_default()
                .parse::<usize>()
            {
                ev.prevent_default();
                ev.stop_propagation();
                on_artifact.call(idx);
            }
            return;
        }
        if n.tag_name().eq_ignore_ascii_case("a") {
            if let Some(resource_id) = n.get_attribute("data-resource-id") {
                ev.prevent_default();
                ev.stop_propagation();
                if let Some(resource) = resources.iter().find(|resource| resource.id == resource_id)
                {
                    if let Some(version_id) = resource
                        .artifact_version_id
                        .as_ref()
                        .filter(|_| resource.status == "ready")
                    {
                        on_file.call((
                            format!("artifact-version:{version_id}"),
                            resource.display_name.clone(),
                            resource.kind.clone(),
                        ));
                    }
                }
                return;
            }
            if let Some(href) = n.get_attribute("href") {
                if opens_in_system_browser(&href) {
                    ev.prevent_default();
                    ev.stop_propagation();
                    open_external_url(href);
                    return;
                }
                if !is_external_href(&href) {
                    ev.prevent_default();
                    ev.stop_propagation();
                    let path = normalize_path(&decode_href(&href));
                    if let Some(idx) = artifact_index_for_href(arts, &path) {
                        on_artifact.call(idx);
                    } else {
                        let kind = file_kind(&path).unwrap_or("text").to_string();
                        let name = attachment_name(&path);
                        on_file.call((path, name, kind));
                    }
                    return;
                }
            }
        }
        el = n.parent_element();
    }
}

pub(super) fn refresh_sessions(
    sessions: RwSignal<Vec<SessionInfo>>,
    pending: RwSignal<HashMap<String, usize>>,
    running: RwSignal<HashSet<String>>,
    next_cursor: RwSignal<Option<SessionCursor>>,
) {
    next_cursor.set(None);
    spawn_local(async move {
        let args = to_value(&serde_json::json!({ "cursor": null })).unwrap();
        let v = invoke("list_sessions_page", args).await;
        if let Ok(page) = serde_wasm_bindgen::from_value::<SessionPage>(v) {
            let set = pending.with_untracked(|m| rebuilt_running_set(&page.running_ids, m));
            running.set(set);
            sessions.set(page.items);
            next_cursor.set(page.next_cursor);
        }
    });
}

pub(super) fn load_older_sessions(
    sessions: RwSignal<Vec<SessionInfo>>,
    pending: RwSignal<HashMap<String, usize>>,
    running: RwSignal<HashSet<String>>,
    next_cursor: RwSignal<Option<SessionCursor>>,
    loading: RwSignal<bool>,
) {
    let Some(cursor) = next_cursor.get_untracked() else {
        return;
    };
    loading.set(true);
    spawn_local(async move {
        let args = to_value(&serde_json::json!({ "cursor": cursor })).unwrap();
        let v = invoke("list_sessions_page", args).await;
        if let Ok(page) = serde_wasm_bindgen::from_value::<SessionPage>(v) {
            let set = pending.with_untracked(|m| rebuilt_running_set(&page.running_ids, m));
            running.set(set);
            sessions.update(|current| {
                let existing = current
                    .iter()
                    .map(|item| item.id.clone())
                    .collect::<HashSet<_>>();
                current.extend(
                    page.items
                        .into_iter()
                        .filter(|item| !existing.contains(&item.id)),
                );
            });
            next_cursor.set(page.next_cursor);
        }
        loading.set(false);
    });
}

/// Rebuild the local `running` set from the backend's session-page snapshot
/// so restarts, project switches and other windows' turns are reflected. Keeps
/// locally pending sends the backend may not have registered yet.
pub(super) fn rebuilt_running_set(
    running_ids: &[String],
    pending: &HashMap<String, usize>,
) -> HashSet<String> {
    let mut set: HashSet<String> = running_ids.iter().cloned().collect();
    set.extend(pending.keys().cloned());
    set
}

#[cfg(test)]
mod rebuilt_running_set_tests {
    use super::*;

    #[test]
    fn keeps_server_running_and_local_pending() {
        let running = vec!["a".to_string()];
        let pending = HashMap::from([("c".to_string(), 1)]);
        let set = rebuilt_running_set(&running, &pending);
        assert!(set.contains("a"), "server-running kept");
        assert!(!set.contains("b"), "stale local state dropped");
        assert!(set.contains("c"), "local pending send kept");
    }
}

pub(super) fn refresh_folders(folders: RwSignal<Vec<FolderInfo>>) {
    spawn_local(async move {
        let v = invoke("list_folders", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<FolderInfo>>(v) {
            folders.set(list);
        }
    });
}

pub(super) fn bucket_sessions_by_date(
    list: &[SessionInfo],
) -> (Vec<SessionInfo>, Vec<SessionInfo>) {
    let now_ms = js_sys::Date::now();
    let (mut today, mut earlier) = (Vec::new(), Vec::new());
    for s in list {
        let ts_ms = if s.ts > 1_000_000_000_000 {
            s.ts as f64
        } else {
            s.ts as f64 * 1000.0
        };
        if s.ts > 0 && ts_ms >= now_ms - 86_400_000.0 {
            today.push(s.clone());
        } else {
            earlier.push(s.clone());
        }
    }
    (today, earlier)
}

// --- Artifact detection (Markdown tables + fenced CSV) -----------------------

/// Segment assistant text into plain-text and rendered Markdown-table chunks.
pub(super) enum Seg {
    Text,
    Table(TableData),
}

pub(super) fn split_segments(text: &str) -> Vec<Seg> {
    let lines: Vec<&str> = text.lines().collect();
    let mut segs: Vec<Seg> = vec![];
    let mut buf: Vec<&str> = vec![];
    let mut i = 0;
    while i < lines.len() {
        if is_table_row(lines[i]) && i + 1 < lines.len() && is_separator(lines[i + 1]) {
            if !buf.is_empty() {
                segs.push(Seg::Text);
                buf.clear();
            }
            let headers = split_row(lines[i]);
            let mut rows = vec![];
            let mut j = i + 2;
            while j < lines.len() && is_table_row(lines[j]) {
                rows.push(split_row(lines[j]));
                j += 1;
            }
            segs.push(Seg::Table(TableData { headers, rows }));
            i = j;
        } else {
            buf.push(lines[i]);
            i += 1;
        }
    }
    if !buf.is_empty() {
        segs.push(Seg::Text);
    }
    segs
}

/// Locale-free artifact material extracted from one chat item. Ids, localized
/// names, per-kind numbering, and cross-item file dedup are assigned later in
/// `assemble_artifacts`, so this half is cacheable per item and streaming only
/// re-extracts the tail message instead of rescanning the whole transcript.
pub(super) enum ProtoArtifact {
    Table(TableData),
    Csv(TableData),
    Fasta(String),
    Latex(String),
    File { path: String, kind: &'static str },
}

pub(super) fn file_proto(word: &str) -> Option<ProtoArtifact> {
    let p = normalize_path(
        word.trim()
            .trim_matches('`')
            .trim_matches('"')
            .trim_matches('\''),
    );
    if p.is_empty() {
        return None;
    }
    let kind = file_kind(&p)?;
    // Source files preview fine once opened, but this scan runs over every word
    // of tool output — auto-promoting each .py/.R path it mentions (tracebacks,
    // import lists, pip logs) would bury the pane. Code belongs in Notebook.
    if kind == "code" {
        return None;
    }
    Some(ProtoArtifact::File { path: p, kind })
}

pub(super) struct ArtifactScan {
    tbl_n: usize,
    csv_n: usize,
    tex_n: usize,
}

pub(super) fn extract_markdown_protos(out: &mut Vec<ProtoArtifact>, s: &str) {
    for seg in split_segments(s) {
        if let Seg::Table(t) = seg {
            out.push(ProtoArtifact::Table(t));
        }
    }
    for (lang, body) in fenced_blocks(s) {
        if lang == "csv" || lang == "tsv" {
            let lines: Vec<&str> = body.lines().collect();
            if let Some(header) = lines.first() {
                let headers = parse_csv_line(header);
                let rows = lines[1..].iter().map(|line| parse_csv_line(line)).collect();
                out.push(ProtoArtifact::Csv(TableData { headers, rows }));
            }
        } else if lang == "fasta" || lang == "fa" {
            out.push(ProtoArtifact::Fasta(body));
        }
    }
    let lines: Vec<&str> = s.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim().starts_with("```") {
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                i += 1;
            }
            i = i.saturating_add(1);
            continue;
        }
        if lines[i].trim().starts_with("$") {
            let mut body = vec![];
            let mut j = i + 1;
            while j < lines.len() && !lines[j].trim().ends_with("$") {
                body.push(lines[j]);
                j += 1;
            }
            if j < lines.len() {
                body.push(lines[j].trim().trim_end_matches("$"));
            }
            out.push(ProtoArtifact::Latex(body.join("\n")));
            i = j + 1;
            continue;
        }
        i += 1;
    }
    for word in s.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == '[' || c == ']')
    {
        if let Some(p) = file_proto(word) {
            out.push(p);
        }
    }
}

/// Extraction half of the artifact scan: pure function of one item's content.
pub(super) fn extract_protos(it: &ChatItem) -> Vec<ProtoArtifact> {
    let mut out = vec![];
    match it {
        // Uploaded files live only in the user turn ("Uploaded files: a, b").
        ChatItem::User(s) => {
            for word in s.split(|c: char| c.is_whitespace() || c == ',' || c == '"' || c == '\'') {
                if let Some(p) = file_proto(word) {
                    out.push(p);
                }
            }
        }
        ChatItem::Assistant { text: s, .. } => extract_markdown_protos(&mut out, s),
        ChatItem::Tool {
            name,
            input,
            output,
            ..
        } => {
            if name == "attempt_completion" && !output.is_empty() {
                extract_markdown_protos(&mut out, output);
            } else {
                let text = if output.is_empty() {
                    input.as_str()
                } else {
                    output.as_str()
                };
                for word in
                    text.split(|c: char| c.is_whitespace() || c == '\n' || c == '"' || c == '\'')
                {
                    if let Some(p) = file_proto(word) {
                        out.push(p);
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// Numbering half: assign ids, localized names, and cross-item file dedup.
/// O(artifact count) per run — cheap next to re-scanning message text.
pub(super) fn assemble_artifacts(
    per_item: &[Rc<Vec<ProtoArtifact>>],
    locale: Locale,
) -> Vec<Artifact> {
    let mut out: Vec<Artifact> = vec![];
    let mut scan = ArtifactScan {
        tbl_n: 0,
        csv_n: 0,
        tex_n: 0,
    };
    for (source_item, protos) in per_item.iter().enumerate() {
        for p in protos.iter() {
            match p {
                ProtoArtifact::Table(t) => {
                    scan.tbl_n += 1;
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name: tf(locale, "artifact.table", &[("n", &scan.tbl_n.to_string())]),
                        kind: "table",
                        data: PreviewData::Table(t.clone()),
                        source_item,
                        superseded: false,
                    });
                }
                ProtoArtifact::Csv(t) => {
                    scan.csv_n += 1;
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name: format!("data-{}.csv", scan.csv_n),
                        kind: "csv",
                        data: PreviewData::Table(t.clone()),
                        source_item,
                        superseded: false,
                    });
                }
                ProtoArtifact::Fasta(body) => {
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name: format!("alignment-{}.fasta", scan.csv_n),
                        kind: "fasta",
                        data: PreviewData::Fasta(body.clone()),
                        source_item,
                        superseded: false,
                    });
                }
                ProtoArtifact::Latex(tex) => {
                    scan.tex_n += 1;
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name: tf(
                            locale,
                            "artifact.equation",
                            &[("n", &scan.tex_n.to_string())],
                        ),
                        kind: "latex",
                        data: PreviewData::Latex {
                            tex: tex.clone(),
                            display: true,
                        },
                        source_item,
                        superseded: false,
                    });
                }
                ProtoArtifact::File { path, kind } => {
                    if out.iter().any(|a| {
                        a.source_item == source_item
                            && matches!(&a.data, PreviewData::File { path: p, .. } if p == path)
                    }) {
                        continue;
                    }
                    for existing in out.iter_mut().filter(
                        |a| matches!(&a.data, PreviewData::File { path: p, .. } if p == path),
                    ) {
                        existing.superseded = true;
                    }
                    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name,
                        kind,
                        data: PreviewData::File {
                            path: path.clone(),
                            kind: kind.to_string(),
                        },
                        source_item,
                        superseded: false,
                    });
                }
            }
        }
    }
    out
}

/// Promote `attempt_completion` output into the assistant bubble (web-dist renders
/// completion as the final markdown response, not a collapsed tool row).
pub(super) fn promote_assistant_text(items: &mut Vec<ChatItem>, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    if let Some(i) = items
        .iter()
        .rposition(|i| matches!(i, ChatItem::Assistant { .. }))
    {
        if let ChatItem::Assistant { text: s, .. } = &mut items[i] {
            if s.is_empty() {
                s.push_str(text);
                return;
            }
        }
    }
    items.push(ChatItem::Assistant {
        text: text.to_string(),
        model: None,
        resources: Vec::new(),
    });
}

/// Identity hash of the artifact list as seen by assistant markdown (chip
/// index, id, and label). Mixed into assistant row keys so chips re-render
/// when the artifact list changes, and nothing re-renders when it doesn't.
pub(super) fn artifacts_fingerprint(arts: &[Artifact]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for a in arts {
        (&a.id, &a.name).hash(&mut h);
    }
    h.finish()
}

pub(super) type ProtoCache = HashMap<(usize, u64), Rc<Vec<ProtoArtifact>>>;

/// Collect tables, data blocks, equations, and file-path artifacts from the transcript.
/// Extraction is cached per (index, content hash): a streaming flush only
/// re-extracts the message it touched, then renumbering runs over the cached
/// protos — instead of rescanning every message on every signal write.
pub(super) fn collect_artifacts(
    items: &[ChatItem],
    locale: Locale,
    cache: &mut ProtoCache,
) -> Vec<Artifact> {
    let mut next: ProtoCache = HashMap::with_capacity(items.len());
    let mut per_item: Vec<Rc<Vec<ProtoArtifact>>> = Vec::with_capacity(items.len());
    for (i, it) in items.iter().enumerate() {
        let key = (i, it.fingerprint());
        let protos = cache
            .remove(&key)
            .unwrap_or_else(|| Rc::new(extract_protos(it)));
        next.insert(key, protos.clone());
        per_item.push(protos);
    }
    *cache = next;
    let mut artifacts = assemble_artifacts(&per_item, locale);
    for artifact in &mut artifacts {
        if matches!(items.get(artifact.source_item), Some(ChatItem::Tool { .. })) {
            if let Some(next_assistant) = items
                .iter()
                .enumerate()
                .skip(artifact.source_item + 1)
                .find_map(|(index, item)| {
                    matches!(item, ChatItem::Assistant { .. }).then_some(index)
                })
            {
                artifact.source_item = next_assistant;
            }
        }
    }
    artifacts
}

/// Stable file identity for the current workspace. Agent and tool messages may
/// refer to the same file using either an absolute path or a workspace-relative
/// path; the Artifacts panel should still treat those references as one file.
fn artifact_file_identity(path: &str, project_root: &str) -> String {
    fn slash_path(path: &str) -> String {
        let mut normalized = path.trim().replace('\\', "/");
        while normalized.contains("//") {
            normalized = normalized.replace("//", "/");
        }
        while normalized.starts_with("./") {
            normalized.drain(..2);
        }
        normalized.trim_end_matches('/').to_string()
    }

    let path = slash_path(path);
    let root = slash_path(project_root);
    let windows_workspace = root.as_bytes().get(1) == Some(&b':');
    let comparable_path = if windows_workspace {
        path.to_ascii_lowercase()
    } else {
        path.clone()
    };
    let comparable_root = if windows_workspace {
        root.to_ascii_lowercase()
    } else {
        root.clone()
    };

    let relative = comparable_path
        .strip_prefix(&(comparable_root + "/"))
        .unwrap_or(&comparable_path);
    relative.to_string()
}

/// Current artifact projection for the right-hand panel. The full scan is kept
/// for transcript cards and provenance, while this view shows only the latest
/// live reference for each physical workspace file.
pub(super) fn current_artifacts(
    artifacts: &[Artifact],
    project_root: &str,
    missing_paths: &HashSet<String>,
) -> Vec<Artifact> {
    let mut seen_files = HashSet::new();
    let mut current = Vec::with_capacity(artifacts.len());
    for artifact in artifacts.iter().rev() {
        if artifact.superseded {
            continue;
        }
        match &artifact.data {
            PreviewData::File { path, .. } => {
                if missing_paths.contains(path) {
                    continue;
                }
                let identity = artifact_file_identity(path, project_root);
                if !seen_files.insert(identity) {
                    continue;
                }
            }
            _ => {}
        }
        current.push(artifact.clone());
    }
    current.reverse();
    current
}

#[cfg(test)]
mod artifact_scan_tests {
    use super::*;

    fn fresh(items: &[ChatItem], locale: Locale) -> Vec<Artifact> {
        collect_artifacts(items, locale, &mut ProtoCache::new())
    }

    /// #307 made .R/.py previewable, which must not also turn every source path
    /// this scan walks past (tracebacks, import lists) into an artifact chip.
    #[test]
    fn source_paths_do_not_become_artifacts() {
        assert!(file_proto("scripts/deseq2.R").is_none());
        assert!(file_proto("/mock/root/train.py").is_none());
        assert!(file_proto("pixi.toml").is_none());
        // Data and documents still do.
        assert!(matches!(
            file_proto("out/report.csv"),
            Some(ProtoArtifact::File { kind: "csv", .. })
        ));
        assert!(matches!(
            file_proto("notes.md"),
            Some(ProtoArtifact::File {
                kind: "markdown",
                ..
            })
        ));
    }

    /// Streaming reuses cached extractions for untouched messages; the result
    /// must be identical to a from-scratch scan (ids, names, order, dedup).
    #[test]
    fn cached_scan_matches_fresh_scan() {
        let mut cache = ProtoCache::new();
        let mut items = vec![
            ChatItem::User("check data.csv and data.csv".into()),
            ChatItem::Assistant {
                text: "| a | b |\n|---|---|\n| 1 | 2 |\n\n```csv\nx,y\n1,2\n```".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        let a1 = collect_artifacts(&items, Locale::En, &mut cache);
        assert!(a1 == fresh(&items, Locale::En));
        // csv file (deduped once) + table + fenced csv
        assert_eq!(a1.len(), 3);

        // Simulate a streaming flush: the tail message grows a code fence.
        if let ChatItem::Assistant { text, .. } = &mut items[1] {
            text.push_str("\n```py\nprint(1)\n```");
        }
        items.push(ChatItem::Tool {
            name: "write".into(),
            ok: Some(true),
            input: String::new(),
            output: "wrote out/result.csv".into(),
            started_at_ms: None,
            duration_ms: Some(1),
        });
        let a2 = collect_artifacts(&items, Locale::En, &mut cache);
        assert!(a2 == fresh(&items, Locale::En));
        assert_eq!(a2.len(), 4); // code moves to Notebook; result.csv remains an artifact
    }

    #[test]
    fn overwritten_file_belongs_to_its_latest_message() {
        let items = vec![
            ChatItem::Assistant {
                text: "Created `result.csv`".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::Assistant {
                text: "Updated `result.csv`".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        let artifacts = fresh(&items, Locale::En);
        assert_eq!(artifacts.len(), 2);
        assert!(artifacts[0].superseded);
        assert_eq!(artifacts[0].source_item, 0);
        assert!(!artifacts[1].superseded);
        assert_eq!(artifacts[1].source_item, 1);
    }

    #[test]
    fn tool_output_belongs_to_the_following_reply() {
        let items = vec![
            ChatItem::Tool {
                name: "write".into(),
                ok: Some(true),
                input: String::new(),
                output: "wrote result.csv".into(),
                started_at_ms: None,
                duration_ms: None,
            },
            ChatItem::Assistant {
                text: "Done.".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        let artifacts = fresh(&items, Locale::En);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].source_item, 1);
    }

    #[test]
    fn artifact_panel_deduplicates_message_versions_and_path_forms() {
        let items = vec![
            ChatItem::Tool {
                name: "write".into(),
                ok: Some(true),
                input: String::new(),
                output: "wrote sample_statistics.png".into(),
                started_at_ms: None,
                duration_ms: None,
            },
            ChatItem::Assistant {
                text: "Created `sample_statistics.png`".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::Tool {
                name: "write".into(),
                ok: Some(true),
                input: String::new(),
                output: r"wrote E:\cross-species-root\sample_statistics.png".into(),
                started_at_ms: None,
                duration_ms: None,
            },
            ChatItem::Assistant {
                text: r"Updated `E:\cross-species-root\sample_statistics.png`".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        let all = fresh(&items, Locale::En);
        assert_eq!(all.len(), 4);

        let current = current_artifacts(&all, r"E:\cross-species-root", &HashSet::<String>::new());
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].name, "sample_statistics.png");
        assert!(matches!(
            &current[0].data,
            PreviewData::File { path, .. }
                if path == r"E:\cross-species-root\sample_statistics.png"
        ));
    }
}

pub(super) fn table_view(table: &TableData, locale: Locale) -> impl IntoView {
    let total = table.rows.len();
    let truncated = total > 500;
    let copy = table_data_to_tsv(table);
    let headers: Vec<String> = table.headers.iter().map(|h| md_inline_to_html(h)).collect();
    let rows: Vec<Vec<String>> = table
        .rows
        .iter()
        .take(500)
        .map(|r| r.iter().map(|c| md_inline_to_html(c)).collect())
        .collect();
    view! {
        <div class="tbl-card">
            <div class="tbl-head">
                {truncated.then(|| view! {
                    <span class="tbl-note">{tf(locale, "table.rows_note", &[("total", &total.to_string())])}</span>
                })}
                <button type="button" class="tbl-copy" on:click=move |_| copy_text(copy.clone())>
                    {move || crate::i18n::t(locale, "table.copy")}
                </button>
            </div>
            <div class="tbl-wrap">
                <table class="tbl">
                    <thead><tr>{headers.into_iter().map(|h| view! { <th inner_html=h></th> }).collect_view()}</tr></thead>
                    <tbody>
                        {rows.into_iter().map(|r| view! {
                            <tr>{r.into_iter().map(|c| view! { <td inner_html=c></td> }).collect_view()}</tr>
                        }).collect_view()}
                    </tbody>
                </table>
            </div>
        </div>
    }
}

pub(super) fn artifact_group_label(key: &str, locale: Locale) -> String {
    if let Some(kind) = key.strip_prefix('@') {
        let i18n = match kind {
            "table" => "artifact.group.table",
            "latex" => "artifact.group.latex",
            "csv" => "artifact.group.csv",
            "fasta" => "artifact.group.fasta",
            "msa" => "artifact.group.msa",
            "text" | "markdown" | "code" | "notebook" => "artifact.group.text",
            _ => return kind.to_string(),
        };
        t(locale, i18n).into()
    } else if key == "." {
        t(locale, "artifact.group.root").into()
    } else {
        key.to_string()
    }
}

pub(super) fn artifact_meta(a: &Artifact, locale: Locale) -> String {
    match &a.data {
        PreviewData::Table(t) => tf(
            locale,
            "artifact.meta.table",
            &[
                ("rows", &t.rows.len().to_string()),
                ("cols", &t.headers.len().to_string()),
            ],
        ),
        PreviewData::File { path, kind } => {
            if kind == "fasta" {
                t(locale, "artifact.kind.fasta").into()
            } else if kind == "msa" {
                t(locale, "artifact.kind.msa").into()
            } else if let Some(parent) = path.rsplit(['/', '\\']).nth(1) {
                if parent.is_empty() {
                    tf(locale, "artifact.meta.file", &[("kind", kind)])
                } else {
                    format!("{parent}/")
                }
            } else {
                tf(locale, "artifact.meta.file", &[("kind", kind)])
            }
        }
        PreviewData::Latex { .. } => t(locale, "artifact.latex").into(),
        PreviewData::Fasta(s) => tf(
            locale,
            "artifact.meta.fasta",
            &[("seqs", &fasta_seq_count(s).max(1).to_string())],
        ),
        PreviewData::Smiles(s) => s.chars().take(28).collect(),
        PreviewData::Text(s) | PreviewData::Markdown(s) => tf(
            locale,
            "artifact.meta.text",
            &[("chars", &s.len().to_string())],
        ),
    }
}

#[component]
pub(super) fn HeavyPreview(dom_id: String, kind: String, payload: String) -> impl IntoView {
    let id_for_effect = dom_id.clone();
    let kind_for_effect = kind.clone();
    let payload_for_effect = payload.clone();
    create_effect(move |_| {
        let dom_id = id_for_effect.clone();
        let kind = kind_for_effect.clone();
        let payload = payload_for_effect.clone();
        spawn_local(async move {
            let _ = mount_preview(&kind, &dom_id, &payload).await;
        });
    });
    view! { <div class="rp-heavy" id=dom_id></div> }
}

pub(super) fn parse_csv_text(text: &str) -> Option<TableData> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return None;
    }
    let headers = parse_csv_line(lines[0]);
    let rows: Vec<Vec<String>> = lines[1..].iter().map(|l| parse_csv_line(l)).collect();
    Some(TableData { headers, rows })
}

pub(super) fn artifact_id_path(path: &str) -> Option<&str> {
    path.strip_prefix("artifact:").filter(|id| !id.is_empty())
}

pub(super) fn artifact_version_id_path(path: &str) -> Option<&str> {
    path.strip_prefix("artifact-version:")
        .filter(|id| !id.is_empty())
}

/// The remote-preview path spelling: `remote:ssh:<alias>:<path>`. Returns the
/// execution-context id (`ssh:<alias>`) and the path on that host. SSH aliases
/// never contain `:` (see `remote_file_download_uri`), so the split after the
/// alias is unambiguous even though remote paths may contain colons.
pub(super) fn remote_file_path(path: &str) -> Option<(&str, &str)> {
    let ctx_and_path = path.strip_prefix("remote:")?;
    let after_kind = ctx_and_path.strip_prefix("ssh:")?;
    let alias_end = after_kind.find(':')?;
    let remote_path = &after_kind[alias_end + 1..];
    (alias_end > 0 && !remote_path.is_empty())
        .then(|| (&ctx_and_path[.."ssh:".len() + alias_end], remote_path))
}

#[cfg(test)]
mod remote_file_path_tests {
    use super::remote_file_path;

    #[test]
    fn splits_context_id_from_remote_path() {
        assert_eq!(
            remote_file_path("remote:ssh:gpu-server:/home/research/report.html"),
            Some(("ssh:gpu-server", "/home/research/report.html"))
        );
        assert_eq!(
            remote_file_path("remote:ssh:gpu:~/analysis.ipynb"),
            Some(("ssh:gpu", "~/analysis.ipynb"))
        );
        // Colons inside the remote path stay with the path.
        assert_eq!(
            remote_file_path("remote:ssh:gpu:/data/a:b.py"),
            Some(("ssh:gpu", "/data/a:b.py"))
        );
    }

    #[test]
    fn rejects_other_spellings() {
        assert_eq!(remote_file_path("reviews/notes.md"), None);
        assert_eq!(remote_file_path("artifact:abc"), None);
        assert_eq!(remote_file_path("remote:ssh:gpu:"), None);
        assert_eq!(remote_file_path("remote:ssh::/x"), None);
        assert_eq!(remote_file_path("remote:local:/x"), None);
    }
}

#[component]
pub(super) fn CsvFilePreview(path: String) -> impl IntoView {
    let locale = use_locale();
    let table = create_rw_signal::<Option<TableData>>(None);
    let err = create_rw_signal::<Option<String>>(None);
    create_effect(move |_| {
        let path = path.clone();
        let loc = locale.get();
        spawn_local(async move {
            table.set(None);
            err.set(None);
            let fc = match load_file_content(&path, loc).await {
                Ok(fc) => fc,
                Err(e) => {
                    err.set(Some(e));
                    return;
                }
            };
            match fc.text.as_deref().and_then(parse_csv_text) {
                Some(t) => table.set(Some(t)),
                None => err.set(Some(tf(loc, "err.file_not_found", &[("path", &path)]))),
            }
        });
    });
    move || match (table.get(), err.get()) {
        (Some(t), _) => table_view(&t, locale.get()).into_view(),
        (_, Some(e)) => view! { <div class="rp-error">{e}</div> }.into_view(),
        _ => view! { <div class="rp-heavy">{move || t(locale.get(), "loading")}</div> }.into_view(),
    }
}

/// Read a workspace file, a remote (SSH) file, an artifact, or a pinned
/// artifact version — the four path spellings a preview can be handed — into
/// its `FileContent`. All previews route through here so every kind (html,
/// notebook, code, image, …) works for every spelling.
async fn load_file_content(path: &str, loc: Locale) -> Result<FileContent, String> {
    let result = if let Some((context_id, remote_path)) = remote_file_path(path) {
        invoke_checked(
            "read_remote_file",
            to_value(&serde_json::json!({ "contextId": context_id, "path": remote_path })).unwrap(),
        )
        .await
    } else if let Some(version_id) = artifact_version_id_path(path) {
        invoke_checked(
            "read_artifact_version",
            to_value(&serde_json::json!({ "versionId": version_id })).unwrap(),
        )
        .await
    } else if let Some(id) = artifact_id_path(path) {
        invoke_checked(
            "read_artifact",
            to_value(&serde_json::json!({ "id": id })).unwrap(),
        )
        .await
    } else {
        invoke_checked(
            "read_file",
            to_value(&tauri_args::read_file(path, Some(32 * 1024 * 1024))).unwrap(),
        )
        .await
    };
    match result {
        Ok(v) => serde_wasm_bindgen::from_value::<FileContent>(v)
            .map_err(|_| tf(loc, "err.file_not_found", &[("path", path)])),
        Err(err_value) => Err(localize_backend(loc, &js_error_text(err_value))),
    }
}

/// Text/source preview: line-numbered and syntax-highlighted via `RpCodeView`.
/// The old plain-text mount dropped the file's newlines (`textContent` on a
/// non-`pre` div), which is what made R/shell scripts render as one paragraph.
#[component]
pub(super) fn CodeFilePreview(path: String, lang: String) -> impl IntoView {
    let locale = use_locale();
    let body = create_rw_signal::<Option<String>>(None);
    let err = create_rw_signal::<Option<String>>(None);
    let is_json = lang == "json";
    create_effect(move |_| {
        let path = path.clone();
        let loc = locale.get();
        spawn_local(async move {
            body.set(None);
            err.set(None);
            let fc = match load_file_content(&path, loc).await {
                Ok(fc) => fc,
                Err(e) => {
                    err.set(Some(e));
                    return;
                }
            };
            // No text means the backend judged it binary; an empty code view
            // would just look like an empty file.
            let Some(text) = fc.text.as_deref() else {
                err.set(Some(t(loc, "preview.unsupported_file")));
                return;
            };
            body.set(Some(if is_json {
                pretty_json(text)
            } else {
                text.to_string()
            }));
        });
    });
    move || match (body.get(), err.get()) {
        (Some(text), _) => view! { <RpCodeView lang=lang.clone() body=text /> }.into_view(),
        (_, Some(e)) => view! { <div class="rp-error">{e}</div> }.into_view(),
        _ => view! { <div class="rp-heavy">{move || t(locale.get(), "loading")}</div> }.into_view(),
    }
}

/// `.ipynb` preview: Markdown cells rendered, code cells highlighted in the
/// kernel's language, and saved static outputs under each cell. Reuses the chat
/// Notebook pane's styling so both read the same.
fn notebook_output_view(out: &NbOutput, dom_id: String, locale: Locale) -> View {
    match out {
        NbOutput::Text { text, error } => view! {
            <pre class=if *error { "nb-out-error" } else { "" }>{text.clone()}</pre>
        }
        .into_view(),
        NbOutput::Image { mime, b64 } => view! {
            <img
                class="rp-img"
                src=format!("data:{mime};base64,{b64}")
                alt=""
                loading="lazy"
                decoding="async"
            />
        }
        .into_view(),
        NbOutput::Html(html) => {
            let payload = serde_json::json!({ "text": html }).to_string();
            view! {
                <HeavyPreview dom_id=dom_id kind="notebook-html".to_string() payload=payload />
            }
            .into_view()
        }
        NbOutput::Svg(svg) => {
            let payload = serde_json::json!({
                "text": svg,
                "error": t(locale, "preview.unsupported_file"),
            })
            .to_string();
            view! {
                <HeavyPreview dom_id=dom_id kind="notebook-svg".to_string() payload=payload />
            }
            .into_view()
        }
        NbOutput::Latex(tex) => {
            let payload = serde_json::json!({ "tex": tex, "display": true }).to_string();
            view! {
                <div class="nb-out-latex">
                    <HeavyPreview dom_id=dom_id kind="latex".to_string() payload=payload />
                </div>
            }
            .into_view()
        }
        NbOutput::Omitted { mime, bytes } => {
            let size = format_bytes(*bytes as u64);
            let message = tf(
                locale,
                "preview.output_omitted",
                &[("kind", mime), ("size", &size)],
            );
            view! { <div class="nb-out-omitted">{message}</div> }.into_view()
        }
    }
}

#[component]
pub(super) fn NotebookFilePreview(path: String) -> impl IntoView {
    let locale = use_locale();
    let nb = create_rw_signal::<Option<Notebook>>(None);
    let err = create_rw_signal::<Option<String>>(None);
    let hid = unique_dom_id("nb");
    create_effect(move |_| {
        let path = path.clone();
        let loc = locale.get();
        spawn_local(async move {
            nb.set(None);
            err.set(None);
            match load_file_content(&path, loc).await {
                // A .ipynb that doesn't parse is corrupt or not really a notebook;
                // say so rather than drawing an empty cell list.
                Ok(fc) => match parse_notebook(fc.text.as_deref().unwrap_or("")) {
                    Some(parsed) => nb.set(Some(parsed)),
                    None => err.set(Some(t(loc, "preview.unsupported_file"))),
                },
                Err(e) => err.set(Some(e)),
            }
        });
    });
    let hid_effect = hid.clone();
    // One pass over the whole list: highlights the fenced code and math inside
    // rendered Markdown cells. Code cells highlight themselves via RpCodeView.
    create_effect(move |_| {
        let _ = nb.get();
        schedule_highlight(hid_effect.clone());
    });
    move || match (nb.get(), err.get()) {
        (Some(parsed), _) => {
            let lang = parsed.lang.clone();
            view! {
                <div class="notebook-cells" id=hid.clone()>
                    {parsed.cells.iter().enumerate().map(|(i, cell)| {
                        let outputs = cell.outputs.iter().enumerate().map(|(output_i, out)| {
                            notebook_output_view(
                                out,
                                format!("{hid}-output-{i}-{output_i}"),
                                locale.get_untracked(),
                            )
                        }).collect_view();
                        let body = if cell.markdown {
                            view! { <div class="md" inner_html=md_to_html(&cell.source)></div> }.into_view()
                        } else {
                            view! {
                                <div class="notebook-source">
                                    <RpCodeView lang=lang.clone() body=cell.source.clone() />
                                </div>
                            }.into_view()
                        };
                        view! {
                            <div class="notebook-cell">
                                <div class="notebook-cell-head">
                                    <span class="notebook-index">{i + 1}</span>
                                    <span class="notebook-language">
                                        {if cell.markdown { "markdown".to_string() } else { lang.clone() }}
                                    </span>
                                </div>
                                {body}
                                {(!cell.outputs.is_empty()).then(|| view! {
                                    <div class="notebook-output">{outputs}</div>
                                })}
                            </div>
                        }
                    }).collect_view()}
                </div>
            }
            .into_view()
        }
        (_, Some(e)) => view! { <div class="rp-error">{e}</div> }.into_view(),
        _ => view! { <div class="rp-heavy">{move || t(locale.get(), "loading")}</div> }.into_view(),
    }
}

#[component]
pub(super) fn WorkspaceFilePreview(dom_id: String, path: String, kind: String) -> impl IntoView {
    match kind.as_str() {
        "csv" => view! { <CsvFilePreview path=path /> }.into_view(),
        // Artifact/version tabs aren't real paths, so the extension can't be read
        // back off them — the kind is the only language signal here.
        "json" => view! { <CodeFilePreview path=path lang="json".to_string() /> }.into_view(),
        "code" | "text" => {
            let lang = code_lang(&path).unwrap_or("plaintext").to_string();
            view! { <CodeFilePreview path=path lang=lang /> }.into_view()
        }
        "notebook" => view! { <NotebookFilePreview path=path /> }.into_view(),
        // Image + PDF share the zoom viewport: the wheel zooms, and PDF pages are
        // stepped with the toolbar buttons / arrow keys / Page Up-Down.
        "image" | "pdf" => {
            view! { <ZoomableFilePreview dom_id=dom_id path=path kind=kind /> }.into_view()
        }
        _ => view! { <FilePreview dom_id=dom_id path=path kind=kind /> }.into_view(),
    }
}

#[component]
fn ZoomableFilePreview(dom_id: String, path: String, kind: String) -> impl IntoView {
    let locale = use_locale();
    let zoom = create_rw_signal(100u16);
    let is_dragging = create_rw_signal(false);
    let drag_start = Rc::new(Cell::new(None::<(i32, i32, i32, i32)>));
    let viewport_id = unique_dom_id("preview-viewport");
    // Region-crop (images only): drag a rectangle, then choose how to attach it.
    let is_image = kind == "image";
    let crop_mode = create_rw_signal(false);
    let crop_busy = create_rw_signal(false);
    let crop_path = create_rw_signal(None::<String>);
    // Live rubber-band rect in client (viewport) pixels: (left, top, right, bottom).
    let crop_rect = create_rw_signal(None::<(f64, f64, f64, f64)>);
    let crop_host_id = dom_id.clone();
    let finish_crop = Callback::new(move |()| {
        if crop_busy.get_untracked() || crop_path.get_untracked().is_some() {
            return;
        }
        let Some((l, t, r, b)) = crop_rect.get_untracked() else {
            return;
        };
        let (left, top) = (l.min(r), t.min(b));
        let (w, h) = ((l - r).abs(), (t - b).abs());
        // Ignore stray clicks; require a real region.
        if w < 8.0 || h < 8.0 {
            crop_rect.set(None);
            return;
        }
        let host_id = crop_host_id.clone();
        crop_busy.set(true);
        spawn_local(async move {
            let path = crop_region_to_upload(&host_id, left, top, w, h)
                .await
                .as_string()
                .unwrap_or_default();
            crop_busy.set(false);
            if path.is_empty() {
                crop_rect.set(None);
            } else {
                crop_path.set(Some(path));
            }
        });
    });
    let adjust_zoom = move |delta: i16| {
        zoom.update(|value| {
            *value = ((*value as i16) + delta).clamp(25, 400) as u16;
        });
    };
    let viewport_for_event = Rc::new({
        let viewport_id = viewport_id.clone();
        move || {
            web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id(&viewport_id))
                .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
        }
    });
    let stop_drag = {
        let viewport_for_event = viewport_for_event.clone();
        let drag_start = drag_start.clone();
        move |pointer_id: i32| {
            if let Some(viewport) = viewport_for_event() {
                let _ = viewport.release_pointer_capture(pointer_id);
            }
            drag_start.set(None);
            is_dragging.set(false);
        }
    };
    let viewport_for_pointerdown = viewport_for_event.clone();
    let viewport_for_pointermove = viewport_for_event.clone();
    let stop_drag_up = stop_drag.clone();
    let stop_drag_cancel = stop_drag.clone();
    let drag_start_down = drag_start.clone();
    let drag_start_move = drag_start.clone();
    let drag_start_lost = drag_start.clone();
    view! {
        <div class="file-preview-zoom">
            <div class="file-preview-zoom-bar">
                <button type="button" aria-label=move || t(locale.get(), "preview.zoom_out")
                    disabled=move || { zoom.get() <= 25 }
                    on:click=move |_| adjust_zoom(-25)>"−"</button>
                <button type="button" aria-label=move || t(locale.get(), "preview.zoom_reset")
                    on:click=move |_| zoom.set(100)>{move || format!("{}%", zoom.get())}</button>
                <button type="button" aria-label=move || t(locale.get(), "preview.zoom_in")
                    disabled=move || { zoom.get() >= 400 }
                    on:click=move |_| adjust_zoom(25)>"+"</button>
                {is_image.then(|| view! {
                    <button type="button" class="file-preview-crop-btn"
                        class:active=move || crop_mode.get()
                        disabled=move || crop_busy.get()
                        aria-pressed=move || crop_mode.get().to_string()
                        title=move || t(locale.get(), "preview.select_region")
                        aria-label=move || t(locale.get(), "preview.select_region")
                        on:click=move |_| {
                            crop_rect.set(None);
                            crop_path.set(None);
                            crop_mode.update(|m| *m = !*m);
                        }>
                        {compose_icon("crop")}
                    </button>
                })}
            </div>
            <div id=viewport_id class="file-preview-zoom-viewport"
                class:is-dragging=move || { is_dragging.get() }
                class:is-cropping=move || { crop_mode.get() }
                on:pointerdown=move |ev: web_sys::PointerEvent| {
                    if ev.button() != 0 || crop_mode.get_untracked() {
                        return;
                    }
                    // PDF glyphs remain drag-selectable for quoting. Starting
                    // on the surrounding page/whitespace pans the preview.
                    if ev
                        .target()
                        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                        .and_then(|target| target.closest(".rp-pdf-textlayer span").ok().flatten())
                        .is_some()
                    {
                        return;
                    }
                    let Some(viewport) = viewport_for_pointerdown() else {
                        return;
                    };
                    // Zoom percentage is not a reliable proxy for pannability:
                    // a tall image or PDF page can overflow the modal at 100%.
                    // Only capture the drag when there is actual scrollable
                    // content in at least one direction.
                    if viewport.scroll_width() <= viewport.client_width()
                        && viewport.scroll_height() <= viewport.client_height()
                    {
                        return;
                    }
                    ev.prevent_default();
                    let _ = viewport.set_pointer_capture(ev.pointer_id());
                    drag_start_down.set(Some((
                        ev.client_x(),
                        ev.client_y(),
                        viewport.scroll_left(),
                        viewport.scroll_top(),
                    )));
                    is_dragging.set(true);
                }
                on:pointermove=move |ev: web_sys::PointerEvent| {
                    let Some((start_x, start_y, scroll_left, scroll_top)) = drag_start_move.get() else {
                        return;
                    };
                    let Some(viewport) = viewport_for_pointermove() else {
                        return;
                    };
                    ev.prevent_default();
                    viewport.set_scroll_left(scroll_left - (ev.client_x() - start_x));
                    viewport.set_scroll_top(scroll_top - (ev.client_y() - start_y));
                }
                on:pointerup=move |ev: web_sys::PointerEvent| stop_drag_up(ev.pointer_id())
                on:pointercancel=move |ev: web_sys::PointerEvent| stop_drag_cancel(ev.pointer_id())
                on:lostpointercapture=move |_| {
                    drag_start_lost.set(None);
                    is_dragging.set(false);
                }
                on:wheel=move |ev: web_sys::WheelEvent| {
                    ev.prevent_default();
                    if ev.delta_y() < 0.0 {
                        adjust_zoom(25);
                    } else if ev.delta_y() > 0.0 {
                        adjust_zoom(-25);
                    }
                }>
                <div class="file-preview-zoom-content" data-zoom-kind=kind.clone()
                    style=move || format!("--preview-zoom:{}", zoom.get() as f32 / 100.0)>
                    <FilePreview dom_id=dom_id path=path kind=kind />
                </div>
                // Region-crop overlay: captures the drag so it never pans, draws
                // the rubber-band, and on release crops+uploads the region.
                {move || crop_mode.get().then(|| view! {
                    <div class="file-preview-crop-layer"
                        on:pointerdown=move |ev: web_sys::PointerEvent| {
                            if ev.button() != 0
                                || crop_busy.get_untracked()
                                || crop_path.get_untracked().is_some()
                            {
                                return;
                            }
                            ev.prevent_default();
                            if let Some(target) = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                                let _ = target.set_pointer_capture(ev.pointer_id());
                            }
                            let (x, y) = (ev.client_x() as f64, ev.client_y() as f64);
                            crop_rect.set(Some((x, y, x, y)));
                        }
                        on:pointermove=move |ev: web_sys::PointerEvent| {
                            if crop_busy.get_untracked() || crop_path.get_untracked().is_some() {
                                return;
                            }
                            crop_rect.update(|r| {
                                if let Some((l, t, _, _)) = *r {
                                    *r = Some((l, t, ev.client_x() as f64, ev.client_y() as f64));
                                }
                            });
                        }
                        on:pointerup=move |_| finish_crop.call(())
                        on:pointercancel=move |_| crop_rect.set(None)>
                        {move || crop_rect.get().map(|(left, top, right, bottom)| {
                            let selected = crop_path.get().is_some();
                            let style = format!(
                                "left:{}px;top:{}px;width:{}px;height:{}px",
                                left.min(right),
                                top.min(bottom),
                                (left - right).abs(),
                                (top - bottom).abs(),
                            );
                            view! {
                                <div class="file-preview-crop-rect" class:selected=selected style=style>
                                    {selected.then(|| view! {
                                        <span class="file-preview-crop-label">
                                            {move || t(locale.get(), "preview.region_selected")}
                                        </span>
                                    })}
                                </div>
                            }
                        })}
                        {move || crop_path.get().and_then(|path| crop_rect.get().map(|(left, top, right, bottom)| {
                            let add_path = path.clone();
                            let jump_path = path;
                            let x = (left + right) / 2.0;
                            let y = top.min(bottom);
                            let style = format!(
                                "left:clamp(190px,{x}px,calc(100vw - 190px));top:max(52px,{y}px)",
                            );
                            view! {
                                <div class="selection-popup file-preview-crop-actions" style=style
                                    on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                                    on:pointerup=|ev: web_sys::PointerEvent| ev.stop_propagation()>
                                    <button type="button" class="selection-popup-btn"
                                        on:click=move |_| {
                                            crop_path.set(None);
                                            crop_rect.set(None);
                                            crop_mode.set(false);
                                            attach_cropped_region(&add_path, false);
                                        }>
                                        {compose_icon("plus")}
                                        <span>{move || t(locale.get(), "selection.add_to_chat")}</span>
                                    </button>
                                    <button type="button" class="selection-popup-btn"
                                        on:click=move |_| {
                                            crop_path.set(None);
                                            crop_rect.set(None);
                                            crop_mode.set(false);
                                            attach_cropped_region(&jump_path, true);
                                        }>
                                        {compose_icon("chat")}
                                        <span>{move || t(locale.get(), "selection.add_to_chat_and_jump")}</span>
                                    </button>
                                </div>
                            }
                        }))}
                    </div>
                })}
            </div>
        </div>
    }
}

#[component]
pub(super) fn FilePreview(dom_id: String, path: String, kind: String) -> impl IntoView {
    let locale = use_locale();
    let id_for_effect = dom_id.clone();
    let path_for_effect = path.clone();
    create_effect(move |_| {
        let path = path_for_effect.clone();
        let kind = kind.clone();
        let dom_id = id_for_effect.clone();
        let loc = locale.get();
        spawn_local(async move {
            let doc = web_sys::window().and_then(|w| w.document());
            let el = doc.as_ref().and_then(|d| d.get_element_by_id(&dom_id));
            // `load_file_content` reads up to the backend's 32 MB ceiling so a large
            // produced figure or PDF still renders (the default 8 MB cap silently
            // rejected them, #35), and surfaces the real backend error (size limit /
            // outside project root / …) instead of a blanket "file not found".
            let fc = match load_file_content(&path, loc).await {
                Ok(fc) => fc,
                Err(message) => {
                    if let Some(el) = el {
                        el.set_class_name("rp-heavy rp-error");
                        el.set_text_content(Some(&message));
                    }
                    return;
                }
            };
            if !matches!(kind.as_str(), "image" | "pdf" | "docx") && fc.text.is_none() {
                if let Some(el) = el {
                    el.set_class_name("rp-heavy rp-error");
                    el.set_text_content(Some(&t(loc, "preview.unsupported_file")));
                }
                return;
            }
            if kind == "markdown" {
                if let Some(el) = el {
                    el.set_class_name("rp-heavy md");
                    el.set_inner_html(&md_to_html(fc.text.as_deref().unwrap_or("")));
                    schedule_highlight(dom_id.clone());
                }
                return;
            }
            let (mount_kind, payload) = match kind.as_str() {
                "pdf" => (
                    "pdf",
                    serde_json::json!({
                        "b64": fc.base64,
                        "loading": t(loc, "loading"),
                        "error": t(loc, "preview.pdf_error"),
                        "pageLabel": t(loc, "preview.pdf_page"),
                        "prevPage": t(loc, "preview.pdf_prev_page"),
                        "nextPage": t(loc, "preview.pdf_next_page"),
                    })
                    .to_string(),
                ),
                "image" => (
                    "image",
                    serde_json::json!({ "b64": fc.base64, "mime": fc.mime }).to_string(),
                ),
                "docx" => (
                    "docx",
                    serde_json::json!({
                        "b64": fc.base64,
                        "loading": t(loc, "loading"),
                        "error": t(loc, "preview.docx_error"),
                    })
                    .to_string(),
                ),
                "html" => {
                    // A remote file's path would resolve as a local file:// base
                    // href; better no base at all than the wrong machine's.
                    let base = remote_file_path(&path)
                        .is_none()
                        .then_some(fc.path.as_str());
                    (
                        "html",
                        serde_json::json!({ "text": fc.text, "path": base }).to_string(),
                    )
                }
                "structure" => (
                    "structure",
                    serde_json::json!({ "text": fc.text, "format": "pdb" }).to_string(),
                ),
                "molecule" | "smiles" => (
                    "molecule",
                    serde_json::json!({ "text": fc.text, "smiles": fc.text }).to_string(),
                ),
                "fasta" => ("fasta", serde_json::json!({ "text": fc.text }).to_string()),
                "msa" => ("msa", serde_json::json!({ "text": fc.text }).to_string()),
                _ => ("text", serde_json::json!({ "text": fc.text }).to_string()),
            };
            let _ = mount_preview(mount_kind, &dom_id, &payload).await;
        });
    });
    view! { <div class="rp-heavy" id=dom_id>{move || t(locale.get(), "loading")}</div> }
}

pub(super) fn artifact_preview(a: &Artifact, dom_id: String, locale: Locale) -> impl IntoView {
    match &a.data {
        PreviewData::Table(t) => table_view(t, locale).into_view(),
        PreviewData::Text(s) => view! { <pre class="rp-pre">{s.clone()}</pre> }.into_view(),
        PreviewData::Markdown(s) => {
            let hid_for_effect = dom_id.clone();
            create_effect(move |_| schedule_highlight(hid_for_effect.clone()));
            view! { <div class="md rp-md" id=dom_id inner_html=md_to_html(s)></div> }.into_view()
        }
        PreviewData::Latex { tex, display } => {
            let payload = serde_json::json!({ "tex": tex, "display": display }).to_string();
            view! { <HeavyPreview dom_id=dom_id kind="latex".to_string() payload=payload /> }
                .into_view()
        }
        PreviewData::Fasta(text) => {
            let payload = serde_json::json!({ "text": text }).to_string();
            view! { <HeavyPreview dom_id=dom_id kind="fasta".to_string() payload=payload /> }
                .into_view()
        }
        PreviewData::Smiles(s) => {
            let payload = serde_json::json!({ "smiles": s }).to_string();
            view! { <HeavyPreview dom_id=dom_id kind="molecule".to_string() payload=payload /> }
                .into_view()
        }
        PreviewData::File { path, kind } => view! {
            <p class="rp-path hint">{path.clone()}</p>
            <div class="rp-file-preview" data-file-path=path.clone()>
                <WorkspaceFilePreview dom_id=dom_id path=path.clone() kind=kind.clone() />
            </div>
        }
        .into_view(),
    }
}

#[component]
pub(super) fn CodeBlock(lang: String, body: String) -> impl IntoView {
    let lang_class = if lang.is_empty() {
        "plaintext".to_string()
    } else {
        lang.clone()
    };
    let hid = unique_dom_id("code");
    let hid_for_effect = hid.clone();
    let lang_track = lang_class.clone();
    let body_track = body.clone();
    create_effect(move |_| {
        let _ = (&lang_track, &body_track);
        schedule_highlight(hid_for_effect.clone());
    });
    view! {
        <div class="code-block" id=hid.clone()>
            {(!lang.is_empty()).then(|| view! { <div class="code-lang">{lang.clone()}</div> })}
            <pre class="md-code"><code class=format!("language-{lang_class}")>{body.clone()}</code></pre>
        </div>
    }
}

/// Right-pane code view with a line-number gutter (Claude Science style).
/// The gutter is a plain <pre> (no <code>) so highlight.js skips it.
#[component]
pub(super) fn RpCodeView(lang: String, body: String) -> impl IntoView {
    let lang_class = if lang.is_empty() {
        "plaintext".to_string()
    } else {
        lang.clone()
    };
    let hid = unique_dom_id("rpcode");
    let hid_for_effect = hid.clone();
    let body_track = body.clone();
    create_effect(move |_| {
        let _ = &body_track;
        schedule_highlight(hid_for_effect.clone());
    });
    // split('\n') matches how <pre> renders a trailing newline, keeping the
    // gutter aligned with the body line-for-line.
    let n = body.split('\n').count().max(1);
    let gutter = (1..=n)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    view! {
        <div class="rp-code" id=hid.clone()>
            <pre class="rp-code-gutter">{gutter}</pre>
            <pre class="rp-code-body"><code class=format!("language-{lang_class}")>{body.clone()}</code></pre>
        </div>
    }
}

/// File kinds that open in the full ArtifactModal viewer on click (image/pdf
/// full-size, csv as a dataset table) rather than rendering inline in the pane.
pub(super) fn opens_in_modal(kind: &str) -> bool {
    matches!(kind, "image" | "pdf" | "csv")
}

/// Fire the native save dialog to download a workspace file (backend copies it).
/// Remote-preview paths go out as the ssh:// spelling `download_file` already
/// understands, so the modal's download button works for remote files too.
pub(super) fn download_artifact(path: String) {
    let path = match remote_file_path(&path) {
        Some((context_id, remote_path)) => {
            match crate::context_menu::remote_file_download_uri(context_id, remote_path) {
                Some(uri) => uri,
                None => return,
            }
        }
        None => path,
    };
    spawn_local(async move {
        let arg = to_value(&serde_json::json!({ "path": path })).unwrap();
        let _ = invoke("download_file", arg).await;
    });
}

pub(super) fn keyboard_event_targets_text_entry(ev: &web_sys::KeyboardEvent) -> bool {
    let mut el = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    while let Some(node) = el {
        if node.dyn_ref::<web_sys::HtmlInputElement>().is_some()
            || node.dyn_ref::<web_sys::HtmlTextAreaElement>().is_some()
            || node.tag_name().eq_ignore_ascii_case("select")
            || node.has_attribute("contenteditable")
        {
            return true;
        }
        el = node.parent_element();
    }
    false
}

/// Click-to-expand modal for a produced artifact: shows the full-size
/// image/PDF (or a CSV as a dataset table) plus tabbed provenance
/// (Code/Log/Inputs/Environment) fetched from `get_artifact_provenance`.
/// Provenance is best-effort — a `None` result (or any empty field within it)
/// renders an empty state; the figure never depends on provenance being present.
#[component]
pub(super) fn ArtifactModal(
    path: String,
    name: String,
    kind: String,
    session: Option<String>,
    can_prev: bool,
    can_next: bool,
    on_prev: Callback<()>,
    on_next: Callback<()>,
    on_close: Callback<()>,
    on_open_center: Callback<ModalArtifact>,
    on_open_path: Callback<(String, String)>, // open an input file (path, kind)
    library_items: ReadSignal<Vec<LibraryItem>>,
    on_library_changed: Callback<()>,
) -> impl IntoView {
    let locale = use_locale();
    let prov = create_rw_signal(None::<ArtifactProvenance>);
    let loaded = create_rw_signal(false);
    let tab = create_rw_signal("code");
    let dom_id = unique_dom_id("amodal");
    {
        let path = path.clone();
        let session = session.clone();
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "sessionId": session, "path": path })).unwrap();
            let v = invoke("get_artifact_provenance", arg).await;
            prov.set(
                serde_wasm_bindgen::from_value::<Option<ArtifactProvenance>>(v)
                    .ok()
                    .flatten(),
            );
            loaded.set(true);
        });
    }
    let path_head = path.clone();
    let path_dl = path.clone();
    let center_artifact = (path.clone(), name.clone(), kind.clone());
    let star_path = path.clone();
    let star_session = session.clone();
    let starred = create_memo(move |_| {
        star_session.as_deref().is_some_and(|session| {
            library_items
                .get()
                .iter()
                .any(|item| item.matches_figure(session, &star_path))
        })
    });
    let click_path = path.clone();
    let click_name = name.clone();
    let click_session = session.clone();
    let is_html = kind == "html";
    let is_zoomable = matches!(kind.as_str(), "image" | "pdf");
    let is_docx = kind == "docx";
    let can_star = kind == "image";
    view! {
        <div class="overlay" on:click=move |_| on_close.call(())>
            <div class="modal artifact-modal" class:html-preview=is_html on:click=|ev| ev.stop_propagation()>
                <div class="am-head">
                    <span class="am-name">{name.clone()}</span>
                    {(can_prev || can_next).then(|| view! {
                        <div class="am-nav">
                            <button type="button" class="icon-btn am-nav-btn"
                                disabled=!can_prev
                                aria-label=move || t(locale.get(), "artifact.prev_image")
                                title=move || format!("{} (←)", t(locale.get(), "artifact.prev_image"))
                                on:click=move |_| on_prev.call(())>{compose_icon("chevron-left")}</button>
                            <button type="button" class="icon-btn am-nav-btn"
                                disabled=!can_next
                                aria-label=move || t(locale.get(), "artifact.next_image")
                                title=move || format!("{} (→)", t(locale.get(), "artifact.next_image"))
                                on:click=move |_| on_next.call(())>{compose_icon("chevron-right")}</button>
                        </div>
                    })}
                    <div class="spacer"></div>
                    {can_star.then(|| view! {
                        <button type="button" class="icon-btn" class:starred=move || starred.get()
                            disabled=click_session.is_none()
                            title=move || t(locale.get(), if starred.get() { "library.remove" } else { "library.add" })
                            aria-label=move || t(locale.get(), if starred.get() { "library.remove" } else { "library.add" })
                            aria-pressed=move || starred.get().to_string()
                            on:click=move |_| {
                                let Some(session_id) = click_session.clone() else { return; };
                                let existing = library_items.get_untracked().into_iter().find(|item| {
                                    item.matches_figure(&session_id, &click_path)
                                });
                                let path = click_path.clone();
                                let name = click_name.clone();
                                spawn_local(async move {
                                    let (command, args) = match existing {
                                        Some(item) => (
                                            "delete_library_item",
                                            serde_json::json!({ "id": item.id }),
                                        ),
                                        None => (
                                            "star_library_figure",
                                            serde_json::json!({
                                                "sessionId": session_id,
                                                "path": path,
                                                "name": name,
                                            }),
                                        ),
                                    };
                                    if invoke_checked(command, to_value(&args).unwrap()).await.is_ok() {
                                        on_library_changed.call(());
                                    }
                                });
                            }>
                            {move || compose_icon(if starred.get() { "star-filled" } else { "star" })}
                        </button>
                    })}
                    <button type="button" class="icon-btn"
                        aria-label=move || t(locale.get(), "center.open_file")
                        title=move || t(locale.get(), "center.open_file")
                        on:click=move |_| on_open_center.call(center_artifact.clone())>{compose_icon("expand")}</button>
                    <button class="icon-btn" title=move || t(locale.get(), "artifact.download")
                        on:click=move |_| download_artifact(path_dl.clone())>{compose_icon("download")}</button>
                    <button class="icon-btn" title=move || t(locale.get(), "right.close")
                        on:click=move |_| on_close.call(())>{compose_icon("close")}</button>
                </div>
                <div class="am-figure" class:zoomable-preview=is_zoomable
                    class:docx-preview=is_docx data-file-path=path_head.clone()>
                    <WorkspaceFilePreview dom_id=dom_id path=path_head.clone() kind=kind.clone() />
                </div>
                <div class="am-tabs">
                    {["code","log","inputs","env"].iter().map(|k| {
                        let k = *k;
                        let label_key = format!("artifact.tab.{k}");
                        view! {
                            <button class="am-tab" class:active=move || tab.get()==k
                                on:click=move |_| tab.set(k)>
                                {move || t(locale.get(), &label_key)}</button>
                        }
                    }).collect_view()}
                </div>
                <div class="am-panel">
                    {move || {
                        let loc = locale.get();
                        if !loaded.get() { return view! { <div class="rp-heavy">{t(loc,"loading")}</div> }.into_view(); }
                        let Some(p) = prov.get() else {
                            return view! { <div class="am-empty">{t(loc,"artifact.none")}</div> }.into_view();
                        };
                        match tab.get() {
                            "code" => view! { <RpCodeView lang=p.language.clone() body=p.code.clone() /> }.into_view(),
                            "log" => view! { <pre class="am-log">{p.output.clone()}</pre> }.into_view(),
                            "inputs" => view! {
                                <div class="am-inputs">
                                    {p.inputs.iter().map(|i| {
                                        let ip = i.path.clone();
                                        let linkable = i.produced_here;
                                        let open = on_open_path;
                                        view! {
                                            <button class="am-input" class:linkable=linkable
                                                on:click=move |_| if linkable {
                                                    let kind = file_kind(&ip).unwrap_or("text").to_string();
                                                    open.call((ip.clone(), kind));
                                                }>
                                                {i.path.clone()}</button>
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_view(),
                            _ => match p.env.clone() {
                                None => view! { <div class="am-empty">{t(loc,"artifact.env.none")}</div> }.into_view(),
                                Some(env) => view! {
                                    <table class="am-env">
                                        {env.packages.iter().map(|pk| view! {
                                            <tr><td>{pk.name.clone()}</td><td>{pk.version.clone()}</td></tr>
                                        }).collect_view()}
                                    </table>
                                }.into_view(),
                            },
                        }
                    }}
                </div>
            </div>
        </div>
    }
}

pub(super) fn composer_text_from_user_message(text: &str) -> String {
    [
        "\n\nUploaded files: ",
        "\n\nAttached artifacts: ",
        "\n\nAttached sessions: ",
        "\n\nSelected skills: ",
        "\n\nTarget environments: ",
        "\n\nTarget runtimes: ",
        "\n\nAI source-edit instruction: ",
    ]
    .iter()
    .filter_map(|marker| text.find(marker))
    .min()
    .map(|idx| text[..idx].trim().to_string())
    .unwrap_or_else(|| text.to_string())
}

pub(super) fn user_message_index(items: &[ChatItem], ui_index: usize) -> Option<usize> {
    if !matches!(items.get(ui_index), Some(ChatItem::User(_))) {
        return None;
    }
    Some(
        items
            .iter()
            .take(ui_index + 1)
            .filter(|item| matches!(item, ChatItem::User(_)))
            .count()
            .saturating_sub(1),
    )
}

/// Return a DOM-sized transcript slice without splitting a user turn. A
/// `requested_start` of `usize::MAX` follows the newest available turns.
pub(super) fn transcript_render_window(
    items: &[ChatItem],
    requested_start: usize,
    max_user_turns: usize,
) -> (std::ops::Range<usize>, usize, usize) {
    let user_rows = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            matches!(item, ChatItem::User(_) | ChatItem::QueuedUser(_)).then_some(index)
        })
        .collect::<Vec<_>>();
    let total = user_rows.len();
    if total == 0 {
        return (0..items.len(), 0, 0);
    }
    let max_user_turns = max_user_turns.max(1);
    let latest_start = total.saturating_sub(max_user_turns);
    let start = if requested_start == usize::MAX {
        latest_start
    } else {
        requested_start.min(latest_start)
    };
    let end = (start + max_user_turns).min(total);
    let first_item = if start == 0 { 0 } else { user_rows[start] };
    let last_item = if end == total {
        items.len()
    } else {
        user_rows[end]
    };
    (first_item..last_item, start, total)
}

#[cfg(test)]
mod transcript_render_window_tests {
    use super::transcript_render_window;
    use crate::dto::ChatItem;

    #[test]
    fn limits_complete_user_turns_and_can_follow_the_tail() {
        let items = (0..6)
            .flat_map(|turn| {
                [
                    ChatItem::User(format!("question {turn}")),
                    ChatItem::Assistant {
                        text: format!("answer {turn}"),
                        model: None,
                        resources: Vec::new(),
                    },
                ]
            })
            .collect::<Vec<_>>();

        assert_eq!(transcript_render_window(&items, 0, 2), (0..4, 0, 6));
        assert_eq!(
            transcript_render_window(&items, usize::MAX, 2),
            (8..12, 4, 6)
        );
        assert_eq!(transcript_render_window(&items, 2, 2), (4..8, 2, 6));
    }
}

pub(super) fn focus_composer() {
    focus_element("composer-input");
}

pub(super) fn focus_element(id: &str) {
    focus_element_inner(id, false);
}

pub(super) fn focus_and_select_element(id: &str) {
    focus_element_inner(id, true);
}

fn focus_element_inner(id: &str, select_all: bool) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(el) = doc.get_element_by_id(id) else {
        return;
    };
    let _ = el.dyn_ref::<web_sys::HtmlElement>().map(|e| e.focus());
    if !select_all {
        return;
    }
    if let Some(input) = el.dyn_ref::<web_sys::HtmlInputElement>() {
        input.select();
    } else if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        ta.select();
    }
}

pub(super) fn focus_element_soon(id: &'static str) {
    schedule_focus(id, false);
}

/// Focus a text field after the next paint and select its contents.
/// Used by rename/create modals so Ctrl/⌘A and typing work immediately.
pub(super) fn focus_and_select_soon(id: &'static str) {
    schedule_focus(id, true);
}

fn schedule_focus(id: &'static str, select_all: bool) {
    let focus = Closure::once(move || {
        if select_all {
            focus_and_select_element(id);
        } else {
            focus_element(id);
        }
    });
    if let Some(window) = web_sys::window() {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            focus.as_ref().unchecked_ref(),
            0,
        );
    }
    focus.forget();
}

/// Shared Lucide-style UI icons. Interactive controls must use these SVGs,
/// never font glyphs whose shape varies by platform and fallback font.
pub(super) fn compose_icon(kind: &str) -> impl IntoView {
    let body = match kind {
        "attach" => view! { <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/> }.into_view(),
        "folder" => view! { <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/> }.into_view(),
        "plan" => view! { <path d="M8 6h13"/><path d="M8 12h13"/><path d="M8 18h13"/><path d="M3 6l1 1 2-2"/><path d="M3 12l1 1 2-2"/><path d="M3 18l1 1 2-2"/> }.into_view(),
        "chat" => view! { <path d="M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4z"/><path d="M8 10h8"/><path d="M8 14h5"/> }.into_view(),
        "branch" => view! { <path d="M6 3v6a4 4 0 0 0 4 4h8"/><path d="M18 7v12"/><path d="M14 15l4 4 4-4"/><circle cx="6" cy="3" r="2"/> }.into_view(),
        "chevron-down" => view! { <path d="m6 9 6 6 6-6"/> }.into_view(),
        "chevron-left" => view! { <path d="m15 18-6-6 6-6"/> }.into_view(),
        "chevron-right" => view! { <path d="m9 18 6-6-6-6"/> }.into_view(),
        "expand" => view! { <path d="M15 3h6v6"/><path d="m21 3-7 7"/><path d="M9 21H3v-6"/><path d="m3 21 7-7"/> }.into_view(),
        "download" => view! { <path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/> }.into_view(),
        "upload" => view! { <path d="M12 21V9"/><path d="m7 14 5-5 5 5"/><path d="M5 3h14"/> }.into_view(),
        "sync" => view! { <path d="M20 7h-9"/><path d="m16 3 4 4-4 4"/><path d="M4 17h9"/><path d="m8 21-4-4 4-4"/> }.into_view(),
        "pin" => view! { <path d="M12 17v5"/><path d="M5 17h14"/><path d="m6 3 1 7-3 4h16l-3-4 1-7Z"/> }.into_view(),
        "link" => view! { <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/> }.into_view(),
        "close" => view! { <path d="M18 6 6 18"/><path d="m6 6 12 12"/> }.into_view(),
        "more" => view! { <circle cx="12" cy="5" r="1" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="1" fill="currentColor" stroke="none"/><circle cx="12" cy="19" r="1" fill="currentColor" stroke="none"/> }.into_view(),
        "plus" => view! { <path d="M12 5v14"/><path d="M5 12h14"/> }.into_view(),
        "crop" => view! { <path d="M6 2v14a2 2 0 0 0 2 2h14"/><path d="M2 6h14a2 2 0 0 1 2 2v14"/> }.into_view(),
        "split" => view! { <rect x="3" y="4" width="18" height="16" rx="2"/><path d="M14 4v16"/> }.into_view(),
        "runtime-panel" => view! { <rect x="3" y="3" width="18" height="18" rx="2"/><path d="M14 3v18"/><path d="M3 15h11"/><circle cx="17.5" cy="7" r="1" fill="currentColor" stroke="none"/><circle cx="17.5" cy="11" r="1" fill="currentColor" stroke="none"/> }.into_view(),
        "play" => view! { <path d="M6 4.5v15l13-7.5Z"/> }.into_view(),
        "up" => view! { <path d="m18 15-6-6-6 6"/> }.into_view(),
        "copy" => view! { <rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/> }.into_view(),
        "star" => view! { <path d="m12 2.7 2.85 5.77 6.37.93-4.61 4.49 1.09 6.34L12 17.23l-5.7 3 1.09-6.34L2.78 9.4l6.37-.93Z"/> }.into_view(),
        "star-filled" => view! { <path d="m12 2.7 2.85 5.77 6.37.93-4.61 4.49 1.09 6.34L12 17.23l-5.7 3 1.09-6.34L2.78 9.4l6.37-.93Z" fill="currentColor"/> }.into_view(),
        "edit" => view! { <path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L8 18l-4 1 1-4Z"/> }.into_view(),
        "doc" => view! { <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z"/><path d="M14 2v6h6"/> }.into_view(),
        "image" => view! { <rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="m21 15-5-5L5 21"/> }.into_view(),
        "review" => view! { <circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 1 0 18Z" fill="currentColor" stroke="none"/> }.into_view(),
        "controls" => view! { <path d="M4 21v-7"/><path d="M4 10V3"/><path d="M12 21v-9"/><path d="M12 8V3"/><path d="M20 21v-5"/><path d="M20 12V3"/><path d="M1 14h6"/><path d="M9 8h6"/><path d="M17 16h6"/> }.into_view(),
        "check" => view! { <path d="m20 6-11 11-5-5"/> }.into_view(),
        "skill" => view! { <path d="M19 17V5a2 2 0 0 0-2-2H4"/><path d="M8 21h12a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1H11a1 1 0 0 0-1 1v1a2 2 0 1 1-4 0V5a2 2 0 1 0-4 0v2a1 1 0 0 0 1 1h3"/> }.into_view(),
        "computer" => view! { <rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/> }.into_view(),
        "server" => view! { <rect x="3" y="4" width="18" height="7" rx="1"/><rect x="3" y="13" width="18" height="7" rx="1"/><circle cx="7" cy="7.5" r="0.5" fill="currentColor"/><circle cx="7" cy="16.5" r="0.5" fill="currentColor"/> }.into_view(),
        "terminal" => view! { <path d="m4 17 6-5-6-5"/><path d="M12 19h8"/> }.into_view(),
        "grid" => view! { <rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/> }.into_view(),
        "list" => view! { <path d="M8 6h13"/><path d="M8 12h13"/><path d="M8 18h13"/><path d="M3 6h.01"/><path d="M3 12h.01"/><path d="M3 18h.01"/> }.into_view(),
        _ => view! { <path d="M9 18l6-6-6-6"/> }.into_view(), // chevron
    };
    let size = if matches!(
        kind,
        "chevron" | "chevron-down" | "chevron-left" | "chevron-right"
    ) {
        "16"
    } else {
        "18"
    };
    view! {
        <svg width=size height=size viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>
    }
}

fn attachment_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Small, lazy image preview shared by composer cards and sent messages. The
/// source stays inside the WebView as a data URL and gracefully falls back to
/// an image icon when a native path cannot be read from the active project.
#[component]
pub(super) fn AttachmentThumbnail(path: String, alt: String) -> impl IntoView {
    let source = create_rw_signal(None::<String>);
    let path_for_effect = path;
    create_effect(move |_| {
        let path = path_for_effect.clone();
        spawn_local(async move {
            let args = to_value(&tauri_args::read_file(&path, Some(16 * 1024 * 1024))).unwrap();
            let Ok(value) = invoke_checked("read_file", args).await else {
                return;
            };
            let Ok(file) = serde_wasm_bindgen::from_value::<FileContent>(value) else {
                return;
            };
            if let Some(base64) = file.base64 {
                source.set(Some(format!("data:{};base64,{base64}", file.mime)));
            }
        });
    });
    view! {
        <span class="attachment-thumbnail">
            {move || source.get().map_or_else(
                || view! { <span class="attachment-thumbnail-placeholder">{compose_icon("image")}</span> }.into_view(),
                |src| view! { <img src=src alt=alt.clone() /> }.into_view(),
            )}
        </span>
    }
}

#[component]
pub(super) fn UserMessage(
    text: String,
    ui_index: usize,
    busy: ReadSignal<bool>,
    can_modify: bool,
    on_copy: Callback<String>,
    on_edit: Callback<usize>,
    on_branch: Callback<usize>,
    on_file: Callback<ModalArtifact>,
) -> impl IntoView {
    let locale = use_locale();
    let presentation = user_message_presentation(&text);
    let body = presentation.body;
    let (images, files): (Vec<_>, Vec<_>) = presentation
        .attachments
        .into_iter()
        .partition(|path| file_kind(path) == Some("image"));
    let has_images = !images.is_empty();
    let has_files = !files.is_empty();
    let has_context = !presentation.artifacts.is_empty()
        || !presentation.sessions.is_empty()
        || !presentation.skills.is_empty();
    let has_body = !body.is_empty();
    let image_cards = images
        .into_iter()
        .map(|path| {
            let name = attachment_name(&path);
            let name_for_click = name.clone();
            let path_for_click = path.clone();
            let on_file = on_file.clone();
            view! {
                <button type="button" class="user-attachment-image"
                    title=name.clone()
                    on:click=move |_| on_file.call((path_for_click.clone(), name_for_click.clone(), "image".into()))>
                    <AttachmentThumbnail path=path alt=name.clone() />
                    <span class="user-attachment-image-name">{name}</span>
                </button>
            }
        })
        .collect_view();
    let file_cards = files
        .into_iter()
        .map(|path| {
            let name = attachment_name(&path);
            let name_for_click = name.clone();
            let kind = file_kind(&path).unwrap_or("text").to_string();
            let path_for_click = path.clone();
            let kind_for_click = kind.clone();
            let on_file = on_file.clone();
            view! {
                <button type="button" class="user-attachment-file"
                    title=path.clone()
                    on:click=move |_| on_file.call((path_for_click.clone(), name_for_click.clone(), kind_for_click.clone()))>
                    <span class="user-attachment-file-icon">{compose_icon("doc")}</span>
                    <span class="user-attachment-file-copy">
                        <span class="user-attachment-file-name">{name}</span>
                        <span class="user-attachment-file-meta">{move || t(locale.get(), "attachment.file")}</span>
                    </span>
                    <span class="user-attachment-open">{compose_icon("chevron-right")}</span>
                </button>
            }
        })
        .collect_view();
    let context_cards = [
        ("artifact", "attachment.artifact", presentation.artifacts),
        ("session", "attachment.session", presentation.sessions),
        ("skill", "attachment.skill", presentation.skills),
    ]
    .into_iter()
    .flat_map(|(kind, label_key, items)| {
        items.into_iter().map(move |label| {
            view! {
                <span class=format!("user-context-card {kind}") data-reference-kind=kind>
                    <span class="user-context-icon">{compose_icon(if kind == "skill" { "skill" } else if kind == "session" { "chat" } else { "doc" })}</span>
                    <span class="user-context-copy">
                        <span class="user-context-label">{label}</span>
                        <span class="user-context-meta">{move || t(locale.get(), label_key)}</span>
                    </span>
                </span>
            }
        })
    })
    .collect_view();
    view! {
        <div class="user-bubble">
            {has_images.then(|| view! { <div class="user-attachment-images">{image_cards}</div> })}
            {has_files.then(|| view! { <div class="user-attachment-files">{file_cards}</div> })}
            {has_context.then(|| view! { <div class="user-context-cards">{context_cards}</div> })}
            {has_body.then(|| view! { <div class="body">{body}</div> })}
            <div class="msg-actions">
                <button
                    type="button"
                    class="msg-btn"
                    disabled=move || busy.get()
                    title=move || t(locale.get(), "msg.copy")
                    on:click=move |_| on_copy.call(text.clone())
                >{move || t(locale.get(), "msg.copy")}</button>
                {can_modify.then(|| view! {
                    <button
                        type="button"
                        class="msg-btn"
                        disabled=move || busy.get()
                        title=move || t(locale.get(), "msg.edit")
                        on:click=move |_| on_edit.call(ui_index)
                    >{move || t(locale.get(), "msg.edit")}</button>
                    <button
                        type="button"
                        class="msg-btn"
                        title=move || t(locale.get(), "msg.branch")
                        on:click=move |_| on_branch.call(ui_index)
                    >{move || t(locale.get(), "msg.branch")}</button>
                })}
            </div>
        </div>
    }
}

#[component]
pub(super) fn AssistantMessage(
    text: String,
    model: Option<String>,
    resources: Vec<MessageResource>,
    artifacts: Vec<Artifact>,
    source_item: usize,
    on_artifact: Callback<usize>,
    on_file: Callback<ModalArtifact>,
    on_copy: Callback<String>,
) -> impl IntoView {
    let locale = use_locale();
    let arts_for_html = artifacts.clone();
    let resources_for_html = resources.clone();
    let text_for_html = text.clone();
    let html = create_memo(move |_| {
        enrich_md_html(
            md_to_html(&text_for_html),
            &arts_for_html,
            &resources_for_html,
            locale.get(),
        )
    });
    let hid = unique_dom_id("md");
    let hid_for_effect = hid.clone();
    create_effect(move |_| {
        let _ = html.get();
        schedule_highlight(hid_for_effect.clone());
    });
    let hid_for_resources = hid.clone();
    let resources_for_effect = resources.clone();
    create_effect(move |_| {
        let _ = html.get();
        let dom_id = hid_for_resources.clone();
        let resources = resources_for_effect.clone();
        spawn_local(async move {
            for resource in resources
                .into_iter()
                .filter(|resource| resource.status == "ready" && resource.kind == "image")
            {
                let Some(version_id) = resource.artifact_version_id else {
                    continue;
                };
                let Ok(value) = invoke_checked(
                    "read_artifact_version",
                    to_value(&serde_json::json!({ "versionId": version_id })).unwrap(),
                )
                .await
                else {
                    continue;
                };
                let Ok(file) = serde_wasm_bindgen::from_value::<FileContent>(value) else {
                    continue;
                };
                let Some(base64) = file.base64 else {
                    continue;
                };
                let selector = format!(r#"#{dom_id} [data-resource-id="{}"]"#, resource.id);
                if let Some(element) = web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|document| document.query_selector(&selector).ok().flatten())
                {
                    let _ = element
                        .set_attribute("src", &format!("data:{};base64,{base64}", file.mime));
                    let _ = element.set_attribute("class", "resource-inline-image");
                }
            }
        });
    });
    let on_artifact = on_artifact.clone();
    let on_file = on_file.clone();
    let arts_for_click = artifacts.clone();
    let resources_for_click = resources.clone();
    let generated = artifacts
        .iter()
        .enumerate()
        .filter(|(_, artifact)| artifact.source_item == source_item)
        .map(|(index, artifact)| {
            (
                index,
                artifact.name.clone(),
                artifact.kind,
                artifact.superseded,
            )
        })
        .collect::<Vec<_>>();
    let generated_count = generated.len();
    let generated_cards = generated.into_iter().map(|(index, name, kind, superseded)| {
        let on_artifact = on_artifact.clone();
        view! {
            <button type="button" class="message-artifact-card" class:superseded=superseded
                disabled=superseded
                data-artifact-name=name.clone()
                on:click=move |_| on_artifact.call(index)>
                <span class=format!("rp-badge {kind}")>{kind}</span>
                <span class="message-artifact-name">{name}</span>
                {superseded.then(|| view! { <span class="message-artifact-status">{move || t(locale.get(), "artifact.updated")}</span> })}
            </button>
        }
    }).collect_view();
    let text_for_disabled = text.clone();
    let text_for_click_copy = text;
    view! {
        <div class="role">
            <span class="role-brand">{move || t(locale.get(), "chat.assistant")}</span>
            {move || model.clone().filter(|m| !m.is_empty()).map(|m| view! {
                <span class="role-model">{m}</span>
            })}
        </div>
        <div class="assistant-wrap">
            <div class="body md" id=hid.clone()
                inner_html=move || html.get()
                on:click=move |ev: web_sys::MouseEvent| {
                    handle_md_click(
                        &ev,
                        &arts_for_click,
                        &resources_for_click,
                        &on_artifact,
                        &on_file,
                    )
                }></div>
            {(generated_count > 0).then(|| view! {
                <div class="message-artifacts">
                    <div class="message-artifacts-label">{format!("Generated · {generated_count}")}</div>
                    <div class="message-artifact-cards">{generated_cards}</div>
                </div>
            })}
            <div class="msg-actions">
                <button
                    type="button"
                    class="msg-icon-btn"
                    title=move || t(locale.get(), "ctx.copy_message")
                    aria-label=move || t(locale.get(), "ctx.copy_message")
                    disabled=move || text_for_disabled.trim().is_empty()
                    on:click=move |_| on_copy.call(text_for_click_copy.clone())
                >
                    <span class="gi copy" aria-hidden="true"></span>
                </button>
            </div>
        </div>
    }
}

#[component]
pub(super) fn ToolBlock(
    name: String,
    ok: Option<bool>,
    input: String,
    output: String,
) -> impl IntoView {
    let locale = use_locale();
    let open = ok != Some(true);
    let lang = tool_lang(&name).to_string();
    let hid = unique_dom_id("tool");
    let hid_for_effect = hid.clone();
    let has_input = !input.is_empty();
    let has_output = !output.is_empty();
    let input_track = input.clone();
    let output_track = output.clone();
    let lang_track = lang.clone();
    create_effect(move |_| {
        let _ = (&input_track, &output_track, &lang_track);
        schedule_highlight(hid_for_effect.clone());
    });
    let name_for_label = name.clone();
    let input_label = move || {
        if matches!(name_for_label.as_str(), "python" | "r") {
            t(locale.get(), "tool.copy_code")
        } else {
            t(locale.get(), "tool.copy_input")
        }
    };

    view! {
        <details class="tool" open=open>
            <summary class="head">
                <span>{name.clone()}</span>
                {match ok {
                    Some(true) => view!{ <span class="ok">"✓"</span> }.into_view(),
                    Some(false) => view!{ <span class="fail">"✗"</span> }.into_view(),
                    None => view!{ <span class="run"><span class="run-dot"></span>{move || t(locale.get(), "tool.running")}</span> }.into_view(),
                }}
            </summary>
            <div class="tool-panel" id=hid.clone()>
                <div class="tool-actions">
                    {has_input.then(|| {
                        let text = input.clone();
                        view! {
                            <button type="button" class="tool-btn" on:click=move |_| copy_text(text.clone())>
                                {input_label}
                            </button>
                        }
                    })}
                    {has_output.then(|| {
                        let text = output.clone();
                        view! {
                            <button type="button" class="tool-btn" on:click=move |_| copy_text(text.clone())>{move || t(locale.get(), "tool.copy_output")}</button>
                        }
                    })}
                </div>
                {has_input.then(|| view! {
                    <pre class="tool-input md-code"><code class=format!("language-{lang}")>{input.clone()}</code></pre>
                })}
                {has_output.then(|| view! {
                    <pre class="tool-output md-code"><code class="language-plaintext">{output.clone()}</code></pre>
                })}
            </div>
        </details>
    }
}

/// Parse a rendered plan checklist line (`[x] text` / `[~] text` / `[ ] text`)
/// into (status_class, text). Mirrors `update_plan`'s render in wisp-tools.
pub(super) fn plan_step_line(line: &str) -> Option<(&'static str, &str)> {
    for (prefix, cls) in [("[x] ", "done"), ("[~] ", "running"), ("[ ] ", "pending")] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some((cls, rest));
        }
    }
    None
}

pub(super) fn approval_allow_label_key(scope: &str) -> &'static str {
    match scope {
        "session" => "approval.allow_session",
        "project" => "approval.allow_project",
        "global" => "approval.allow_global",
        _ => "approval.allow_once",
    }
}

#[component]
pub(super) fn ApprovalCard(
    tool: String,
    preview: String,
    session_id: String,
    on_decide: Callback<(String, bool, Option<String>, String)>,
) -> impl IntoView {
    let locale = use_locale();
    let is_plan = tool == "update_plan";
    let show_feedback = create_rw_signal(false);
    let feedback = create_rw_signal(String::new());
    let approval_scope = create_rw_signal(String::from("once"));
    let feedback_ready = move || !feedback.get().trim().is_empty();
    create_effect(move |_| {
        if show_feedback.get() {
            focus_element_soon("plan-feedback-input");
        }
    });
    let lang = tool_lang(&tool).to_string();
    // For the plan card, `preview` is the rendered checklist; parse it into rows.
    let plan_steps: Vec<(&'static str, String)> = if is_plan {
        preview
            .lines()
            .filter_map(|l| plan_step_line(l).map(|(c, t)| (c, t.to_string())))
            .collect()
    } else {
        vec![]
    };
    let tool_for_title = tool.clone();
    let title = move || {
        let loc = locale.get();
        match tool_for_title.as_str() {
            _ if is_plan => t(loc, "approval.review_plan"),
            "python" => t(loc, "approval.run_python"),
            "r" => t(loc, "approval.run_r"),
            "shell" => t(loc, "approval.run_shell"),
            _ => tf(loc, "approval.run_tool", &[("tool", &tool_for_title)]),
        }
    };
    let sid_allow = session_id.clone();
    let sid_deny = session_id.clone();
    let sid_feedback = create_rw_signal(session_id);
    view! {
        <div class="approval-wrap">
            <div class="approval-wait-line">{move || t(locale.get(), "approval.waiting_line")}</div>
            <div class="approval-card" class:plan=is_plan>
                <div class="approval-head">
                    <span class="approval-title">{title}</span>
                    <span class="approval-status">
                        <span class="approval-dot"></span>
                        {move || t(locale.get(), "approval.waiting")}
                    </span>
                </div>
                {if is_plan {
                    view! {
                        <div class="plan-steps">
                            {plan_steps.into_iter().map(|(cls, text)| view! {
                                <div class=format!("plan-step {cls}")>
                                    <span class="plan-step-mark"></span>
                                    <span class="plan-step-text">{text}</span>
                                </div>
                            }).collect_view()}
                        </div>
                    }.into_view()
                } else {
                    let show_tag = !tool.is_empty();
                    let tag = tool.clone();
                    let show_code = !preview.is_empty();
                    let p = preview.clone();
                    let lang = lang.clone();
                    view! {
                        {show_tag.then(|| view! {
                            <div class="approval-tags"><span class="approval-tag">{tag}</span></div>
                        })}
                        {show_code.then(|| view! {
                            <details class="approval-code" open=true>
                                <summary>{move || t(locale.get(), "approval.code")}</summary>
                                <pre><code class=format!("language-{lang}")>{p}</code></pre>
                            </details>
                        })}
                    }.into_view()
                }}
                <p class="approval-hint">{move || t(locale.get(), if is_plan { "approval.plan_hint" } else { "approval.hint" })}</p>
                <div class="approval-actions">
                    {(!is_plan).then(|| view! {
                        <label class="approval-scope">
                            <span>{move || t(locale.get(), "approval.scope")}</span>
                            <select
                                aria-label=move || t(locale.get(), "approval.scope")
                                prop:value=move || approval_scope.get()
                                on:change=move |ev| approval_scope.set(dom_value(&ev))>
                                <option value="once">{move || t(locale.get(), "approval.scope.once")}</option>
                                <option value="session">{move || t(locale.get(), "approval.scope.session")}</option>
                                <option value="project">{move || t(locale.get(), "approval.scope.project")}</option>
                                <option value="global">{move || t(locale.get(), "approval.scope.global")}</option>
                            </select>
                        </label>
                    })}
                    <button type="button" class="primary"
                        on:click=move |_| {
                            let scope = if is_plan { "once".into() } else { approval_scope.get() };
                            on_decide.call((sid_allow.clone(), true, None, scope));
                        }>
                        {move || {
                            if is_plan {
                                t(locale.get(), "approval.plan_approve").to_string()
                            } else {
                                t(locale.get(), approval_allow_label_key(&approval_scope.get())).to_string()
                            }
                        }}
                    </button>
                    <button type="button"
                        on:click=move |_| on_decide.call((sid_deny.clone(), false, None, "once".into()))>
                        {move || t(locale.get(), if is_plan { "approval.plan_reject" } else { "confirm.deny" })}
                    </button>
                    {is_plan.then(|| view! {
                        <button type="button" on:click=move |_| show_feedback.update(|open| *open = !*open)>
                            {move || t(locale.get(), "approval.plan_other")}
                        </button>
                    })}
                </div>
                {is_plan.then(move || {
                    view! {
                        <Show when=move || show_feedback.get()>
                            <div class="plan-feedback"
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ev.key() == "Escape" && !ime_composing(&ev) {
                                        // Collapse feedback before the window-level
                                        // handler rejects the whole plan.
                                        ev.prevent_default();
                                        feedback.set(String::new());
                                        show_feedback.set(false);
                                    }
                                }>
                                <textarea
                                    id="plan-feedback-input"
                                    class="plan-feedback-input"
                                    rows="3"
                                    prop:value=move || feedback.get()
                                    placeholder=move || t(locale.get(), "approval.plan_feedback_placeholder")
                                    on:input=move |ev| feedback.set(event_target_value(&ev))
                                ></textarea>
                                <div class="plan-feedback-actions">
                                    <button
                                        type="button"
                                        class="primary"
                                        disabled=move || !feedback_ready()
                                        on:click=move |_| {
                                            let text = feedback.get().trim().to_string();
                                            if !text.is_empty() {
                                                on_decide.call((sid_feedback.get_untracked(), false, Some(text), "once".into()));
                                            }
                                        }
                                    >
                                        {move || t(locale.get(), "approval.plan_feedback_submit")}
                                    </button>
                                    <button
                                        type="button"
                                        on:click=move |_| {
                                            feedback.set(String::new());
                                            show_feedback.set(false);
                                        }
                                    >
                                        {move || t(locale.get(), "approval.plan_feedback_cancel")}
                                    </button>
                                </div>
                            </div>
                        </Show>
                    }
                })}
            </div>
        </div>
    }
}

#[component]
pub(super) fn ProjectsScreen(
    locale: RwSignal<Locale>,
    running: RwSignal<HashSet<String>>,
    approval_pending: ReadSignal<HashSet<String>>,
    open_error: RwSignal<Option<String>>,
    on_open: Callback<String>,
    on_open_session: Callback<(String, String)>,
    on_open_artifact: Callback<(String, String, String)>,
    on_open_settings: Callback<()>,
    on_open_library: Callback<()>,
    on_open_demo: Callback<()>,
    on_search: Callback<()>,
) -> impl IntoView {
    let projects = create_rw_signal(Vec::<ProjectSummary>::new());
    let recent = create_rw_signal(Vec::<RecentSession>::new());
    let artifact_hits = create_rw_signal(Vec::<ArtifactInfo>::new());
    let search_open = create_rw_signal(false);
    let search_query = create_rw_signal(String::new());
    let search_active = create_rw_signal(0usize);
    let demo_count = create_rw_signal(0usize);
    let creating = create_rw_signal(false);
    let new_name = create_rw_signal(String::new());
    let new_dir = create_rw_signal(String::new());
    let new_desc = create_rw_signal(String::new());
    let new_ctx = create_rw_signal(String::new());
    let importing = create_rw_signal(false);
    let syncing_projects = create_rw_signal(HashSet::<String>::new());
    let sync_notice = create_rw_signal(None::<(bool, String)>);
    let sync_conflict_project = create_rw_signal(None::<String>);
    // Pending project deletion, awaiting in-app confirmation. Native
    // `window.confirm()` is a no-op in this webview (wry's WKUIDelegate doesn't
    // implement the JS confirm panel), so it always returned false and the ✕
    // did nothing — use an in-app modal instead.
    let pending_delete = create_rw_signal(None::<String>);

    let reload = move || {
        spawn_local(async move {
            let v = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(v) {
                projects.set(list);
            }
            let r = invoke("list_recent_sessions", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<RecentSession>>(r) {
                recent.set(list);
            }
            let dm = invoke("list_demos", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DemoInfo>>(dm) {
                demo_count.set(list.len());
            }
        });
    };
    reload();

    // Refresh dashboard when a background turn starts/finishes or waits on approval.
    create_effect(move |_| {
        running.get();
        approval_pending.get();
        reload();
    });

    create_effect(move |_| {
        if search_open.get() {
            search_query.get();
            refresh_artifact_search(search_query, artifact_hits);
            focus_element_soon("project-search-input");
        }
    });

    create_effect(move |_| {
        if creating.get() {
            focus_and_select_soon("new-project-name");
        }
    });

    create_effect(move |_| {
        search_open.get();
        search_query.get();
        search_active.set(0);
    });

    let search_count = move || {
        let q = search_query.get().trim().to_lowercase();
        let projects_n = projects
            .get()
            .into_iter()
            .filter(|p| contains_search(&q, &[&p.name, &p.description]))
            .take(HOME_SEARCH_PROJECT_LIMIT)
            .count();
        let artifacts_n = artifact_hits
            .get()
            .into_iter()
            .take(HOME_SEARCH_ARTIFACT_LIMIT)
            .count();
        let sessions_n = recent
            .get()
            .into_iter()
            .filter(|s| contains_search(&q, &[&s.title]))
            .take(HOME_SEARCH_SESSION_LIMIT)
            .count();
        projects_n + artifacts_n + sessions_n + 1
    };

    let run_search_action = Callback::new(move |idx: usize| {
        let q = search_query.get().trim().to_lowercase();
        let mut pos = 0usize;
        for p in projects
            .get()
            .into_iter()
            .filter(|p| contains_search(&q, &[&p.name, &p.description]))
            .take(HOME_SEARCH_PROJECT_LIMIT)
        {
            if pos == idx {
                search_open.set(false);
                on_open.call(p.id);
                return;
            }
            pos += 1;
        }
        for a in artifact_hits
            .get()
            .into_iter()
            .take(HOME_SEARCH_ARTIFACT_LIMIT)
        {
            if pos == idx {
                search_open.set(false);
                let path = stored_artifact_path(&a.path);
                let kind = file_kind(&a.name)
                    .or_else(|| file_kind(&path))
                    .unwrap_or_else(|| {
                        if a.kind.starts_with("image/") {
                            "image"
                        } else if a.kind.contains("pdf") {
                            "pdf"
                        } else if a.kind.contains("csv") {
                            "csv"
                        } else {
                            "text"
                        }
                    })
                    .to_string();
                on_open_artifact.call((path, a.name, kind));
                return;
            }
            pos += 1;
        }
        for s in recent
            .get()
            .into_iter()
            .filter(|s| contains_search(&q, &[&s.title]))
            .take(HOME_SEARCH_SESSION_LIMIT)
        {
            if pos == idx {
                search_open.set(false);
                on_open_session.call((s.project_id, s.id));
                return;
            }
            pos += 1;
        }
        if pos == idx {
            search_open.set(false);
            creating.set(true);
        }
    });

    let choose_dir = move |_| {
        spawn_local(async move {
            let v = invoke("pick_directory", JsValue::UNDEFINED).await;
            if let Ok(Some(p)) = serde_wasm_bindgen::from_value::<Option<String>>(v) {
                new_dir.set(p);
            }
        })
    };

    let submit = move |_| {
        let (n, d, desc, ctx) = (new_name.get(), new_dir.get(), new_desc.get(), new_ctx.get());
        if n.trim().is_empty() || d.trim().is_empty() {
            return;
        }
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "name": n, "workspaceDir": d, "description": desc, "agentContext": ctx,
            }))
            .unwrap();
            let v = invoke("create_project", arg).await;
            if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectSummary>(v) {
                new_name.set(String::new());
                new_dir.set(String::new());
                new_desc.set(String::new());
                new_ctx.set(String::new());
                creating.set(false);
                on_open.call(p.id);
            }
        });
    };

    let delete = move |id: String| {
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
            let _ = invoke("delete_project", arg).await;
            let v = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(v) {
                projects.set(list);
            }
        });
    };
    let delete_confirmed = delete.clone(); // used by the confirm modal below

    let import_project = move |_| {
        if importing.get_untracked() {
            return;
        }
        importing.set(true);
        open_error.set(None);
        spawn_local(async move {
            match invoke_checked("import_project", JsValue::UNDEFINED).await {
                Ok(value) => {
                    if let Ok(Some(project)) =
                        serde_wasm_bindgen::from_value::<Option<ProjectSummary>>(value)
                    {
                        on_open.call(project.id);
                    }
                }
                Err(error) => {
                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                    open_error.set(Some(message));
                }
            }
            importing.set(false);
        });
    };

    let resolve_sync_conflict = Callback::new(move |strategy: String| {
        let Some(id) = sync_conflict_project.get_untracked() else {
            return;
        };
        if syncing_projects.with_untracked(|ids| ids.contains(&id)) {
            return;
        }
        syncing_projects.update(|ids| {
            ids.insert(id.clone());
        });
        open_error.set(None);
        sync_notice.set(Some((
            true,
            t(locale.get_untracked(), "projects.sync.running").into(),
        )));
        spawn_local(async move {
            let args =
                to_value(&serde_json::json!({ "id": id.clone(), "strategy": strategy })).unwrap();
            match invoke_checked("resolve_project_sync", args).await {
                Ok(value) => {
                    if let Ok(result) = serde_wasm_bindgen::from_value::<ProjectSyncResult>(value) {
                        let loc = locale.get_untracked();
                        let text = if result.direction == "pull" {
                            tf(
                                loc,
                                "projects.sync.pulled",
                                &[("n", &result.downloaded_files.to_string())],
                            )
                        } else {
                            tf(
                                loc,
                                "projects.sync.pushed",
                                &[("n", &result.uploaded_files.to_string())],
                            )
                        };
                        sync_notice.set(Some((true, text)));
                    }
                    sync_conflict_project.set(None);
                    reload();
                }
                Err(error) => {
                    sync_notice.set(None);
                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                    open_error.set(Some(message));
                }
            }
            syncing_projects.update(|ids| {
                ids.remove(&id);
            });
        });
    });

    // Local Escape stack — ProjectsScreen owns its own modals, so the App
    // window listener cannot see `creating` / `pending_delete`.
    window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        if ev.key() != "Escape" || ev.default_prevented() || ime_composing(ev) {
            return;
        }
        if pending_delete.get().is_some() {
            ev.prevent_default();
            pending_delete.set(None);
            return;
        }
        if sync_conflict_project.get().is_some() {
            ev.prevent_default();
            sync_conflict_project.set(None);
            return;
        }
        if search_open.get() {
            ev.prevent_default();
            search_open.set(false);
            return;
        }
        if creating.get() {
            ev.prevent_default();
            creating.set(false);
        }
    });

    view! {
        <div class="projects-screen">
            <div class="projects-head">
                <div class="projects-title">"Wisp Science"<span class="beta">"Beta"</span></div>
                <div class="projects-actions">
                    <button type="button" class="projects-icon-btn"
                        title=move || t(locale.get(), "sidebar.library")
                        aria-label=move || t(locale.get(), "sidebar.library")
                        on:click=move |_| on_open_library.call(())>
                        {compose_icon("star")}
                    </button>
                    <button type="button" class="projects-icon-btn"
                        title=move || t(locale.get(), "projects.search")
                        aria-label=move || t(locale.get(), "projects.search")
                        on:click=move |_| on_search.call(())>
                        <span class="gi search"></span>
                    </button>
                    <button type="button" class="projects-icon-btn"
                        title=move || t(locale.get(), "sidebar.settings")
                        aria-label=move || t(locale.get(), "sidebar.settings")
                        on:click=move |_| on_open_settings.call(())>
                        <span class="gi gear"></span>
                    </button>
                    <button type="button" class="btn-ghost projects-import"
                        disabled=move || importing.get()
                        on:click=import_project>
                        {compose_icon("upload")}<span>{move || t(locale.get(), "projects.import")}</span>
                    </button>
                    <button class="btn-primary" on:click=move |_| creating.set(true)>
                        <span class="new-plus">"+"</span>{move || t(locale.get(), "projects.new")}
                    </button>
                </div>
            </div>
            {move || open_error.get().map(|message| view! {
                <div class="project-open-error" role="alert">{message}</div>
            })}
            {move || search_open.get().then(|| view! {
                <div class="project-search-overlay" on:click=move |_| search_open.set(false)>
                    <div class="project-search-dialog" role="dialog" aria-label=move || t(locale.get(), "projects.search")
                        on:click=|ev| ev.stop_propagation()>
                        <div class="project-search-input">
                            <span class="gi search"></span>
                            <input id="project-search-input" type="text" inputmode="search" autofocus=true
                                autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false"
                                placeholder=move || t(locale.get(), "projects.search_ph")
                                prop:value=move || search_query.get()
                                on:input=move |ev| search_query.set(event_target_value(&ev))
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ime_composing(&ev) { return; }
                                    let key = ev.key();
                                    let last = search_count().saturating_sub(1);
                                    match key.as_str() {
                                        "Escape" => {
                                            ev.prevent_default();
                                            search_open.set(false);
                                        }
                                        "ArrowDown" => {
                                            ev.prevent_default();
                                            search_active.update(|i| *i = (*i + 1).min(last));
                                        }
                                        "ArrowUp" => {
                                            ev.prevent_default();
                                            search_active.update(|i| *i = i.saturating_sub(1));
                                        }
                                        "Enter" => {
                                            ev.prevent_default();
                                            run_search_action.call(search_active.get().min(last));
                                        }
                                        _ => {}
                                    }
                                } />
                        </div>
                        <div class="project-search-results">
                            {move || {
                                let loc = locale.get();
                                let q = search_query.get().trim().to_lowercase();
                                let mut idx = 0usize;

                                let project_start = idx;
                                let project_rows = projects
                                    .get()
                                    .into_iter()
                                    .filter(|p| contains_search(&q, &[&p.name, &p.description]))
                                    .take(HOME_SEARCH_PROJECT_LIMIT)
                                    .map(|p| {
                                        let row_idx = idx;
                                        idx += 1;
                                        let open = run_search_action;
                                        let sessions = tf(loc, "projects.sessions_n", &[("n", &p.session_count.to_string())]);
                                        let when = format_relative_time(p.updated_at, loc);
                                        view! {
                                            <button type="button" class="project-search-row" class:active=move || search_active.get() == row_idx
                                                on:click=move |_| open.call(row_idx)>
                                                <span class="gi folder"></span>
                                                <span class="project-search-main">
                                                    <span class="project-search-title">{p.name.clone()}</span>
                                                    <span class="project-search-sub">
                                                        {sessions}{(!when.is_empty()).then(|| format!(" · {when}")).unwrap_or_default()}
                                                    </span>
                                                </span>
                                            </button>
                                        }
                                    })
                                    .collect_view();
                                let has_project_rows = idx > project_start;
                                let artifact_start = idx;
                                let artifact_rows = artifact_hits
                                    .get()
                                    .into_iter()
                                    .take(HOME_SEARCH_ARTIFACT_LIMIT)
                                    .map(|a| {
                                        let row_idx = idx;
                                        idx += 1;
                                        let open = run_search_action;
                                        let badge = artifact_badge(&a.kind, &a.name);
                                        let when = format_relative_time(a.ts, loc);
                                        view! {
                                            <button type="button" class="project-search-row" class:active=move || search_active.get() == row_idx
                                                on:click=move |_| open.call(row_idx)>
                                                <span class="gi doc"></span>
                                                <span class="project-search-main">
                                                    <span class="project-search-title">{a.name.clone()}</span>
                                                    <span class="project-search-sub">
                                                        {a.path.clone()}{(!when.is_empty()).then(|| format!(" · {when}")).unwrap_or_default()}
                                                    </span>
                                                </span>
                                                <span class="project-search-badge">{badge}</span>
                                            </button>
                                        }
                                    })
                                    .collect_view();
                                let has_artifact_rows = idx > artifact_start;
                                let session_start = idx;
                                let session_rows = recent
                                    .get()
                                    .into_iter()
                                    .filter(|s| contains_search(&q, &[&s.title]))
                                    .take(HOME_SEARCH_SESSION_LIMIT)
                                    .map(|s| {
                                        let row_idx = idx;
                                        idx += 1;
                                        let open = run_search_action;
                                        let status = SessionStatusKind::from_str(&s.status);
                                        view! {
                                            <button type="button" class="project-search-row" class:active=move || search_active.get() == row_idx
                                                on:click=move |_| open.call(row_idx)>
                                                <span class="gi bubble"></span>
                                                <span class="project-search-main">
                                                    <span class="project-search-title">{s.title.clone()}</span>
                                                    <span class="project-search-sub">{format_relative_time(s.ts, loc)}</span>
                                                </span>
                                                <SessionStatusBadge status=status locale=locale />
                                            </button>
                                        }
                                    })
                                    .collect_view();
                                let has_session_rows = idx > session_start;
                                view! {
                                    {has_project_rows.then(|| view! {
                                        <div class="project-search-section">
                                            <div class="project-search-label">{move || t(locale.get(), "projects.title")}</div>
                                            {project_rows}
                                        </div>
                                    })}
                                    {has_artifact_rows.then(|| view! {
                                        <div class="project-search-section">
                                            <div class="project-search-label">{move || t(locale.get(), "projects.search_artifacts")}</div>
                                            {artifact_rows}
                                        </div>
                                    })}
                                    {has_session_rows.then(|| view! {
                                        <div class="project-search-section">
                                            <div class="project-search-label">{move || t(locale.get(), "projects.recent")}</div>
                                            {session_rows}
                                        </div>
                                    })}
                                }.into_view()
                            }}
                        </div>
                        <button type="button" class="project-search-new"
                            class:active=move || search_active.get() + 1 == search_count()
                            on:click=move |_| {
                                search_open.set(false);
                                creating.set(true);
                            }>
                            <span class="gi plus"></span>
                            <span>{move || t(locale.get(), "projects.new")}</span>
                        </button>
                        <div class="project-search-foot">
                            <span><kbd>"↑↓"</kbd>{move || t(locale.get(), "projects.search_nav")}</span>
                            <span><kbd>"↵"</kbd>{move || t(locale.get(), "projects.search_open")}</span>
                            <span><kbd>"esc"</kbd>{move || t(locale.get(), "projects.search_close")}</span>
                        </div>
                    </div>
                </div>
            })}
            {move || creating.get().then(|| view! {
                <div class="overlay">
                    <div class="modal proj-settings-modal" role="dialog" aria-modal="true">
                        <div class="ps-head">
                            <h2>{move || t(locale.get(), "projects.new")}</h2>
                            <button type="button" class="ps-close"
                                title=move || t(locale.get(), "projects.cancel")
                                on:click=move |_| creating.set(false)>{compose_icon("close")}</button>
                        </div>
                        <label>
                            {move || t(locale.get(), "proj_settings.name")}
                            <input id="new-project-name" autofocus=true
                                placeholder=move || t(locale.get(), "projects.name_ph")
                                prop:value=move || new_name.get()
                                on:input=move |e| new_name.set(event_target_value(&e)) />
                        </label>
                        <label>
                            {move || t(locale.get(), "projects.directory")}
                            <div class="pn-dir">
                                <button type="button" class="btn-ghost" on:click=choose_dir>
                                    {move || t(locale.get(), "projects.choose_dir")}</button>
                                <span class="path">{move || new_dir.get()}</span>
                            </div>
                        </label>
                        <label>
                            {move || t(locale.get(), "proj_settings.description")}
                            <span class="ps-hint">{move || t(locale.get(), "proj_settings.description_hint")}</span>
                            <textarea class="ps-textarea" rows="2"
                                prop:value=move || new_desc.get()
                                on:input=move |ev| new_desc.set(event_target_value(&ev))></textarea>
                        </label>
                        <label>
                            {move || t(locale.get(), "proj_settings.agent_context")}
                            <span class="ps-hint">{move || t(locale.get(), "proj_settings.agent_context_hint")}</span>
                            <textarea class="ps-textarea ps-ctx" rows="8"
                                prop:value=move || new_ctx.get()
                                on:input=move |ev| new_ctx.set(event_target_value(&ev))></textarea>
                        </label>
                        <div class="row">
                            <button type="button" on:click=move |_| creating.set(false)>
                                {move || t(locale.get(), "projects.cancel")}</button>
                            <button type="button" class="primary"
                                disabled=move || new_name.get().trim().is_empty() || new_dir.get().trim().is_empty()
                                on:click=submit>{move || t(locale.get(), "projects.create")}</button>
                        </div>
                    </div>
                </div>
            })}
            <div class="projects-cols">
                <div class="projects-col">
                    <h2>{move || t(locale.get(), "projects.title")}</h2>
                    <button type="button" class="proj-card proj-example" on:click=move |_| on_open_demo.call(())>
                        <div>
                            <div class="pc-name">
                                {move || t(locale.get(), "projects.example")}
                                <span class="pc-tag">{move || t(locale.get(), "projects.example_tag")}</span>
                            </div>
                            <div class="pc-meta">{move || tf(locale.get(), "projects.sessions_n", &[("n", &demo_count.get().to_string())])}</div>
                        </div>
                    </button>
                    {move || {
                        let loc = locale.get();
                        let list = projects.get();
                        if list.is_empty() && !creating.get() {
                            return view! {}.into_view();
                        }
                        list.into_iter().map(|p| {
                            let id_open = p.id.clone();
                            let id_del = p.id.clone();
                            let id_win = p.id.clone();
                            let id_export = p.id.clone();
                            let id_sync = p.id.clone();
                            let id_sync_disabled = p.id.clone();
                            let id_code = p.id.clone();
                            let meta = tf(loc, "projects.sessions_n", &[("n", &p.session_count.to_string())]);
                            let active = p.running_count + p.needs_you_count;
                            let dot_class = if p.running_count > 0 { "running" } else { "ready" };
                            let when = format_relative_time(p.updated_at, loc);
                            let sync_when = p.last_synced_at
                                .map(|timestamp| format_relative_time(timestamp, loc))
                                .filter(|value| !value.is_empty());
                            let sync_label = if p.sync_configured {
                                Some(sync_when.as_deref().map_or_else(
                                    || t(loc, "projects.sync.enabled").into(),
                                    |when| tf(loc, "projects.sync.last", &[("when", when)]),
                                ))
                            } else {
                                None
                            };
                            view! {
                                <div class="proj-card">
                                    <button type="button" class="proj-card-main" on:click=move |_| on_open.call(id_open.clone())>
                                    <div class="pc-main">
                                        <div class="pc-name-row">
                                            <div class="pc-name">{p.name.clone()}</div>
                                            {(active > 0).then(|| view! {
                                                <span class=format!("pc-dot {dot_class}")>
                                                    <span class="pc-dot-mark"></span>
                                                    <span class="pc-dot-n">{active}</span>
                                                </span>
                                            })}
                                        </div>
                                        <div class="pc-meta-row">
                                            <span class="pc-meta">{meta}</span>
                                            {(!when.is_empty()).then(|| view! { <span class="pc-when">{when.clone()}</span> })}
                                            {sync_label.clone().map(|label| view! { <span class="pc-sync-state">{label}</span> })}
                                        </div>
                                    </div>
                                    </button>
                                    <div class="pc-actions">
                                    <button class="pc-sync" title=t(loc, "projects.sync.now")
                                        aria-label=t(loc, "projects.sync.now")
                                        disabled=move || syncing_projects.with(|ids| ids.contains(&id_sync_disabled))
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            let id = id_sync.clone();
                                            if syncing_projects.with(|ids| ids.contains(&id)) { return; }
                                            syncing_projects.update(|ids| { ids.insert(id.clone()); });
                                            sync_notice.set(Some((true, t(locale.get_untracked(), "projects.sync.running").into())));
                                            open_error.set(None);
                                            spawn_local(async move {
                                                let args = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                                match invoke_checked("sync_project", args).await {
                                                    Ok(value) => {
                                                        if let Ok(result) = serde_wasm_bindgen::from_value::<ProjectSyncResult>(value) {
                                                            let loc = locale.get_untracked();
                                                            let text = match result.direction.as_str() {
                                                                "push" => tf(loc, "projects.sync.pushed", &[("n", &result.uploaded_files.to_string())]),
                                                                "pull" => tf(loc, "projects.sync.pulled", &[("n", &result.downloaded_files.to_string())]),
                                                                _ => t(loc, "projects.sync.current").into(),
                                                            };
                                                            let text = if result.skipped_paths.is_empty() {
                                                                text
                                                            } else {
                                                                format!("{text} {}", tf(loc, "projects.sync.skipped", &[("n", &result.skipped_paths.len().to_string())]))
                                                            };
                                                            sync_notice.set(Some((true, text)));
                                                        }
                                                        reload();
                                                    }
                                                    Err(error) => {
                                                        sync_notice.set(None);
                                                        let raw = js_error_text(error);
                                                        if raw.contains("Sync conflict") {
                                                            sync_conflict_project.set(Some(id.clone()));
                                                        } else {
                                                            let message = localize_backend(locale.get_untracked(), &raw);
                                                            open_error.set(Some(message));
                                                        }
                                                    }
                                                }
                                                syncing_projects.update(|ids| { ids.remove(&id); });
                                            });
                                        }>{compose_icon("sync")}</button>
                                    <button class="pc-sync-code" title=t(loc, "projects.sync.copy_code")
                                        aria-label=t(loc, "projects.sync.copy_code")
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            let id = id_code.clone();
                                            open_error.set(None);
                                            spawn_local(async move {
                                                let args = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                match invoke_checked("project_sync_code", args).await {
                                                    Ok(value) => {
                                                        if let Ok(code) = serde_wasm_bindgen::from_value::<String>(value) {
                                                            copy_text(code);
                                                            sync_notice.set(Some((true, t(locale.get_untracked(), "projects.sync.code_copied").into())));
                                                        }
                                                    }
                                                    Err(error) => {
                                                        let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                                                        open_error.set(Some(message));
                                                    }
                                                }
                                            });
                                        }>{compose_icon("link")}</button>
                                    <button class="pc-export" title=t(loc, "projects.export")
                                        aria-label=t(loc, "projects.export")
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            open_error.set(None);
                                            let id = id_export.clone();
                                            spawn_local(async move {
                                                let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                if let Err(error) = invoke_checked("export_project", arg).await {
                                                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                                                    open_error.set(Some(message));
                                                }
                                            });
                                        }>{compose_icon("download")}</button>
                                    <button class="pc-window" title=t(loc, "projects.new_window")
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            let id = id_win.clone();
                                            spawn_local(async move {
                                                let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                let _ = invoke("open_project_window", arg).await;
                                            });
                                        }>{compose_icon("copy")}</button>
                                    <button class="pc-del" title=t(loc, "projects.delete")
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            pending_delete.set(Some(id_del.clone()));
                                        }>{compose_icon("close")}</button>
                                    </div>
                                </div>
                            }
                        }).collect_view()
                    }}
                </div>
                <div class="projects-col">
                    <h2>{move || t(locale.get(), "projects.recent")}</h2>
                    {move || recent.get().into_iter().map(|s| {
                        let (pid, sid) = (s.project_id.clone(), s.id.clone());
                        let status = SessionStatusKind::from_str(&s.status);
                        view! {
                            <button type="button" class="proj-card proj-recent" data-testid="recent-session-card"
                                on:click=move |_| on_open_session.call((pid.clone(), sid.clone()))>
                                <div class="pc-main">
                                    <div class="pc-name-row">
                                        <div class="pc-name">{s.title.clone()}</div>
                                        <SessionStatusBadge status=status locale=locale />
                                    </div>
                                </div>
                            </button>
                        }
                    }).collect_view()}
                </div>
            </div>
            <div class="projects-footer">
                <span>{move || t(locale.get(), "projects.star_hint")}</span>
                <button type="button" class="projects-star-link"
                    on:click=move |_| open_external_url("https://github.com/xuzhougeng/wisp-science".into())>
                    {move || t(locale.get(), "projects.star_link")}
                </button>
            </div>
            {move || sync_notice.get().map(|(ok, text)| view! {
                <div class="projects-sync-notice" class:ok=move || ok>{text}</div>
            })}
            {move || sync_conflict_project.get().map(|_| {
                let use_remote = resolve_sync_conflict;
                let use_local = resolve_sync_conflict;
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal project-sync-conflict-modal" role="dialog"
                            aria-label=move || t(locale.get(), "projects.sync.conflict_title")>
                            <h2>{move || t(locale.get(), "projects.sync.conflict_title")}</h2>
                            <p class="hint">{move || t(locale.get(), "projects.sync.conflict_hint")}</p>
                            <p class="hint">{move || t(locale.get(), "projects.sync.conflict_backup")}</p>
                            <div class="row project-sync-conflict-actions">
                                <button type="button" on:click=move |_| sync_conflict_project.set(None)>
                                    {move || t(locale.get(), "projects.cancel")}</button>
                                <button type="button" on:click=move |_| use_remote.call("remote".into())>
                                    {move || t(locale.get(), "projects.sync.use_remote")}</button>
                                <button type="button" class="primary" on:click=move |_| use_local.call("local".into())>
                                    {move || t(locale.get(), "projects.sync.use_local")}</button>
                            </div>
                        </div>
                    </div>
                }
            })}
            {move || pending_delete.get().map(|id| {
                let confirm_del = delete_confirmed.clone();
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal">
                            <h2>{move || t(locale.get(), "confirm.title")}</h2>
                            <div class="hint">{move || t(locale.get(), "projects.delete_confirm")}</div>
                            <div class="row">
                                <button on:click=move |_| pending_delete.set(None)>
                                    {move || t(locale.get(), "settings.cancel")}</button>
                                <button class="primary" on:click=move |_| {
                                    pending_delete.set(None);
                                    confirm_del(id.clone());
                                }>{move || t(locale.get(), "confirm.approve")}</button>
                            </div>
                        </div>
                    </div>
                }
            })}
        </div>
    }
}

/// Apply a transcript mutation to the right session: the live `items` view when
/// `fid` is the active session, otherwise the background cache keyed by `fid`.
/// This is what lets a second conversation stream while the user views another.
pub(super) fn route_items(
    active: RwSignal<Option<String>>,
    items: RwSignal<Vec<ChatItem>>,
    transcripts: RwSignal<HashMap<String, Vec<ChatItem>>>,
    fid: &str,
    f: impl FnOnce(&mut Vec<ChatItem>),
) {
    if active.get().as_deref() == Some(fid) {
        items.update(f);
    } else {
        transcripts.update(|m| f(m.entry(fid.to_string()).or_insert_with(Vec::new)));
    }
}

/// A dedicated project window (#52) carries `?project=<id>` in its URL. Returns
/// that id so the window opens straight into the project and skips the landing.
/// Project ids are UUIDs or "default" — no percent-decoding needed.
pub(super) fn url_project_param() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let q = search.strip_prefix('?').unwrap_or(&search);
    q.split('&')
        .find_map(|p| p.strip_prefix("project="))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

#[component]
pub(super) fn CommandPalette(
    open: RwSignal<bool>,
    current_project_id: Signal<Option<String>>,
    on_open_project: Callback<String>,
    on_open_session: Callback<(String, String)>,
    on_open_artifact: Callback<(String, String, String)>,
    on_command: Callback<&'static str>,
    on_new_session: Callback<()>,
    on_project_settings: Callback<()>,
    on_manage_skills: Callback<()>,
    on_attach: Callback<ComposerReferenceChip>,
) -> impl IntoView {
    let locale = use_locale();
    let query = create_rw_signal(String::new());
    let active = create_rw_signal(0usize);
    let projects = create_rw_signal(Vec::<ProjectSummary>::new());
    let artifacts = create_rw_signal(Vec::<ArtifactInfo>::new());
    let sessions = create_rw_signal(Vec::<SessionSearchInfo>::new());
    create_effect(move |_| {
        if !open.get() {
            return;
        }
        let q = query.get();
        spawn_local(async move {
            let p = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(p) {
                projects.set(rows);
            }
            let a = invoke(
                "search_artifacts",
                to_value(&serde_json::json!({ "query": q, "limit": 12, "allProjects": true }))
                    .unwrap(),
            )
            .await;
            if query.get_untracked() == q {
                if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<ArtifactInfo>>(a) {
                    artifacts.set(rows);
                }
            }
            let s = invoke(
                "search_sessions",
                to_value(&serde_json::json!({ "query": q, "limit": 12 })).unwrap(),
            )
            .await;
            if query.get_untracked() == q {
                if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SessionSearchInfo>>(s) {
                    sessions.set(rows);
                }
            }
        });
    });
    create_effect(move |_| {
        open.get();
        query.get();
        active.set(0);
    });
    create_effect(move |_| {
        if open.get() {
            focus_element_soon("command-palette-input");
        }
    });
    let items = create_memo(move |_| {
        let q = query.get().trim().to_lowercase();
        let current = current_project_id.get();
        let mut out = Vec::new();
        let mut ps: Vec<_> = projects
            .get()
            .into_iter()
            .filter(|p| contains_search(&q, &[&p.name, &p.description]))
            .collect();
        ps.sort_by_key(|p| (current.as_deref() != Some(p.id.as_str()), p.name.clone()));
        out.extend(ps.into_iter().map(CommandPaletteItem::Project));
        let mut ars = artifacts.get();
        ars.sort_by_key(|a| {
            (
                current.as_deref() != a.project_id.as_deref(),
                std::cmp::Reverse(a.ts),
            )
        });
        out.extend(ars.into_iter().map(CommandPaletteItem::Artifact));
        let mut ss = sessions.get();
        ss.sort_by_key(|s| {
            (
                current.as_deref() != Some(s.project_id.as_str()),
                std::cmp::Reverse(s.activity_at),
            )
        });
        out.extend(ss.into_iter().map(CommandPaletteItem::Session));
        out.push(CommandPaletteItem::Command("new"));
        out.push(CommandPaletteItem::Command("check-updates"));
        out.push(CommandPaletteItem::Command("star-us"));
        if current.is_some() {
            out.push(CommandPaletteItem::Command("settings"));
            out.push(CommandPaletteItem::Command("skills"));
        }
        out
    });
    let open_item = Callback::new(move |idx: usize| {
        let Some(item) = items.get().get(idx).cloned() else {
            return;
        };
        open.set(false);
        match item {
            CommandPaletteItem::Project(p) => on_open_project.call(p.id),
            CommandPaletteItem::Artifact(a) => {
                let kind = file_kind(&a.name)
                    .or_else(|| file_kind(&a.path))
                    .unwrap_or("text")
                    .to_string();
                on_open_artifact.call((format!("artifact:{}", a.id), a.name, kind));
            }
            CommandPaletteItem::Session(s) => on_open_session.call((s.project_id, s.id)),
            CommandPaletteItem::Command("new") => on_new_session.call(()),
            CommandPaletteItem::Command("check-updates") => on_command.call("check-updates"),
            CommandPaletteItem::Command("star-us") => on_command.call("star-us"),
            CommandPaletteItem::Command("settings") => on_project_settings.call(()),
            CommandPaletteItem::Command("skills") => on_manage_skills.call(()),
            CommandPaletteItem::Command(_) => {}
        }
    });
    let attach_item = Callback::new(move |idx: usize| {
        let list = items.get();
        let item = list
            .get(idx)
            .cloned()
            .filter(|item| {
                matches!(
                    item,
                    CommandPaletteItem::Artifact(_) | CommandPaletteItem::Session(_)
                )
            })
            .or_else(|| {
                list.into_iter().find(|item| {
                    matches!(
                        item,
                        CommandPaletteItem::Artifact(_) | CommandPaletteItem::Session(_)
                    )
                })
            });
        let Some(item) = item else {
            return;
        };
        match item {
            CommandPaletteItem::Artifact(a) => on_attach.call(ComposerReferenceChip::Artifact {
                id: a.id,
                name: a.name,
            }),
            CommandPaletteItem::Session(s) => on_attach.call(ComposerReferenceChip::Session {
                id: s.id,
                title: s.title,
                project_name: s.project_name,
            }),
            _ => return,
        }
        open.set(false);
        focus_composer();
    });
    view! {
        {move || open.get().then(|| view! {
            <div class="project-search-overlay" on:click=move |_| open.set(false)>
                <div class="project-search-dialog" role="dialog" aria-label="Search"
                    on:click=|ev| ev.stop_propagation()>
                    <div class="project-search-input">
                        <span class="gi search"></span>
                        <input id="command-palette-input" type="text" inputmode="search" autofocus=true
                            autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false"
                            placeholder=move || t(locale.get(), "command.search_ph")
                            prop:value=move || query.get()
                            on:input=move |ev| query.set(event_target_value(&ev))
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ime_composing(&ev) { return; }
                                let n = items.get().len();
                                match ev.key().as_str() {
                                    "Escape" => { ev.prevent_default(); open.set(false); }
                                    "ArrowDown" => { ev.prevent_default(); if n > 0 { let next = (active.get() + 1) % n; active.set(next); scroll_picker_item(".project-search-dialog:not(.action-palette) .project-search-row", next); } }
                                    "ArrowUp" => { ev.prevent_default(); if n > 0 { let next = (active.get() + n - 1) % n; active.set(next); scroll_picker_item(".project-search-dialog:not(.action-palette) .project-search-row", next); } }
                                    "Enter" if ev.shift_key() => { ev.prevent_default(); attach_item.call(active.get()); }
                                    "Enter" => { ev.prevent_default(); open_item.call(active.get()); }
                                    _ => {}
                                }
                            } />
                    </div>
                    <div class="project-search-results">
                        {move || items.get().into_iter().enumerate().map(|(i, item)| {
                            let (icon, title, sub) = match item {
                                CommandPaletteItem::Project(p) => ("folder", p.name, p.description),
                                CommandPaletteItem::Artifact(a) => ("doc", a.name, a.project_name.unwrap_or_default()),
                                CommandPaletteItem::Session(s) => ("bubble", s.title, s.project_name),
                                CommandPaletteItem::Command("new") => ("plus", t(locale.get(), "projects.new").to_string(), t(locale.get(), "command.category")),
                                CommandPaletteItem::Command("check-updates") => ("gear", t(locale.get(), "command.check_updates").to_string(), t(locale.get(), "command.category")),
                                CommandPaletteItem::Command("star-us") => ("star", t(locale.get(), "command.star_us").to_string(), t(locale.get(), "command.category")),
                                CommandPaletteItem::Command("settings") => ("gear", t(locale.get(), "proj_settings.title").to_string(), t(locale.get(), "command.category")),
                                CommandPaletteItem::Command("skills") => ("grid", t(locale.get(), "settings.nav.skills").to_string(), t(locale.get(), "command.category")),
                                CommandPaletteItem::Command(_) => ("doc", String::new(), String::new()),
                            };
                            view! {
                                <button type="button" class="project-search-row" class:active=move || active.get() == i
                                    on:mousemove=move |_| active.set(i)
                                    on:click=move |_| open_item.call(i)>
                                    <span class=format!("gi {icon}")></span>
                                    <span class="project-search-main">
                                        <span class="project-search-title">{title}</span>
                                        {(!sub.trim().is_empty()).then(|| view! { <span class="project-search-sub">{sub}</span> })}
                                    </span>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                    <div class="project-search-foot"><span><kbd>"↑↓"</kbd>{t(locale.get(), "command.hint.navigate")}</span><span><kbd>"↵"</kbd>{t(locale.get(), "command.hint.open")}</span><span><kbd>"⇧↵"</kbd>{t(locale.get(), "command.hint.attach")}</span><span><kbd>"esc"</kbd>{t(locale.get(), "command.hint.close")}</span><span class="palette-version">{concat!("v", env!("CARGO_PKG_VERSION"))}</span></div>
                </div>
            </div>
        })}
    }
}

#[component]
pub(super) fn ActionPalette(
    open: RwSignal<bool>,
    on_action: Callback<&'static str>,
) -> impl IntoView {
    let locale = use_locale();
    let query = create_rw_signal(String::new());
    let active = create_rw_signal(0usize);
    create_effect(move |_| {
        if !open.get() {
            return;
        }
        query.set(String::new());
        active.set(0);
        let focus = Closure::once(|| {
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let Some(input) = doc.get_element_by_id("action-palette-input") else {
                return;
            };
            let _ = input.dyn_ref::<web_sys::HtmlElement>().map(|el| el.focus());
        });
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                focus.as_ref().unchecked_ref(),
                0,
            );
        }
        focus.forget();
    });
    let actions = create_memo(move |_| {
        let loc = locale.get();
        let general = t(loc, "command.group.general").to_string();
        let navigate = t(loc, "command.group.navigate").to_string();
        let appearance = t(loc, "command.group.appearance").to_string();
        let entries = [
            (
                "new",
                "plus",
                "command.new_session",
                general.clone(),
                "Ctrl/⌘ N",
            ),
            (
                "search",
                "search",
                "command.search",
                general.clone(),
                "Ctrl/⌘ K",
            ),
            (
                "settings",
                "gear",
                "command.settings",
                general.clone(),
                "Ctrl/⌘ ,",
            ),
            (
                "check-updates",
                "gear",
                "command.check_updates",
                general.clone(),
                "",
            ),
            ("star-us", "star", "command.star_us", general.clone(), ""),
            (
                "project-settings",
                "gear",
                "command.project_settings",
                general.clone(),
                "",
            ),
            ("skills", "grid", "command.skills", general, ""),
            (
                "projects",
                "folder",
                "command.projects",
                navigate.clone(),
                "",
            ),
            (
                "toggle-sidebar",
                "panel",
                "command.toggle_sidebar",
                navigate.clone(),
                "Ctrl/⌘ B",
            ),
            (
                "artifacts",
                "grid",
                "command.artifacts",
                navigate.clone(),
                "",
            ),
            ("notebook", "doc", "command.notebook", navigate.clone(), ""),
            ("files", "doc", "command.files", navigate.clone(), ""),
            (
                "provenance",
                "copy",
                "command.provenance",
                navigate.clone(),
                "",
            ),
            (
                "contexts",
                "server",
                "command.contexts",
                navigate.clone(),
                "",
            ),
            (
                "side-chat",
                "bubble",
                "command.side_chat",
                navigate.clone(),
                "",
            ),
            ("close-panel", "panel", "command.close_panel", navigate, ""),
            (
                "theme-light",
                "gear",
                "command.theme_light",
                appearance.clone(),
                "",
            ),
            (
                "theme-dark",
                "gear",
                "command.theme_dark",
                appearance.clone(),
                "",
            ),
            (
                "theme-system",
                "gear",
                "command.theme_system",
                appearance,
                "",
            ),
        ];
        let q = query.get().trim().to_lowercase();
        entries
            .into_iter()
            .filter_map(|(id, icon, key, group, shortcut)| {
                let title = t(loc, key).to_string();
                contains_search(&q, &[id, &title, &group]).then_some(CommandAction {
                    id,
                    icon,
                    title,
                    group,
                    shortcut,
                })
            })
            .collect::<Vec<_>>()
    });
    let run = Callback::new(move |index: usize| {
        let Some(action) = actions.get().get(index).cloned() else {
            return;
        };
        open.set(false);
        on_action.call(action.id);
    });
    view! {
        {move || open.get().then(|| view! {
            <div class="project-search-overlay action-palette-overlay" on:click=move |_| open.set(false)>
                <div class="project-search-dialog action-palette" role="dialog" aria-label="Command Palette"
                    on:click=|ev| ev.stop_propagation()>
                    <div class="project-search-input">
                        <span class="gi search"></span>
                        <input id="action-palette-input" type="text" inputmode="search" autofocus=true
                            autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false"
                            placeholder=move || t(locale.get(), "command.placeholder")
                            prop:value=move || query.get()
                            on:input=move |ev| { query.set(event_target_value(&ev)); active.set(0); }
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ime_composing(&ev) { return; }
                                let n = actions.get().len();
                                match ev.key().as_str() {
                                    "Escape" => { ev.prevent_default(); open.set(false); }
                                    "ArrowDown" => {
                                        ev.prevent_default();
                                        if n > 0 {
                                            let next = (active.get() + 1) % n;
                                            active.set(next);
                                            scroll_picker_item(".action-palette .project-search-row", next);
                                        }
                                    }
                                    "ArrowUp" => {
                                        ev.prevent_default();
                                        if n > 0 {
                                            let next = (active.get() + n - 1) % n;
                                            active.set(next);
                                            scroll_picker_item(".action-palette .project-search-row", next);
                                        }
                                    }
                                    "Enter" => { ev.prevent_default(); run.call(active.get()); }
                                    _ => {}
                                }
                            } />
                    </div>
                    <div class="project-search-results action-palette-results">
                        {move || {
                            let rows = actions.get();
                            rows.into_iter().enumerate().map(|(i, action)| {
                                let previous_group = (i > 0).then(|| actions.get().get(i - 1).map(|a| a.group.clone())).flatten();
                                let show_group = previous_group.as_deref() != Some(action.group.as_str());
                                view! {
                                    {show_group.then(|| view! { <div class="action-palette-group">{action.group.clone()}</div> })}
                                    <button type="button" class="project-search-row action-palette-row" class:active=move || active.get() == i
                                        on:mousemove=move |_| active.set(i)
                                        on:click=move |_| run.call(i)>
                                        <span class=format!("gi {}", action.icon)></span>
                                        <span class="project-search-main"><span class="project-search-title">{action.title}</span></span>
                                        {(!action.shortcut.is_empty()).then(|| view! { <kbd class="action-shortcut">{action.shortcut}</kbd> })}
                                    </button>
                                }
                            }).collect_view()
                        }}
                    </div>
                    <div class="project-search-foot"><span><kbd>"↑↓"</kbd>{t(locale.get(), "command.hint.navigate")}</span><span><kbd>"↵"</kbd>{t(locale.get(), "command.hint.run")}</span><span><kbd>"esc"</kbd>{t(locale.get(), "command.hint.close")}</span></div>
                </div>
            </div>
        })}
    }
}
