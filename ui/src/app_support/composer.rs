use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComposerSendAction {
    Normal,
    BranchNew,
    /// Guide choice: hand the message to the running task, which folds it in
    /// at its next loop iteration (ACP sessions fall back to plain queueing).
    GuideAppend,
    /// Guide choice: stop the running task, roll its unfinished work out of
    /// the model context, and send this message as the replacement.
    InterruptReplace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComposerPickerMode {
    Artifact,
    Session,
    Skill,
}

#[derive(Clone, PartialEq)]
pub(crate) enum ComposerPickerItem {
    Artifact(ArtifactInfo),
    Session(SessionSearchInfo),
    Project {
        id: String,
        name: String,
    },
    Skill(SkillRow),
    Workflow(WorkflowTemplate),
    /// Built-in slash command handled by the app itself (e.g. `/compact`).
    /// Selecting one fills the composer instead of attaching a chip.
    Command {
        name: String,
        description: String,
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

#[derive(Clone)]
pub(crate) enum ComposerReferenceChip {
    FilePath {
        path: String,
    },
    Artifact {
        id: String,
        name: String,
    },
    Session {
        id: String,
        title: String,
        project_name: String,
    },
    Project {
        id: String,
        name: String,
    },
    Skill {
        name: String,
    },
    Workflow {
        id: String,
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
pub(crate) struct ComposerQuote {
    pub(crate) text: String,
    pub(crate) source: Option<String>,
}

impl ComposerQuote {
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            source: None,
        }
    }

    pub(crate) fn from_selection(text: impl Into<String>, source: Option<String>) -> Self {
        Self {
            text: text.into(),
            source,
        }
    }

    pub(crate) fn workspace_source(&self) -> Option<&str> {
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

/// True only when a selection has a real source and that source is the active
/// center preview. Comparing the two `Option`s directly would also make
/// `(None, None)` match, misclassifying an ordinary transcript selection.
pub(crate) fn selection_targets_center_file(
    source: Option<&str>,
    center_file: Option<&str>,
) -> bool {
    matches!((source, center_file), (Some(source), Some(center_file)) if source == center_file)
}

#[derive(Clone, PartialEq)]
pub(crate) enum CommandPaletteItem {
    Project(ProjectSummary),
    Artifact(ArtifactInfo),
    Session(SessionSearchInfo),
    Command(&'static str),
}

#[derive(Clone, PartialEq)]
pub(crate) struct CommandAction {
    pub(crate) id: &'static str,
    pub(crate) icon: &'static str,
    pub(crate) title: String,
    pub(crate) group: String,
    pub(crate) shortcut: String,
}

impl ComposerReferenceChip {
    pub(crate) fn key(&self) -> String {
        match self {
            Self::FilePath { path } => format!("file-path:{path}"),
            Self::Artifact { id, .. } => format!("artifact:{id}"),
            Self::Session { id, .. } => format!("session:{id}"),
            Self::Project { id, .. } => format!("project:{id}"),
            Self::Skill { name } => format!("skill:{name}"),
            Self::Workflow { id, .. } => format!("workflow:{id}"),
            Self::Context { id, .. } => format!("context:{id}"),
            Self::Runtime {
                context_id,
                language,
                ..
            } => format!("runtime:{context_id}:{language}"),
        }
    }

    pub(crate) fn label(&self) -> String {
        match self {
            Self::FilePath { path } => path
                .rsplit(['/', '\\'])
                .find(|s| !s.is_empty())
                .unwrap_or(path)
                .to_string(),
            Self::Artifact { name, .. } | Self::Skill { name } | Self::Workflow { name, .. } => {
                name.clone()
            }
            Self::Session {
                title,
                project_name,
                ..
            } => format!("{project_name} / {title}"),
            Self::Project { name, .. } => format!("#project · {name}"),
            Self::Context { label, .. } => label.clone(),
            Self::Runtime {
                context_label,
                language,
                ..
            } => format!("{} · {context_label}", language_display(language)),
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::FilePath { .. } => "file-path",
            Self::Artifact { .. } => "artifact",
            Self::Session { .. } => "session",
            Self::Project { .. } => "project",
            Self::Skill { .. } => "skill",
            Self::Workflow { .. } => "workflow",
            Self::Context { .. } => "context",
            Self::Runtime { .. } => "runtime",
        }
    }

    pub(crate) fn arg(&self) -> Option<ComposerReferenceArg> {
        Some(match self {
            Self::FilePath { .. } => return None,
            Self::Artifact { id, .. } => ComposerReferenceArg::Artifact { id: id.clone() },
            Self::Session { id, .. } => ComposerReferenceArg::Session { id: id.clone() },
            Self::Project { id, .. } => ComposerReferenceArg::Project { id: id.clone() },
            Self::Skill { name } => ComposerReferenceArg::Skill { name: name.clone() },
            Self::Workflow { id, .. } => ComposerReferenceArg::Workflow { id: id.clone() },
            Self::Context { id, .. } => ComposerReferenceArg::Context { id: id.clone() },
            Self::Runtime {
                context_id,
                language,
                ..
            } => ComposerReferenceArg::Runtime {
                context_id: context_id.clone(),
                language: language.clone(),
            },
        })
    }
}

/// File clipboard context is deliberately excluded from upload/vision arguments.
pub(crate) fn attach_clipboard_paths(
    references: RwSignal<Vec<ComposerReferenceChip>>,
    value: JsValue,
) {
    let paths: Vec<String> = serde_wasm_bindgen::from_value(value).unwrap_or_default();
    references.update(|items| {
        for path in paths.into_iter().filter(|path| !path.is_empty()) {
            let chip = ComposerReferenceChip::FilePath { path };
            if !items.iter().any(|item| item.key() == chip.key()) {
                items.push(chip);
            }
        }
    });
}

pub(crate) fn composer_attachment_key(name: &str, idx: usize) -> String {
    format!("att-{idx}-{name}")
}

fn ready_attachment_key(path: &str) -> String {
    format!("path:{path}")
}

/// Attach an already-project-relative (or absolute native) file or directory path as a chip.
/// Returns false when the path was already attached.
pub(crate) fn attach_ready_path(
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

pub(crate) fn parse_upload_results(v: JsValue) -> Vec<UploadFileResult> {
    if v.is_null() || v.is_undefined() {
        return vec![];
    }
    serde_wasm_bindgen::from_value(v).unwrap_or_default()
}

pub(crate) fn file_list_len(files: &JsValue) -> usize {
    js_sys::Reflect::get(files, &JsValue::from_str("length"))
        .ok()
        .and_then(|n| n.as_f64())
        .map(|n| n as usize)
        .unwrap_or(0)
}

pub(crate) fn begin_uploads(
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

pub(crate) fn finish_uploads(
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
                let path = info.location.unwrap_or(info.path);
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
pub(crate) fn close_details_ancestor(ev: &web_sys::MouseEvent) {
    let el = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    if let Some(details) = el.and_then(|e| e.closest("details").ok().flatten()) {
        details.remove_attribute("open").ok();
    }
}

pub(crate) fn queue_uploads(
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

pub(crate) fn upload_from_input(
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

pub(crate) fn upload_from_paste(
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

pub(crate) fn attachment_paths(items: &[ComposerAttachment]) -> Vec<String> {
    items
        .iter()
        .filter_map(|a| match a {
            ComposerAttachment::Ready { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

pub(crate) fn message_with_attachments(text: &str, paths: &[String]) -> String {
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
fn message_with_quotes_inner(
    text: &str,
    quotes: &[ComposerQuote],
    include_edit_instruction: bool,
) -> String {
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
    if include_edit_instruction {
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
    }
    out.trim_end().to_string()
}

pub(crate) fn message_with_quotes(text: &str, quotes: &[ComposerQuote]) -> String {
    message_with_quotes_inner(text, quotes, true)
}

/// Side chat is intentionally read-only. It still needs the source label for
/// useful context, but must not inherit the main composer's file-edit command.
pub(crate) fn message_with_read_only_quotes(text: &str, quotes: &[ComposerQuote]) -> String {
    message_with_quotes_inner(text, quotes, false)
}

/// Chip label for a quoted selection: first line, capped at 40 chars.
pub(crate) fn quote_label(text: &str) -> String {
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
pub(crate) fn message_with_composer_context(
    text: &str,
    paths: &[String],
    references: &[ComposerReferenceChip],
    quotes: &[ComposerQuote],
) -> String {
    let mut message = message_with_attachments(&message_with_quotes(text, quotes), paths);
    let mut file_paths = Vec::new();
    let mut artifacts = Vec::new();
    let mut sessions = Vec::new();
    let mut projects = Vec::new();
    let mut skills = Vec::new();
    let mut workflows = Vec::new();
    let mut contexts = Vec::new();
    let mut runtimes = Vec::new();
    for reference in references {
        match reference {
            ComposerReferenceChip::FilePath { path } => {
                file_paths.push(serde_json::to_string(path).unwrap_or_default())
            }
            ComposerReferenceChip::Artifact { name, .. } => artifacts.push(name.clone()),
            ComposerReferenceChip::Session {
                title,
                project_name,
                ..
            } => sessions.push(format!("{project_name} / {title}")),
            ComposerReferenceChip::Project { name, .. } => projects.push(name.clone()),
            ComposerReferenceChip::Skill { name } => skills.push(name.clone()),
            ComposerReferenceChip::Workflow { name, .. } => workflows.push(name.clone()),
            ComposerReferenceChip::Context { label, .. } => contexts.push(label.clone()),
            ComposerReferenceChip::Runtime { .. } => runtimes.push(reference.label()),
        }
    }
    for (label, values) in [
        ("Referenced local file paths (not imported)", file_paths),
        ("Attached artifacts", artifacts),
        ("Attached sessions", sessions),
        ("Project context", projects),
        ("Selected skills", skills),
        ("Selected workflows", workflows),
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

fn utf16_to_byte_index(text: &str, wanted: usize) -> Option<usize> {
    let mut utf16 = 0;
    for (byte, ch) in text.char_indices() {
        if utf16 == wanted {
            return Some(byte);
        }
        utf16 += ch.len_utf16();
        if utf16 > wanted {
            return None;
        }
    }
    (utf16 == wanted).then_some(text.len())
}

/// Return the active composer token ending at the textarea's UTF-16 caret.
/// ASCII word boundaries suppress email, URL, path, and fraction false positives;
/// CJK text may directly precede a trigger because it normally has no spaces.
pub(crate) fn active_composer_trigger(
    text: &str,
    caret_utf16: usize,
) -> Option<(usize, usize, ComposerPickerMode, String)> {
    let end = utf16_to_byte_index(text, caret_utf16)?;
    let (at, trigger) = text[..end]
        .char_indices()
        .rev()
        .find(|(_, c)| matches!(c, '@' | '#' | '/'))?;
    if let Some(previous) = text[..at].chars().next_back() {
        let embedded_ascii = previous.is_ascii_alphanumeric() || previous == '_';
        let path_or_url = trigger == '/' && matches!(previous, ':' | '/' | '\\' | '.');
        if embedded_ascii || path_or_url {
            return None;
        }
    }
    let query = &text[at + trigger.len_utf8()..end];
    if query.chars().any(char::is_whitespace) {
        return None;
    }
    let mode = match trigger {
        '@' => ComposerPickerMode::Artifact,
        '#' => ComposerPickerMode::Session,
        '/' => ComposerPickerMode::Skill,
        _ => return None,
    };
    Some((at, end, mode, query.to_string()))
}

/// Whether an input edit should keep or open the picker for the active token.
///
/// Windows IMEs may report punctuation such as `@` and `#` as composition
/// input, with `InputEvent.data` absent or different from the committed text.
/// The textarea value and caret have already established which trigger was
/// entered, so opening must depend on the insertion kind rather than `data`.
pub(crate) fn composer_picker_accepts_edit(
    input_type: &str,
    prior_mode: Option<ComposerPickerMode>,
    prior_start: Option<usize>,
    start: usize,
    query_is_empty: bool,
) -> bool {
    let continues_token = prior_mode.is_some() && prior_start == Some(start);
    let inserts_trigger = query_is_empty
        && matches!(
            input_type,
            "insertText" | "insertCompositionText" | "insertFromComposition"
        );
    continues_token || inserts_trigger
}

/// Built-in slash commands offered by the `/` picker alongside skills and
/// workflows. Each maps to an existing surface: `/compact` is intercepted by
/// the session backend; `/fork` and `/btw` take the remaining text as a
/// payload (branch send / side-chat question); the rest run a UI action
/// directly. CLI-only commands like `/quit` do not belong here.
pub(crate) const SLASH_COMMANDS: &[&str] = &[
    "archive",
    "compact",
    "fork",
    "btw",
    "rewind",
    "review",
    "remember",
    "context",
    "plan",
    "permission",
    "save-as-skill",
    "skills",
    "files",
    "upload",
    "share",
    "trajectory",
];

/// Each command owns a distinct icon in the `/` picker so rows scan by
/// shape, not just by name. The fallback only covers names added to the
/// picker without a mapping here.
pub(crate) fn slash_command_icon(name: &str) -> &'static str {
    match name {
        "archive" => "archive",
        "compact" => "compact",
        "fork" => "fork",
        "btw" => "bubble",
        "rewind" => "undo",
        "review" => "review",
        "remember" => "memory",
        "context" => "gauge",
        "plan" => "plan",
        "permission" => "shield",
        "save-as-skill" => "save",
        "skills" => "book",
        "files" => "folder",
        "upload" => "upload",
        "share" => "share",
        "trajectory" => "timeline",
        _ => "terminal",
    }
}

/// Group-label key under which an item sits in the `/` menu. The menu is
/// layered commands → workflows → skills; other pickers leave their rows
/// ungrouped under one title.
pub(crate) fn picker_item_section(item: &ComposerPickerItem) -> Option<&'static str> {
    match item {
        ComposerPickerItem::Command { .. } => Some("composer.group_commands"),
        ComposerPickerItem::Workflow(_) => Some("composer.group_workflows"),
        ComposerPickerItem::Skill(_) => Some("composer.group_skills"),
        _ => None,
    }
}

/// `/` commands whose name matches the picker query (already lowercased).
pub(crate) fn slash_command_matches(query: &str) -> Vec<&'static str> {
    SLASH_COMMANDS
        .iter()
        .copied()
        .filter(|name| name.contains(query))
        .collect()
}

/// Split composer text into a built-in command name and its payload (the text
/// after the first whitespace run). Returns `None` unless the text is exactly
/// a known command, so ordinary messages and unknown `/words` pass through.
pub(crate) fn parse_slash_command(text: &str) -> Option<(&'static str, &str)> {
    let body = text.trim().strip_prefix('/')?;
    let (name, payload) = match body.find(char::is_whitespace) {
        Some(at) => (&body[..at], body[at..].trim_start()),
        None => (body, ""),
    };
    SLASH_COMMANDS
        .iter()
        .copied()
        .find(|candidate| *candidate == name)
        .map(|name| (name, payload))
}

/// True for commands the picker fills into the composer — `/compact` for
/// confirmation, `/fork`, `/btw`, and `/permission` because they need a
/// payload. Action commands (`/rewind`, `/review`, …) run immediately on
/// selection instead.
pub(crate) fn slash_command_fills_text(name: &str) -> bool {
    matches!(name, "compact" | "fork" | "btw" | "permission")
}

/// How far to move `scrollTop` so the row spanning `item_top..item_bottom`
/// lies fully inside `viewport_top..viewport_bottom`.
///
/// Zero means the row is already on screen: arrow keys should walk the
/// visible window, and the list should move only when the next row crosses
/// an edge. A row taller than the window aligns to its top so the title
/// stays in view.
pub(crate) fn picker_reveal_delta(
    viewport_top: f64,
    viewport_bottom: f64,
    item_top: f64,
    item_bottom: f64,
) -> f64 {
    const SLOP: f64 = 0.5;
    let above = viewport_top - item_top;
    if above > SLOP {
        return -above;
    }
    let below = item_bottom - viewport_bottom;
    if below > SLOP {
        let item_height = item_bottom - item_top;
        let view_height = viewport_bottom - viewport_top;
        if item_height > view_height + SLOP {
            return item_top - viewport_top;
        }
        return below;
    }
    0.0
}

fn picker_item(selector: &str, index: usize) -> Option<web_sys::Element> {
    let document = web_sys::window()?.document()?;
    let items = document.query_selector_all(selector).ok()?;
    items
        .item(index as u32)
        .map(|node| node.unchecked_into::<web_sys::Element>())
}

fn picker_label(element: &web_sys::Element) -> bool {
    element.class_name().split_whitespace().any(|name| {
        matches!(
            name,
            "project-search-label" | "action-palette-group" | "mention-group-label"
        )
    })
}

/// Scroll the picker list just enough to reveal `index`.
///
/// `scrollIntoView` aligns the row to the top and also scrolls ancestor
/// containers, so each arrow press pinned the next row to the first line.
pub(crate) fn scroll_picker_item(selector: &str, index: usize) {
    let Some(item) = picker_item(selector, index) else {
        return;
    };
    let Some(container) = item
        .closest(".project-search-results, .mention-menu")
        .ok()
        .flatten()
    else {
        return;
    };
    let view = container.get_bounding_client_rect();
    let rect = item.get_bounding_client_rect();
    let mut top = rect.top();
    let bottom = rect.bottom();
    if view.top() - top > 0.5 {
        if let Some(label) = item.previous_element_sibling() {
            if picker_label(&label) {
                top = label.get_bounding_client_rect().top().min(top);
            }
        }
    }
    let delta = picker_reveal_delta(view.top(), view.bottom(), top, bottom);
    if delta.abs() <= 0.5 {
        return;
    }
    let max_scroll = (container.scroll_height() - container.client_height()).max(0) as f64;
    let next = (container.scroll_top() as f64 + delta)
        .round()
        .clamp(0.0, max_scroll);
    container.set_scroll_top(next as i32);
}

pub(crate) fn reset_picker_scroll(selector: &str) {
    let Some(container) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector(selector).ok().flatten())
    else {
        return;
    };
    container.set_scroll_top(0);
}

#[cfg(test)]
mod picker_scroll_tests {
    use super::picker_reveal_delta;

    #[test]
    fn a_fully_visible_row_does_not_scroll() {
        assert_eq!(picker_reveal_delta(0.0, 400.0, 40.0, 80.0), 0.0);
        assert_eq!(picker_reveal_delta(0.0, 400.0, 0.0, 40.0), 0.0);
        assert_eq!(picker_reveal_delta(0.0, 400.0, 360.0, 400.0), 0.0);
        assert_eq!(picker_reveal_delta(80.0, 480.0, 80.0, 120.0), 0.0);
    }

    #[test]
    fn subpixel_slop_does_not_scroll() {
        assert_eq!(picker_reveal_delta(0.0, 400.0, -0.4, 40.0), 0.0);
        assert_eq!(picker_reveal_delta(0.0, 400.0, 360.0, 400.4), 0.0);
    }

    #[test]
    fn only_the_clipped_edge_scrolls() {
        assert_eq!(picker_reveal_delta(0.0, 400.0, 390.0, 420.0), 20.0);
        assert_eq!(picker_reveal_delta(100.0, 500.0, 85.0, 120.0), -15.0);
    }

    #[test]
    fn a_row_taller_than_the_window_aligns_to_its_top() {
        assert_eq!(picker_reveal_delta(0.0, 100.0, -10.0, 140.0), -10.0);
        assert_eq!(picker_reveal_delta(0.0, 100.0, 50.0, 200.0), 50.0);
    }
}

#[cfg(test)]
mod mention_tests {
    use super::{
        active_composer_trigger, composer_picker_accepts_edit, parse_slash_command,
        slash_command_fills_text, slash_command_icon, slash_command_matches, ComposerPickerMode,
        SLASH_COMMANDS,
    };

    #[test]
    fn slash_commands_filter_by_query() {
        assert!(slash_command_matches("").contains(&"compact"));
        assert_eq!(slash_command_matches("comp"), vec!["compact"]);
        assert_eq!(slash_command_matches("fo"), vec!["fork"]);
        assert_eq!(slash_command_matches("perm"), vec!["permission"]);
        assert!(slash_command_matches("s").contains(&"skills"));
        assert!(slash_command_matches("s").contains(&"save-as-skill"));
        assert_eq!(slash_command_matches("shar"), vec!["share"]);
        assert_eq!(slash_command_matches("traj"), vec!["trajectory"]);
        assert!(slash_command_matches("literature-review").is_empty());
    }

    #[test]
    fn every_slash_command_has_a_distinct_icon() {
        let icons: std::collections::HashSet<_> = SLASH_COMMANDS
            .iter()
            .map(|name| slash_command_icon(name))
            .collect();
        assert_eq!(icons.len(), SLASH_COMMANDS.len());
        // "terminal" is the fallback for unmapped names, not a real mapping.
        assert!(!icons.contains("terminal"));
    }

    #[test]
    fn parses_only_known_commands_with_payload() {
        assert_eq!(parse_slash_command("/compact"), Some(("compact", "")));
        assert_eq!(
            parse_slash_command("/fork try the other primer"),
            Some(("fork", "try the other primer"))
        );
        assert_eq!(
            parse_slash_command("  /btw   这个峰什么意思？"),
            Some(("btw", "这个峰什么意思？"))
        );
        assert_eq!(
            parse_slash_command("/permission full"),
            Some(("permission", "full"))
        );
        assert_eq!(parse_slash_command("/share"), Some(("share", "")));
        assert_eq!(parse_slash_command("/trajectory"), Some(("trajectory", "")));
        // Unknown commands, substrings, and embedded commands are not intercepted.
        assert_eq!(parse_slash_command("/compact2"), None);
        assert_eq!(parse_slash_command("/quit"), None);
        assert_eq!(parse_slash_command("send /fork now"), None);
        assert_eq!(parse_slash_command("/path/to/file"), None);
    }

    #[test]
    fn only_payload_commands_fill_the_composer() {
        assert!(slash_command_fills_text("compact"));
        assert!(slash_command_fills_text("fork"));
        assert!(slash_command_fills_text("btw"));
        assert!(slash_command_fills_text("permission"));
        assert!(!slash_command_fills_text("rewind"));
        assert!(!slash_command_fills_text("plan"));
        assert!(!slash_command_fills_text("upload"));
        assert!(!slash_command_fills_text("share"));
        assert!(!slash_command_fills_text("trajectory"));
    }

    #[test]
    fn detects_trigger_at_the_caret() {
        assert!(
            matches!(active_composer_trigger("look at @qc", 11), Some((8, 11, ComposerPickerMode::Artifact, q)) if q == "qc")
        );
        assert!(
            matches!(active_composer_trigger("#old", 4), Some((0, 4, ComposerPickerMode::Session, q)) if q == "old")
        );
        assert!(
            matches!(active_composer_trigger("/literature-review", 18), Some((0, 18, ComposerPickerMode::Skill, q)) if q == "literature-review")
        );
    }

    #[test]
    fn detects_trigger_before_existing_text_and_after_cjk() {
        let text = "比较#Current已有结果";
        let caret = "比较#Current".encode_utf16().count();
        assert!(
            matches!(active_composer_trigger(text, caret), Some((at, end, ComposerPickerMode::Session, q))
                if at == "比较".len() && end == "比较#Current".len() && q == "Current")
        );

        let text = "🧬@";
        assert!(
            matches!(active_composer_trigger(text, text.encode_utf16().count()), Some((4, 5, ComposerPickerMode::Artifact, q)) if q.is_empty())
        );
        assert_eq!(active_composer_trigger(text, 1), None);
    }

    #[test]
    fn ignores_literal_or_finished_tokens() {
        for text in ["no trigger", "email a@b.com", "https:/", "foo/", "1/2"] {
            assert_eq!(
                active_composer_trigger(text, text.encode_utf16().count()),
                None
            );
        }
        let text = "@qc then more";
        assert_eq!(
            active_composer_trigger(text, text.encode_utf16().count()),
            None
        );
    }

    #[test]
    fn opens_picker_for_direct_and_ime_trigger_input() {
        for input_type in [
            "insertText",
            "insertCompositionText",
            "insertFromComposition",
        ] {
            assert!(composer_picker_accepts_edit(
                input_type, None, None, 3, true
            ));
        }
        assert!(!composer_picker_accepts_edit(
            "deleteContentBackward",
            None,
            None,
            3,
            true
        ));
    }

    #[test]
    fn keeps_picker_open_while_editing_its_active_token() {
        assert!(composer_picker_accepts_edit(
            "deleteContentBackward",
            Some(ComposerPickerMode::Artifact),
            Some(3),
            3,
            false
        ));
        assert!(!composer_picker_accepts_edit(
            "insertText",
            Some(ComposerPickerMode::Artifact),
            Some(1),
            3,
            false
        ));
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
                location: None,
                ts: 0,
                project_id: None,
                project_name: None,
                session_id: None,
                session_title: None,
                size_bytes: None,
                origin: None,
                logical_path: None,
                source_discarded: false,
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
