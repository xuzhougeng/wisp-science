use crate::i18n::{self, Locale};
use leptos::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/src/context_menu.js")]
extern "C" {
    fn isDevMode() -> bool;
    fn textareaCommand(kind: &str, id: &str);
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

fn add_export(items: &mut Vec<CtxItem>, locale: Locale, enabled: bool) {
    if enabled {
        items.push(item(
            "exportSession",
            i18n::t(locale, "ctx.export_session"),
            String::new(),
        ));
    }
}

fn event_target(ev: &web_sys::MouseEvent) -> Option<web_sys::Element> {
    ev.target()?.dyn_into::<web_sys::Element>().ok()
}

fn closest(el: &web_sys::Element, selector: &str) -> Option<web_sys::Element> {
    el.closest(selector).ok().flatten()
}

fn selection_text() -> Option<String> {
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
        let Some(block) = closest(el, sel) else { continue };
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
    Rename { id: String, title: String },
    Move { id: String, folder_id: Option<String> },
}

#[derive(Clone, PartialEq)]
pub enum FolderAction {
    Rename { id: String, name: String },
    Delete(String),
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

pub fn build(ev: &web_sys::MouseEvent, locale: Locale, can_export: bool) -> Option<CtxMenu> {
    let target = event_target(ev)?;
    let x = ev.client_x() as f64;
    let y = ev.client_y() as f64;

    if closest(&target, "textarea").is_none() {
        if let Some(text) = selection_text() {
            let mut items = vec![item("copy", i18n::t(locale, "ctx.copy"), text)];
            add_export(&mut items, locale, can_export);
            return Some(CtxMenu {
                x,
                y,
                items,
            });
        }
    }

    if closest(&target, "textarea").is_some() {
        return Some(CtxMenu {
            x,
            y,
            items: vec![
                item("cut", i18n::t(locale, "ctx.cut"), String::new()),
                item("copy", i18n::t(locale, "ctx.copy"), String::new()),
                item("paste", i18n::t(locale, "ctx.paste"), String::new()),
                item("selectAll", i18n::t(locale, "ctx.select_all"), String::new()),
            ],
        });
    }

    if let Some(code) = text_from_code_block(&target) {
        let mut items = vec![item("copyCode", i18n::t(locale, "ctx.copy_code"), code)];
        add_export(&mut items, locale, can_export);
        return Some(CtxMenu {
            x,
            y,
            items,
        });
    }

    if let Some(ses) = closest(&target, ".side-item.ses") {
        let title = ses.get_attribute("data-session-title").unwrap_or_default();
        let id = ses.get_attribute("data-session-id").unwrap_or_default();
        let mut items = vec![item(
            "copyTitle",
            i18n::t(locale, "ctx.copy_title"),
            title.clone(),
        )];
        if !id.is_empty() {
            items.push(item(
                "openSession",
                i18n::t(locale, "ctx.open_session"),
                id.clone(),
            ));
            items.push(item(
                "renameSession",
                i18n::t(locale, "ctx.rename_session"),
                format!("{id}\u{1e}{title}"),
            ));
            items.extend(session_move_items(&id, locale));
            items.push(item(
                "exportSession",
                i18n::t(locale, "ctx.export_session"),
                id.clone(),
            ));
            items.push(item(
                "deleteSession",
                i18n::t(locale, "ctx.delete_session"),
                id,
            ));
        }
        return Some(CtxMenu { x, y, items });
    }

    if let Some(folder) = closest(&target, ".side-folder") {
        let name = folder.get_attribute("data-folder-name").unwrap_or_default();
        let id = folder.get_attribute("data-folder-id").unwrap_or_default();
        if !id.is_empty() {
            return Some(CtxMenu {
                x,
                y,
                items: vec![
                    item(
                        "renameFolder",
                        i18n::t(locale, "ctx.rename_folder"),
                        format!("{id}\u{1e}{name}"),
                    ),
                    item(
                        "deleteFolder",
                        i18n::t(locale, "ctx.delete_folder"),
                        id,
                    ),
                ],
            });
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
                items.push(item("downloadFile", i18n::t(locale, "artifact.download"), path));
            }
            add_export(&mut items, locale, can_export);
            return Some(CtxMenu {
                x,
                y,
                items,
            });
        }
    }

    if let Some(file) = closest(&target, ".fb-row[data-workspace-path]") {
        let path = file.get_attribute("data-workspace-path").unwrap_or_default();
        if !path.is_empty() {
            return Some(CtxMenu {
                x,
                y,
                items: vec![
                    item("attachWorkspaceFile", i18n::t(locale, "ctx.attach_file"), path.clone()),
                    item("downloadFile", i18n::t(locale, "artifact.download"), path),
                ],
            });
        }
    }

    if let Some(body) = closest(&target, ".msg .body") {
        let text = body.text_content().unwrap_or_default();
        if !text.trim().is_empty() {
            let mut items = vec![item(
                "copyMessage",
                i18n::t(locale, "ctx.copy_message"),
                text,
            )];
            add_export(&mut items, locale, can_export);
            return Some(CtxMenu {
                x,
                y,
                items,
            });
        }
    }

    let mut items = vec![];
    add_export(&mut items, locale, can_export);
    Some(CtxMenu {
        x,
        y,
        items,
    })
}

pub fn run_action(action: &str, payload: &str, copy: impl Fn(String)) {
    match action {
        "cut" | "paste" | "selectAll" => textareaCommand(action, "composer-input"),
        "copy" if payload.is_empty() => textareaCommand("copy", "composer-input"),
        "copy" | "copyCode" | "copyTitle" | "copyName" | "copyMessage" if !payload.is_empty() => {
            copy(payload.to_string());
        }
        _ => {}
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
            Some(view! {
                <div class="ctx-backdrop" on:click=move |_| set_menu.set(None)></div>
                <div
                    class="ctx-menu"
                    style=format!("left:{}px;top:{}px", m.x, m.y)
                    on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
                >
                    {items.into_iter().map(|it| {
                        let action = it.action.clone();
                        let payload = it.payload.clone();
                        let danger = action == "deleteSession" || action == "deleteFolder";
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
