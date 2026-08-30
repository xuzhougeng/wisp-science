use crate::app_support::{
    selection_targets_center_file, workspace_absolute_path, workspace_relative_path,
    SessionTransferMode,
};
use crate::dto::QuickAction;
use crate::i18n::{self, Locale};
use crate::text::{decode_href, is_runtime_code_selection, normalize_path};
use crate::window_capture_escape;
use leptos::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/src/context_menu.js")]
extern "C" {
    fn isDevMode() -> bool;
    #[wasm_bindgen(catch, js_name = copyImage)]
    async fn copy_image_js(src: &str) -> Result<JsValue, JsValue>;
}

#[derive(Clone)]
pub struct CtxItem {
    pub action: String,
    pub label: String,
    pub payload: String,
    pub children: Vec<CtxItem>,
}

#[derive(Clone)]
pub struct CtxMenu {
    pub x: f64,
    pub y: f64,
    pub items: Vec<CtxItem>,
}

#[derive(Clone, Copy, PartialEq)]
struct SubmenuAnchor {
    item_index: usize,
    left: f64,
    right: f64,
    top: f64,
}

fn submenu_anchor_from_event(item_index: usize, ev: &web_sys::MouseEvent) -> Option<SubmenuAnchor> {
    let target = ev.current_target()?.dyn_into::<web_sys::Element>().ok()?;
    let rect = target.get_bounding_client_rect();
    Some(SubmenuAnchor {
        item_index,
        left: rect.left(),
        right: rect.right(),
        top: rect.top(),
    })
}

pub fn dev_mode() -> bool {
    isDevMode()
}

pub async fn copy_image(src: &str) -> bool {
    copy_image_js(src).await.is_ok()
}

fn item(action: &str, label: String, payload: String) -> CtxItem {
    CtxItem {
        action: action.into(),
        label,
        payload,
        children: Vec::new(),
    }
}

pub(crate) fn motif_sequence_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".dna", ".fa", ".fasta", ".fna", ".ffn", ".faa", ".frn", ".gb", ".gbk", ".genbank", ".seq",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

fn submenu(label: String, children: Vec<CtxItem>) -> CtxItem {
    CtxItem {
        action: String::new(),
        label,
        payload: String::new(),
        children,
    }
}

