use crate::app_support::SessionTransferMode;
use crate::i18n::{self, Locale};
use leptos::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/src/context_menu.js")]
extern "C" {
    fn isDevMode() -> bool;
    #[wasm_bindgen(js_name = captureTextEntryTarget)]
    fn capture_text_entry_target(target: web_sys::Element);
    #[wasm_bindgen(js_name = textEntryCommand)]
    fn text_entry_command(kind: &str);
    #[wasm_bindgen(catch, js_name = copyImage)]
    async fn copy_image_js(src: &str) -> Result<JsValue, JsValue>;
}

#[derive(Clone)]
pub struct CtxItem {
    pub action: String,
    pub label: String,
    pub payload: String,
}

#[derive(Clone)]
pub struct CtxMenu {
    pub x: f64,
    pub y: f64,
    pub items: Vec<CtxItem>,
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
    }
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
    for sel in [".code-block", ".tool-panel", "pre.md-code", "pre.rp-pre"] {
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
    Delete(String),
    Rename {
        id: String,
        title: String,
    },
    Move {
        id: String,
        folder_id: Option<String>,
    },
    Transfer {
        id: String,
        mode: SessionTransferMode,
    },
}

#[derive(Clone, PartialEq)]
pub enum FolderAction {
    Rename { id: String, name: String },
    Delete(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceEntryAction {
    Rename { path: String, is_dir: bool },
    Delete { path: String, is_dir: bool },
}

fn session_move_items(session_id: &str, locale: Locale) -> Vec<CtxItem> {
    let prefix = i18n::t(locale, "ctx.move_to_prefix");
    let mut items = vec![item(
        "moveSession",
        format!("{}: {}", prefix, i18n::t(locale, "ctx.move_to_ungrouped")),
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
        items.push(item(
            "moveSession",
            format!("{prefix}: {name}"),
            format!("{session_id}\u{1e}{id}"),
        ));
    }
    items
}

pub fn session_menu(x: f64, y: f64, session_id: &str, title: &str, locale: Locale) -> CtxMenu {
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
        items.push(item(
            "renameSession",
            i18n::t(locale, "ctx.rename_session"),
            format!("{session_id}\u{1e}{title}"),
        ));
        items.extend(session_move_items(session_id, locale));
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
        items.push(item(
            "deleteSession",
            i18n::t(locale, "ctx.delete_session"),
            session_id.to_string(),
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

    let text_entry = editable_text_entry(&target);
    // A stray text selection (e.g. an accidentally selected file name) must not
    // hijack the context menu of a structural row — file rows, artifact tiles,
    // sessions and folders have their own menus below and should win when
    // right-clicked, regardless of what happens to be selected on the page.
    let on_structural_row =
        closest(&target, ".fb-row, .rp-tile, .side-item.ses, .side-folder").is_some();
    if text_entry.is_none() && !on_structural_row {
        if let Some(text) = selection_text() {
            // Mirror the selection popup's quote/explain actions so right-click
            // offers everything in one menu instead of stacking popups.
            let source = closest(&target, "[data-file-path]")
                .and_then(|el| el.get_attribute("data-file-path"))
                .unwrap_or_default();
            let quote_label = if source.as_str() == center_file.unwrap_or_default() {
                i18n::t(locale, "selection.ask_ai")
            } else {
                i18n::t(locale, "selection.add_to_chat")
            };
            return Some(CtxMenu {
                x,
                y,
                items: vec![
                    item("copy", i18n::t(locale, "ctx.copy"), text.clone()),
                    item(
                        "quoteSelection",
                        quote_label,
                        format!("{source}\u{1e}{text}"),
                    ),
                    item(
                        "explainSelection",
                        i18n::t(locale, "selection.explain"),
                        text,
                    ),
                ],
            });
        }
    }

    if let Some(entry) = text_entry {
        capture_text_entry_target(entry);
        return Some(CtxMenu {
            x,
            y,
            items: vec![
                item("cut", i18n::t(locale, "ctx.cut"), String::new()),
                item("copy", i18n::t(locale, "ctx.copy"), String::new()),
                item("paste", i18n::t(locale, "ctx.paste"), String::new()),
                item(
                    "selectAll",
                    i18n::t(locale, "ctx.select_all"),
                    String::new(),
                ),
            ],
        });
    }

    if let Some(code) = text_from_code_block(&target) {
        return Some(CtxMenu {
            x,
            y,
            items: vec![item("copyCode", i18n::t(locale, "ctx.copy_code"), code)],
        });
    }

    if let Some(ses) = closest(&target, ".side-item.ses") {
        let title = ses.get_attribute("data-session-title").unwrap_or_default();
        let id = ses.get_attribute("data-session-id").unwrap_or_default();
        return Some(session_menu(x, y, &id, &title, locale));
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
                items.push(item(
                    "downloadFile",
                    i18n::t(locale, "artifact.download"),
                    path.clone(),
                ));
                items.push(item(
                    "revealInFileManager",
                    i18n::t(locale, "ctx.reveal_in_manager"),
                    path,
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
            return Some(CtxMenu {
                x,
                y,
                items: vec![
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
                ],
            });
        }
    }

    if let Some(file) = closest(&target, ".fb-row[data-workspace-path]") {
        let path = file
            .get_attribute("data-workspace-path")
            .unwrap_or_default();
        if !path.is_empty() {
            let file_name = path.rsplit('/').next().unwrap_or(path.as_str()).to_string();
            return Some(CtxMenu {
                x,
                y,
                items: vec![
                    item("copyName", i18n::t(locale, "ctx.copy_name"), file_name),
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
                    item(
                        "downloadFile",
                        i18n::t(locale, "artifact.download"),
                        path.clone(),
                    ),
                    item(
                        "revealInFileManager",
                        i18n::t(locale, "ctx.reveal_in_manager"),
                        path,
                    ),
                    item(
                        "renameWorkspaceFile",
                        i18n::t(locale, "files.rename_file"),
                        file.get_attribute("data-workspace-path")
                            .unwrap_or_default(),
                    ),
                    item(
                        "deleteWorkspaceFile",
                        i18n::t(locale, "files.delete_file"),
                        file.get_attribute("data-workspace-path")
                            .unwrap_or_default(),
                    ),
                ],
            });
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
        "cut" | "paste" | "selectAll" => text_entry_command(action),
        "copy" if payload.is_empty() => text_entry_command("copy"),
        "copy" | "copyCode" | "copyTitle" | "copyName" | "copyMessage" if !payload.is_empty() => {
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

pub fn session_action(action: &str, payload: &str) -> Option<SessionAction> {
    match action {
        "openSession" if !payload.is_empty() => Some(SessionAction::Open(payload.to_string())),
        "deleteSession" if !payload.is_empty() => Some(SessionAction::Delete(payload.to_string())),
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

#[component]
pub fn ContextMenuPortal(
    menu: ReadSignal<Option<CtxMenu>>,
    set_menu: WriteSignal<Option<CtxMenu>>,
    on_pick: Callback<(String, String)>,
) -> impl IntoView {
    view! {
        {move || {
            let m = menu.get()?;
            if m.items.is_empty() {
                return None;
            }
            let items = m.items.clone();
            let item_count = items.len() as f64;
            let (viewport_width, viewport_height) = web_sys::window()
                .and_then(|window| Some((window.inner_width().ok()?.as_f64()?, window.inner_height().ok()?.as_f64()?)))
                .unwrap_or((m.x + 280.0, m.y + item_count * 38.0 + 12.0));
            let estimated_width = 280.0_f64.min((viewport_width - 16.0).max(168.0));
            let estimated_height = (item_count * 38.0 + 12.0).min((viewport_height - 16.0).max(50.0));
            let left = m.x.max(8.0).min((viewport_width - estimated_width - 8.0).max(8.0));
            let top = m.y.max(8.0).min((viewport_height - estimated_height - 8.0).max(8.0));
            Some(view! {
                <div class="ctx-backdrop" on:click=move |_| set_menu.set(None)></div>
                <div
                    class="ctx-menu"
                    role="menu"
                    style=format!("left:{left}px;top:{top}px")
                    on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
                >
                    {items.into_iter().map(|it| {
                        let action = it.action.clone();
                        let payload = it.payload.clone();
                        let danger = matches!(
                            action.as_str(),
                            "deleteSession"
                                | "deleteFolder"
                                | "deleteWorkspaceFile"
                                | "deleteWorkspaceDirectory"
                        );
                        view! {
                            <button
                                type="button"
                                class="ctx-item"
                                class:danger=danger
                                on:click=move |_| {
                                    on_pick.call((action.clone(), payload.clone()));
                                    set_menu.set(None);
                                }
                            >{it.label}</button>
                        }
                    }).collect_view()}
                </div>
            }.into_view())
        }}
    }
}