fn workspace_paths_for_copy(path: &str, selected_paths: &[String]) -> Vec<String> {
    if !selected_paths.iter().any(|selected| selected == path) {
        return vec![path.to_string()];
    }
    let mut paths = selected_paths
        .iter()
        .filter(|selected| !selected.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn workspace_path_copy_items(
    path: &str,
    selected_paths: &[String],
    project_root: Option<&str>,
    locale: Locale,
) -> Vec<CtxItem> {
    let paths = workspace_paths_for_copy(path, selected_paths);
    let relative_paths = project_root
        .filter(|root| !root.is_empty())
        .and_then(|root| {
            paths
                .iter()
                .map(|path| workspace_relative_path(root, path))
                .collect::<Option<Vec<_>>>()
        })
        .unwrap_or_else(|| paths.iter().map(|path| path.replace('\\', "/")).collect());
    let mut items = Vec::with_capacity(2);
    if let Some(absolute_paths) = project_root
        .filter(|root| !root.is_empty())
        .and_then(|root| {
            paths
                .iter()
                .map(|path| workspace_absolute_path(root, path))
                .collect::<Option<Vec<_>>>()
        })
    {
        items.push(item(
            "copyAbsolutePath",
            i18n::t(locale, "files.copy_absolute_path"),
            absolute_paths.join("\n"),
        ));
    }
    items.push(item(
        "copyRelativePath",
        i18n::t(locale, "files.copy_relative_path"),
        relative_paths.join("\n"),
    ));
    items
}

pub fn remote_file_download_uri(context_id: &str, path: &str) -> Option<String> {
    let alias = context_id.strip_prefix("ssh:")?;
    if alias.is_empty()
        || !alias
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        || path.is_empty()
        || path.contains(['\0', '\n', '\r'])
    {
        return None;
    }
    let separator = if path.starts_with('/') { "" } else { "/" };
    Some(format!("ssh://{alias}{separator}{path}"))
}

fn event_target(ev: &web_sys::MouseEvent) -> Option<web_sys::Element> {
    ev.target()?.dyn_into::<web_sys::Element>().ok()
}

fn closest(el: &web_sys::Element, selector: &str) -> Option<web_sys::Element> {
    el.closest(selector).ok().flatten()
}

fn editable_text_entry(el: &web_sys::Element) -> Option<web_sys::Element> {
    let entry = closest(el, "textarea, input, [contenteditable=\"true\"]")?;
    if entry.tag_name().eq_ignore_ascii_case("input") {
        let input_type = entry
            .get_attribute("type")
            .unwrap_or_else(|| "text".into())
            .to_ascii_lowercase();
        if matches!(
            input_type.as_str(),
            "button"
                | "checkbox"
                | "color"
                | "file"
                | "hidden"
                | "image"
                | "radio"
                | "range"
                | "reset"
                | "submit"
        ) {
            return None;
        }
    }
    Some(entry)
}

pub(crate) fn uses_native_text_menu(ev: &web_sys::MouseEvent) -> bool {
    event_target(ev)
        .and_then(|target| editable_text_entry(&target))
        .is_some()
}

/// True when this mouseup landed on a control that already has a click action
/// (file/path link, artifact chip, attachment, copy button). A click that
/// happens to select the control's own label must not also raise the quote
/// popup on top of the preview or other action that click opens.
pub(crate) fn selection_popup_blocked(ev: &web_sys::MouseEvent) -> bool {
    event_element(ev)
        .and_then(|target| closest(&target, "a, button"))
        .is_some()
}

fn event_element(ev: &web_sys::MouseEvent) -> Option<web_sys::Element> {
    let target = ev.target()?;
    if let Ok(el) = target.clone().dyn_into::<web_sys::Element>() {
        return Some(el);
    }
    target.dyn_into::<web_sys::Node>().ok()?.parent_element()
}

pub(crate) fn selection_text() -> Option<String> {
    let win = web_sys::window()?;
    let sel = win.get_selection().ok().flatten()?;
    if sel.is_collapsed() {
        return None;
    }
    let text: String = sel.to_string().into();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn text_from_code_block(el: &web_sys::Element) -> Option<String> {
    for sel in [".tool-panel", "pre.md-code", "pre.rp-pre"] {
        let Some(block) = closest(el, sel) else {
            continue;
        };
        if let Ok(Some(code)) = block.query_selector("code") {
            let t = code.text_content().unwrap_or_default();
            if !t.trim().is_empty() {
                return Some(t);
            }
        }
        let t = block.text_content().unwrap_or_default();
        if !t.trim().is_empty() {
            return Some(t);
        }
    }
    None
}

#[derive(Clone, PartialEq)]
pub enum SessionAction {
    Open(String),
    AbandonExploration(String),
    Delete(String),
    DeleteBranch(String),
    MergeBranch(String),
    Rename {
        id: String,
        title: String,
    },
    Move {
        id: String,
        folder_id: Option<String>,
    },
    SetPinned {
        id: String,
        pinned: bool,
    },
    ReloadProjectRules(String),
    Transfer {
        id: String,
        mode: SessionTransferMode,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExplorationAction {
    Open(String),
    SelectAsMainline(String),
    ViewDiff(String),
    Discard(String),
}

#[derive(Clone, PartialEq)]
pub enum FolderAction {
    Rename { id: String, name: String },
    Delete(String),
}

#[derive(Clone, PartialEq)]
pub enum DemoAction {
    CopyToProject(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceEntryAction {
    Rename { path: String, is_dir: bool },
    Delete { path: String, is_dir: bool },
}

fn session_move_items(session_id: &str, locale: Locale) -> Vec<CtxItem> {
    let mut items = vec![item(
        "moveSession",
        i18n::t(locale, "ctx.move_to_ungrouped"),
        format!("{session_id}\u{1e}"),
    )];

    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return items;
    };
    let Ok(nodes) = doc.query_selector_all(".side-folder[data-folder-id]") else {
        return items;
    };
    for idx in 0..nodes.length() {
        let Some(node) = nodes.get(idx) else { continue };
        let Ok(el) = node.dyn_into::<web_sys::Element>() else {
            continue;
        };
        let id = el.get_attribute("data-folder-id").unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let name = el
            .get_attribute("data-folder-name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| i18n::t(locale, "folder.untitled"));
        items.push(item("moveSession", name, format!("{session_id}\u{1e}{id}")));
    }
    items
}

pub fn session_menu(
    x: f64,
    y: f64,
    session_id: &str,
    title: &str,
    pinned: bool,
    is_branch: bool,
    branch_merged: bool,
    has_branch_family: bool,
    has_exploration_round: bool,
    stale_prompt: bool,
    locale: Locale,
) -> CtxMenu {
    let mut items = vec![item(
        "copyTitle",
        i18n::t(locale, "ctx.copy_title"),
        title.to_string(),
    )];
    if !session_id.is_empty() {
        items.push(item(
            "openSession",
            i18n::t(locale, "ctx.open_session"),
            session_id.to_string(),
        ));
        if stale_prompt {
            items.push(item(
                "reloadProjectRules",
                i18n::t(locale, "ctx.reload_rules"),
                session_id.to_string(),
            ));
        }
        if is_branch && !branch_merged {
            items.push(item(
                "mergeSessionBranch",
                i18n::t(locale, "branch.merge"),
                session_id.to_string(),
            ));
        }
        if has_exploration_round && !is_branch {
            items.push(item(
                "abandonExplorationRound",
                i18n::t(locale, "exploration.abandon"),
                session_id.to_string(),
            ));
        }
        items.push(item(
            if pinned { "unpinSession" } else { "pinSession" },
            i18n::t(
                locale,
                if pinned {
                    "ctx.unpin_session"
                } else {
                    "ctx.pin_session"
                },
            ),
            session_id.to_string(),
        ));
        items.push(item(
            "renameSession",
            i18n::t(locale, "ctx.rename_session"),
            format!("{session_id}\u{1e}{title}"),
        ));
        items.push(submenu(
            i18n::t(locale, "ctx.move_to_prefix"),
            session_move_items(session_id, locale),
        ));
        items.push(item(
            "copySessionToProject",
            i18n::t(locale, "ctx.copy_to_project"),
            session_id.to_string(),
        ));
        items.push(item(
            "moveSessionToProject",
            i18n::t(locale, "ctx.move_to_project"),
            session_id.to_string(),
        ));
        items.push(item(
            "exportSession",
            i18n::t(locale, "ctx.export_session"),
            session_id.to_string(),
        ));
        items.push(item(
            "exportDebugRequest",
            i18n::t(locale, "ctx.export_debug_request"),
            session_id.to_string(),
        ));
        if is_branch || (!has_branch_family && !has_exploration_round) {
            items.push(item(
                if is_branch {
                    "deleteSessionBranch"
                } else {
                    "deleteSession"
                },
                i18n::t(
                    locale,
                    if is_branch {
                        "branch.delete"
                    } else {
                        "ctx.delete_session"
                    },
                ),
                session_id.to_string(),
            ));
        }
    }
    CtxMenu { x, y, items }
}

pub fn demo_menu(x: f64, y: f64, demo_id: &str, title: &str, locale: Locale) -> CtxMenu {
    CtxMenu {
        x,
        y,
        items: vec![
            item(
                "copyTitle",
                i18n::t(locale, "ctx.copy_title"),
                title.to_string(),
            ),
            item(
                "copyDemoToProject",
                i18n::t(locale, "ctx.copy_demo_to_project"),
                demo_id.to_string(),
            ),
        ],
    }
}

pub fn exploration_menu(
    x: f64,
    y: f64,
    exploration_id: &str,
    status: &str,
    locale: Locale,
) -> CtxMenu {
    let mut items = vec![item(
        "openExploration",
        i18n::t(locale, "exploration.open"),
        exploration_id.to_string(),
    )];
    if status == "active" {
        items.push(item(
            "selectExplorationAsMainline",
            i18n::t(locale, "exploration.select_as_mainline"),
            exploration_id.to_string(),
        ));
    }
    items.push(item(
        "viewExplorationDiff",
        i18n::t(locale, "exploration.view_diff"),
        exploration_id.to_string(),
    ));
    if matches!(status, "active" | "failed") {
        items.push(item(
            "discardExploration",
            i18n::t(locale, "exploration.discard"),
            exploration_id.to_string(),
        ));
    }
    CtxMenu { x, y, items }
}

pub fn folder_menu(x: f64, y: f64, id: &str, name: &str, locale: Locale) -> CtxMenu {
    let mut items = Vec::new();
    if !id.is_empty() {
        items.push(item(
            "renameFolder",
            i18n::t(locale, "ctx.rename_folder"),
            format!("{id}\u{1e}{name}"),
        ));
        items.push(item(
            "deleteFolder",
            i18n::t(locale, "ctx.delete_folder"),
            id.to_string(),
        ));
    }
    CtxMenu { x, y, items }
}

pub fn build(
    ev: &web_sys::MouseEvent,
    locale: Locale,
    _can_export: bool,
    center_file: Option<&str>,
    quick_actions: &[QuickAction],
    project_root: Option<&str>,
    selected_workspace_paths: &[String],
) -> Option<CtxMenu> {
    let target = event_target(ev)?;
    let x = ev.client_x() as f64;
    let y = ev.client_y() as f64;

    if let Some(tab) = closest(&target, ".center-tab[data-center-path]") {
        let path = tab.get_attribute("data-center-path").unwrap_or_default();
        if !path.is_empty() {
            return Some(CtxMenu {
                x,
                y,
                items: vec![
                    item(
                        "closeCenterCurrent",
                        i18n::t(locale, "center.close_current"),
                        path.clone(),
                    ),
                    item(
                        "closeCenterRight",
                        i18n::t(locale, "center.close_right"),
                        path.clone(),
                    ),
                    item("closeCenterAll", i18n::t(locale, "center.close_all"), path),
                ],
            });
        }
    }

    // Assistant Markdown turns project-local inline-code paths into these
    // anchors. Give them a file-focused menu before selected-text handling so
    // right-clicking a path never falls through to the whole-message menu (or
    // an unrelated selection elsewhere in the transcript).
    if let Some(link) = closest(&target, "a.workspace-path-link[href]") {
        let href = link.get_attribute("href").unwrap_or_default();
        let path = normalize_path(&decode_href(&href));
        if let Some(path) = workspace_relative_path(project_root.unwrap_or_default(), &path)
            .filter(|path| !path.is_empty())
        {
            let mut items = vec![item(
                "openWorkspaceFileCenter",
                i18n::t(locale, "center.open_file"),
                path.clone(),
            )];
            items.extend(workspace_path_copy_items(&path, &[], project_root, locale));
            items.push(item(
                "revealInFileManager",
                i18n::t(locale, "ctx.reveal_in_manager"),
                path,
            ));
            return Some(CtxMenu { x, y, items });
        }
    }

    let text_entry = editable_text_entry(&target);
    // A stray text selection (e.g. an accidentally selected file name) must not
    // hijack the context menu of a structural row — file rows, artifact tiles,
    // sessions and folders have their own menus below and should win when
    // right-clicked, regardless of what happens to be selected on the page.
    let on_structural_row =
        closest(&target, ".fb-row, .rp-tile, .side-item.ses, .side-folder").is_some();
    if text_entry.is_none() && !on_structural_row {
        if let Some(text) = selection_text() {
            // Mirror the selection popup so right-click offers the same
            // destinations instead of stacking overlays. R/Python source
            // keeps quote/explain and skips literature/review actions.
            let source = closest(&target, "[data-file-path]")
                .and_then(|el| el.get_attribute("data-file-path"))
                .filter(|source| !source.is_empty());
            let quote_label = if selection_targets_center_file(source.as_deref(), center_file) {
                i18n::t(locale, "selection.ask_ai")
            } else {
                i18n::t(locale, "selection.add_to_chat")
            };
            let quote_payload = format!("{}\u{1e}{text}", source.as_deref().unwrap_or_default());
            let mut items = vec![
                item("copy", i18n::t(locale, "ctx.copy"), text.clone()),
                item("quoteSelection", quote_label, quote_payload.clone()),
                item(
                    "quoteSelectionSideChat",
                    i18n::t(locale, "selection.quote_side_chat"),
                    quote_payload,
                ),
            ];
            if !is_runtime_code_selection(source.as_deref()) {
                items.extend(
                    quick_actions
                        .iter()
                        .filter(|action| action.enabled && action.context == "selection")
                        .map(|action| {
                            item(
                                "runQuickAction",
                                crate::app_support::quick_action_label(locale, action),
                                format!(
                                    "{}\u{1e}{}\u{1e}{text}",
                                    action.id,
                                    source.as_deref().unwrap_or_default()
                                ),
                            )
                        }),
                );
            }
            items.push(item(
                "explainSelection",
                i18n::t(locale, "selection.explain"),
                text,
            ));
            return Some(CtxMenu { x, y, items });
        }
    }

    if let Some(code) = text_from_code_block(&target) {
        return Some(CtxMenu {
            x,
            y,
            items: vec![item("copyCode", i18n::t(locale, "ctx.copy_code"), code)],
        });
    }

    if let Some(ses) = closest(&target, ".side-item.ses, .message-branch-link") {
        let title = ses.get_attribute("data-session-title").unwrap_or_default();
        let id = ses.get_attribute("data-session-id").unwrap_or_default();
        let pinned = ses.get_attribute("data-session-pinned").as_deref() == Some("true");
        let is_branch = ses.get_attribute("data-session-branch").as_deref() == Some("true")
            || ses.class_list().contains("message-branch-link");
        let branch_merged = ses.get_attribute("data-branch-merged").as_deref() == Some("true");
        let has_branch_family = ses.get_attribute("data-session-family").as_deref() == Some("true");
        let has_exploration_round =
            ses.get_attribute("data-exploration-round").as_deref() == Some("true");
        let stale_prompt = ses.get_attribute("data-session-stale").as_deref() == Some("true");
        return Some(session_menu(
            x,
            y,
            &id,
            &title,
            pinned,
            is_branch,
            branch_merged,
            has_branch_family,
            has_exploration_round,
            stale_prompt,
            locale,
        ));
    }

    if let Some(folder) = closest(&target, ".side-folder") {
        let name = folder.get_attribute("data-folder-name").unwrap_or_default();
        let id = folder.get_attribute("data-folder-id").unwrap_or_default();
        if !id.is_empty() {
            return Some(folder_menu(x, y, &id, &name, locale));
        }
    }

    if let Some(image) = closest(&target, ".rp-img") {
        let src = image.get_attribute("src").unwrap_or_default();
        if !src.is_empty() {
            return Some(CtxMenu {
                x,
                y,
                items: vec![item("copyImage", i18n::t(locale, "ctx.copy_image"), src)],
            });
        }
    }

    if let Some(tile) = closest(&target, ".rp-tile") {
        let name = tile.get_attribute("data-artifact-name").unwrap_or_default();
        let path = tile.get_attribute("data-artifact-path").unwrap_or_default();
        let location = tile
            .get_attribute("data-artifact-location")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| path.clone());
        if !name.is_empty() {
            let mut items = vec![item("copyName", i18n::t(locale, "ctx.copy_name"), name)];
            if !path.is_empty() {
                items.insert(
                    0,
                    item(
                        "openWorkspaceFileCenter",
                        i18n::t(locale, "center.open_file"),
                        path.clone(),
                    ),
                );
                items.insert(
                    1,
                    item(
                        "attachWorkspaceFile",
                        i18n::t(locale, "ctx.attach_file"),
                        location.clone(),
                    ),
                );
                if motif_sequence_path(&path) {
                    items.insert(
                        2,
                        item(
                            "addWorkspaceFileToMotif",
                            i18n::t(locale, "ctx.add_to_motif"),
                            location.clone(),
                        ),
                    );
                }
                items.push(item(
                    "downloadFile",
                    i18n::t(locale, "artifact.download"),
                    path.clone(),
                ));
                items.push(item(
                    "revealInFileManager",
                    i18n::t(locale, "ctx.reveal_in_manager"),
                    location,
                ));
            }
            return Some(CtxMenu { x, y, items });
        }
    }

    if let Some(file) = closest(
        &target,
        ".fb-row.remote-file[data-remote-path][data-remote-context]",
    ) {
        let path = file.get_attribute("data-remote-path").unwrap_or_default();
        let context_id = file
            .get_attribute("data-remote-context")
            .unwrap_or_default();
        if let Some(uri) = remote_file_download_uri(&context_id, &path) {
            return Some(CtxMenu {
                x,
                y,
                items: vec![item(
                    "downloadFile",
                    i18n::t(locale, "artifact.download"),
                    uri,
                )],
            });
        }
    }

    if let Some(directory) = closest(&target, ".fb-row.dir[data-workspace-path]") {
        let path = directory
            .get_attribute("data-workspace-path")
            .unwrap_or_default();
        if !path.is_empty() {
            let mut items = vec![item(
                "attachWorkspaceDirectory",
                i18n::t(locale, "ctx.attach_directory"),
                path.clone(),
            )];
            items.extend(workspace_path_copy_items(
                &path,
                selected_workspace_paths,
                project_root,
                locale,
            ));
            items.extend([
                item(
                    "renameWorkspaceDirectory",
                    i18n::t(locale, "files.rename_directory"),
                    path.clone(),
                ),
                item(
                    "deleteWorkspaceDirectory",
                    i18n::t(locale, "files.delete_directory"),
                    path,
                ),
            ]);
            return Some(CtxMenu { x, y, items });
        }
    }

    if let Some(file) = closest(&target, ".fb-row[data-workspace-path]") {
        let path = file
            .get_attribute("data-workspace-path")
            .unwrap_or_default();
        if !path.is_empty() {
            let file_name = path.rsplit('/').next().unwrap_or(path.as_str()).to_string();
            let mut items = vec![item(
                "copyName",
                i18n::t(locale, "ctx.copy_name"),
                file_name,
            )];
            items.extend(workspace_path_copy_items(
                &path,
                selected_workspace_paths,
                project_root,
                locale,
            ));
            items.extend([
                item(
                    "openWorkspaceFileCenter",
                    i18n::t(locale, "center.open_file"),
                    path.clone(),
                ),
                item(
                    "attachWorkspaceFile",
                    i18n::t(locale, "ctx.attach_file"),
                    path.clone(),
                ),
            ]);
            if motif_sequence_path(&path) {
                items.push(item(
                    "addWorkspaceFileToMotif",
                    i18n::t(locale, "ctx.add_to_motif"),
                    path.clone(),
                ));
            }
            items.extend([
                item(
                    "registerWorkspaceArtifact",
                    i18n::t(locale, "ctx.register_artifact"),
                    path.clone(),
                ),
                item(
                    "downloadFile",
                    i18n::t(locale, "artifact.download"),
                    path.clone(),
                ),
                item(
                    "revealInFileManager",
                    i18n::t(locale, "ctx.reveal_in_manager"),
                    path.clone(),
                ),
                item(
                    "renameWorkspaceFile",
                    i18n::t(locale, "files.rename_file"),
                    path.clone(),
                ),
                item(
                    "deleteWorkspaceFile",
                    i18n::t(locale, "files.delete_file"),
                    path,
                ),
            ]);
            return Some(CtxMenu { x, y, items });
        }
    }

    if let Some(message) = closest(&target, "[data-branch-ui-index]") {
        let ui_index = message
            .get_attribute("data-branch-ui-index")
            .unwrap_or_default();
        if !ui_index.is_empty() {
            let mut items = vec![item(
                "branchMessage",
                i18n::t(locale, "msg.branch_to_new_chat"),
                ui_index,
            )];
            if let Some(body) = closest(&target, ".msg .body") {
                let text = body.text_content().unwrap_or_default();
                if !text.trim().is_empty() {
                    items.push(item(
                        "copyMessage",
                        i18n::t(locale, "ctx.copy_message"),
                        text,
                    ));
                }
            }
            return Some(CtxMenu { x, y, items });
        }
    }

    if let Some(body) = closest(&target, ".msg .body") {
        let text = body.text_content().unwrap_or_default();
        if !text.trim().is_empty() {
            return Some(CtxMenu {
                x,
                y,
                items: vec![item(
                    "copyMessage",
                    i18n::t(locale, "ctx.copy_message"),
                    text,
                )],
            });
        }
    }

    None
}

pub fn run_action(action: &str, payload: &str, copy: impl Fn(String)) {
    match action {
        "copy" | "copyCode" | "copyTitle" | "copyName" | "copyMessage" | "copyAbsolutePath"
        | "copyRelativePath"
            if !payload.is_empty() =>
        {
            copy(payload.to_string());
        }
        _ => {}
    }
}

#[cfg(test)]
mod remote_file_tests {
    use super::{remote_file_download_uri, workspace_entry_action, WorkspaceEntryAction};

    #[test]
    fn builds_download_uri_for_absolute_and_home_paths() {
        assert_eq!(
            remote_file_download_uri("ssh:gpu-server", "/home/research/results.csv"),
            Some("ssh://gpu-server/home/research/results.csv".into())
        );
        assert_eq!(
            remote_file_download_uri("ssh:gpu-server", "~/results.csv"),
            Some("ssh://gpu-server/~/results.csv".into())
        );
        assert_eq!(remote_file_download_uri("local", "/tmp/results.csv"), None);
        assert_eq!(
            remote_file_download_uri("ssh:bad/alias", "/tmp/results.csv"),
            None
        );
    }

    #[test]
    fn parses_workspace_entry_actions_with_entry_kind() {
        assert_eq!(
            workspace_entry_action("renameWorkspaceFile", "notes.md"),
            Some(WorkspaceEntryAction::Rename {
                path: "notes.md".into(),
                is_dir: false,
            })
        );
        assert_eq!(
            workspace_entry_action("deleteWorkspaceDirectory", "results/run-1"),
            Some(WorkspaceEntryAction::Delete {
                path: "results/run-1".into(),
                is_dir: true,
            })
        );
        assert_eq!(workspace_entry_action("renameWorkspaceFile", ""), None);
    }
}

#[cfg(test)]
mod session_branch_action_tests {
    use super::{session_action, SessionAction};

    #[test]
    fn parses_branch_specific_actions() {
        assert!(matches!(
            session_action("mergeSessionBranch", "branch-1"),
            Some(SessionAction::MergeBranch(id)) if id == "branch-1"
        ));
        assert!(matches!(
            session_action("deleteSessionBranch", "branch-1"),
            Some(SessionAction::DeleteBranch(id)) if id == "branch-1"
        ));
    }

    #[test]
    fn parses_demo_copy_action() {
        assert!(matches!(
            super::demo_action("copyDemoToProject", "manifest_memory_01_long_context"),
            Some(super::DemoAction::CopyToProject(id)) if id == "manifest_memory_01_long_context"
        ));
        assert!(super::demo_action("copyDemoToProject", "").is_none());
    }
}

#[cfg(test)]
mod exploration_action_tests {
    use super::{exploration_action, ExplorationAction};

    #[test]
    fn parses_exploration_context_actions() {
        assert_eq!(
            exploration_action("selectExplorationAsMainline", "exploration-1"),
            Some(ExplorationAction::SelectAsMainline("exploration-1".into()))
        );
        assert_eq!(
            exploration_action("discardExploration", "exploration-1"),
            Some(ExplorationAction::Discard("exploration-1".into()))
        );
        assert_eq!(exploration_action("discardExploration", ""), None);
    }
}

pub fn exploration_action(action: &str, payload: &str) -> Option<ExplorationAction> {
    if payload.is_empty() {
        return None;
    }
    let id = payload.to_string();
    match action {
        "openExploration" => Some(ExplorationAction::Open(id)),
        "selectExplorationAsMainline" => Some(ExplorationAction::SelectAsMainline(id)),
        "viewExplorationDiff" => Some(ExplorationAction::ViewDiff(id)),
        "discardExploration" => Some(ExplorationAction::Discard(id)),
        _ => None,
    }
}

pub fn session_action(action: &str, payload: &str) -> Option<SessionAction> {
    match action {
        "openSession" if !payload.is_empty() => Some(SessionAction::Open(payload.to_string())),
        "abandonExplorationRound" if !payload.is_empty() => {
            Some(SessionAction::AbandonExploration(payload.to_string()))
        }
        "deleteSession" if !payload.is_empty() => Some(SessionAction::Delete(payload.to_string())),
        "deleteSessionBranch" if !payload.is_empty() => {
            Some(SessionAction::DeleteBranch(payload.to_string()))
        }
        "mergeSessionBranch" if !payload.is_empty() => {
            Some(SessionAction::MergeBranch(payload.to_string()))
        }
        "reloadProjectRules" if !payload.is_empty() => {
            Some(SessionAction::ReloadProjectRules(payload.to_string()))
        }
        "renameSession" if !payload.is_empty() => {
            let (id, title) = payload.split_once('\u{1e}')?;
            Some(SessionAction::Rename {
                id: id.to_string(),
                title: title.to_string(),
            })
        }
        "moveSession" if !payload.is_empty() => {
            let (id, folder_id) = payload.split_once('\u{1e}')?;
            Some(SessionAction::Move {
                id: id.to_string(),
                folder_id: (!folder_id.is_empty()).then(|| folder_id.to_string()),
            })
        }
        "pinSession" if !payload.is_empty() => Some(SessionAction::SetPinned {
            id: payload.to_string(),
            pinned: true,
        }),
        "unpinSession" if !payload.is_empty() => Some(SessionAction::SetPinned {
            id: payload.to_string(),
            pinned: false,
        }),
        "copySessionToProject" if !payload.is_empty() => Some(SessionAction::Transfer {
            id: payload.to_string(),
            mode: SessionTransferMode::Copy,
        }),
        "moveSessionToProject" if !payload.is_empty() => Some(SessionAction::Transfer {
            id: payload.to_string(),
            mode: SessionTransferMode::Move,
        }),
        _ => None,
    }
}

pub fn demo_action(action: &str, payload: &str) -> Option<DemoAction> {
    match action {
        "copyDemoToProject" if !payload.is_empty() => {
            Some(DemoAction::CopyToProject(payload.to_string()))
        }
        _ => None,
    }
}

pub fn folder_action(action: &str, payload: &str) -> Option<FolderAction> {
    match action {
        "renameFolder" if !payload.is_empty() => {
            let (id, name) = payload.split_once('\u{1e}')?;
            Some(FolderAction::Rename {
                id: id.to_string(),
                name: name.to_string(),
            })
        }
        "deleteFolder" if !payload.is_empty() => Some(FolderAction::Delete(payload.to_string())),
        _ => None,
    }
}

pub fn workspace_entry_action(action: &str, payload: &str) -> Option<WorkspaceEntryAction> {
    if payload.is_empty() {
        return None;
    }
    match action {
        "renameWorkspaceFile" => Some(WorkspaceEntryAction::Rename {
            path: payload.to_string(),
            is_dir: false,
        }),
        "renameWorkspaceDirectory" => Some(WorkspaceEntryAction::Rename {
            path: payload.to_string(),
            is_dir: true,
        }),
        "deleteWorkspaceFile" => Some(WorkspaceEntryAction::Delete {
            path: payload.to_string(),
            is_dir: false,
        }),
        "deleteWorkspaceDirectory" => Some(WorkspaceEntryAction::Delete {
            path: payload.to_string(),
            is_dir: true,
        }),
        _ => None,
    }
}

fn viewport_size() -> Option<(f64, f64)> {
    let window = web_sys::window()?;
    Some((
        window.inner_width().ok()?.as_f64()?,
        window.inner_height().ok()?.as_f64()?,
    ))
}

fn clamp_to_viewport(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    viewport_width: f64,
    viewport_height: f64,
) -> (f64, f64) {
    (
        x.max(8.0).min((viewport_width - width - 8.0).max(8.0)),
        y.max(8.0).min((viewport_height - height - 8.0).max(8.0)),
    )
}

#[component]
pub fn ContextMenuPortal(
    menu: ReadSignal<Option<CtxMenu>>,
    set_menu: WriteSignal<Option<CtxMenu>>,
    on_pick: Callback<(String, String)>,
) -> impl IntoView {
    let submenu_anchor = create_rw_signal(None::<SubmenuAnchor>);
    let menu_el = create_node_ref::<html::Div>();
    let submenu_el = create_node_ref::<html::Div>();
    // The initial position is based on a 38px-per-item estimate, but real item
    // heights vary with fonts, locales, and label wrapping — so after render we
    // measure the true size and re-clamp the menu into the viewport (issue #650).
    // Each fix is tagged with the position it was computed for so a stale fix is
    // never applied to a newly opened menu.
    let menu_fix = create_rw_signal(None::<(f64, f64, f64, f64)>);
    let submenu_fix = create_rw_signal(None::<(SubmenuAnchor, f64, f64)>);
    window_capture_escape(move || {
        if submenu_anchor.get_untracked().is_none() {
            return false;
        }
        submenu_anchor.set(None);
        true
    });

    create_effect(move |_| {
        let Some(m) = menu.get() else {
            menu_fix.set(None);
            return;
        };
        request_animation_frame(move || {
            let Some(el) = menu_el.get() else { return };
            let Some((viewport_width, viewport_height)) = viewport_size() else {
                return;
            };
            let (left, top) = clamp_to_viewport(
                m.x,
                m.y,
                f64::from(el.offset_width()),
                f64::from(el.offset_height()),
                viewport_width,
                viewport_height,
            );
            menu_fix.set(Some((m.x, m.y, left, top)));
        });
    });

    create_effect(move |_| {
        if menu.get().is_none() {
            submenu_fix.set(None);
            return;
        }
        let Some(anchor) = submenu_anchor.get() else {
            submenu_fix.set(None);
            return;
        };
        request_animation_frame(move || {
            let Some(el) = submenu_el.get() else { return };
            let Some((viewport_width, viewport_height)) = viewport_size() else {
                return;
            };
            let width = f64::from(el.offset_width());
            let height = f64::from(el.offset_height());
            let left = if anchor.right + width <= viewport_width - 8.0 {
                anchor.right
            } else {
                (anchor.left - width).max(8.0)
            };
            let (_, top) = clamp_to_viewport(
                left,
                anchor.top,
                width,
                height,
                viewport_width,
                viewport_height,
            );
            submenu_fix.set(Some((anchor, left, top)));
        });
    });

    view! {
        {move || {
            let m = menu.get()?;
            if m.items.is_empty() {
                return None;
            }
            let items = m.items.clone();
            let item_count = items.len() as f64;
            let (viewport_width, viewport_height) = viewport_size()
                .unwrap_or((m.x + 280.0, m.y + item_count * 38.0 + 12.0));
            let estimated_width = 280.0_f64.min((viewport_width - 16.0).max(168.0));
            let estimated_height = (item_count * 38.0 + 12.0).min((viewport_height - 16.0).max(50.0));
            let (estimated_left, estimated_top) = clamp_to_viewport(
                m.x,
                m.y,
                estimated_width,
                estimated_height,
                viewport_width,
                viewport_height,
            );
            let (left, top) = match menu_fix.get() {
                Some((x, y, left, top)) if x == m.x && y == m.y => (left, top),
                _ => (estimated_left, estimated_top),
            };
            Some(view! {
                <div
                    class="ctx-backdrop"
                    on:mouseenter=move |_| submenu_anchor.set(None)
                    on:click=move |_| {
                        submenu_anchor.set(None);
                        set_menu.set(None);
                    }
                ></div>
                <div
                    class="ctx-menu"
                    role="menu"
                    node_ref=menu_el
                    style=format!("left:{left}px;top:{top}px")
                    on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
                >
                    {items.into_iter().enumerate().map(|(item_index, it)| {
                        if !it.children.is_empty() {
                            let label = it.label;
                            return view! {
                                <button
                                    type="button"
                                    class="ctx-item ctx-submenu-trigger"
                                    aria-haspopup="menu"
                                    aria-expanded=move || submenu_anchor.get()
                                        .map(|anchor| anchor.item_index == item_index)
                                        .unwrap_or(false)
                                    on:mouseenter=move |ev: web_sys::MouseEvent| {
                                        submenu_anchor.set(submenu_anchor_from_event(item_index, &ev));
                                    }
                                    on:click=move |ev: web_sys::MouseEvent| {
                                        submenu_anchor.set(submenu_anchor_from_event(item_index, &ev));
                                    }
                                >
                                    <span class="ctx-item-label">{label}</span>
                                    <span class="ctx-submenu-chevron" aria-hidden="true">"›"</span>
                                </button>
                            }.into_view();
                        }
                        let action = it.action.clone();
                        let payload = it.payload.clone();
                        let danger = matches!(
                            action.as_str(),
                            "deleteSession"
                                | "deleteSessionBranch"
                                | "deleteFolder"
                                | "deleteWorkspaceFile"
                                | "deleteWorkspaceDirectory"
                                | "discardExploration"
                        );
                        view! {
                            <button
                                type="button"
                                class="ctx-item"
                                class:danger=danger
                                on:mouseenter=move |_| submenu_anchor.set(None)
                                on:click=move |_| {
                                    on_pick.call((action.clone(), payload.clone()));
                                    submenu_anchor.set(None);
                                    set_menu.set(None);
                                }
                            >{it.label}</button>
                        }.into_view()
                    }).collect_view()}
                </div>
                {move || {
                    let anchor = submenu_anchor.get()?;
                    let m = menu.get()?;
                    let parent = m.items.get(anchor.item_index)?;
                    if parent.children.is_empty() {
                        return None;
                    }
                    let items = parent.children.clone();
                    let item_count = items.len() as f64;
                    let (viewport_width, viewport_height) = viewport_size()
                        .unwrap_or((anchor.right + 280.0, anchor.top + item_count * 38.0 + 12.0));
                    let estimated_width = 280.0_f64.min((viewport_width - 16.0).max(168.0));
                    let estimated_height = (item_count * 38.0 + 12.0).min((viewport_height - 16.0).max(50.0));
                    let left = if anchor.right + estimated_width <= viewport_width - 8.0 {
                        anchor.right
                    } else {
                        (anchor.left - estimated_width).max(8.0)
                    };
                    let top = anchor.top.max(8.0).min((viewport_height - estimated_height - 8.0).max(8.0));
                    let (left, top) = match submenu_fix.get() {
                        Some((a, left, top)) if a == anchor => (left, top),
                        _ => (left, top),
                    };
                    Some(view! {
                        <div
                            class="ctx-menu ctx-submenu-menu"
                            role="menu"
                            aria-label=parent.label.clone()
                            node_ref=submenu_el
                            style=format!("left:{left}px;top:{top}px;width:{estimated_width}px")
                            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
                        >
                            {items.into_iter().map(|it| {
                                let action = it.action.clone();
                                let payload = it.payload.clone();
                                view! {
                                    <button
                                        type="button"
                                        class="ctx-item"
                                        on:click=move |_| {
                                            on_pick.call((action.clone(), payload.clone()));
                                            submenu_anchor.set(None);
                                            set_menu.set(None);
                                        }
                                    >{it.label}</button>
                                }
                            }).collect_view()}
                        </div>
                    }.into_view())
                }}
            }.into_view())
        }}
    }
}

#[cfg(test)]
mod tests {
    use super::motif_sequence_path;

    #[test]
    fn motif_menu_accepts_sequence_files_but_not_unrelated_documents() {
        for path in ["vector.dna", "refs/insert.FASTA", "plasmid.gbk", "read.seq"] {
            assert!(motif_sequence_path(path), "{path}");
        }
        for path in ["notes.md", "results.csv", "figure.png", "archive.zip"] {
            assert!(!motif_sequence_path(path), "{path}");
        }
    }
}
