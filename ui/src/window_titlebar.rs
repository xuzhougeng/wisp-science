//! Windows integrated title bar: brand, File/Edit/View/Help menus, window controls.

use crate::bindings::{arm_caption_drag, open_external_url, window_control};
use crate::i18n::{t, Locale};
use leptos::{ev, window_event_listener, *};
use wasm_bindgen::JsCast;

type MenuItem = (&'static str, &'static str, &'static str); // action, i18n key, shortcut

const FILE_ITEMS: &[MenuItem] = &[
    ("new", "command.new_session", "Ctrl+N"),
    ("new-window", "command.new_window", ""),
    ("projects", "command.projects", ""),
    ("files", "command.files", ""),
    (
        "export-current-project",
        "command.export_current_project",
        "",
    ),
    ("settings", "command.settings", "Ctrl+,"),
    ("", "", ""), // separator
    ("quit", "menu.quit", ""),
];

const EDIT_ITEMS: &[MenuItem] = &[
    ("search", "command.search", "Ctrl+K"),
    ("commands", "menu.commands", "Ctrl+P"),
    ("import-codex", "command.import_codex", ""),
    ("import-claude", "command.import_claude", ""),
    ("import-session", "command.import_session", ""),
    ("project-settings", "command.project_settings", ""),
    ("skills", "command.skills", ""),
];

const VIEW_ITEMS: &[MenuItem] = &[
    ("toggle-sidebar", "command.toggle_sidebar", "Ctrl+B"),
    ("artifacts", "command.artifacts", ""),
    ("notebook", "command.notebook", ""),
    ("files", "command.files", ""),
    ("provenance", "command.provenance", ""),
    ("contexts", "command.contexts", ""),
    ("side-chat", "command.side_chat", ""),
    ("close-panel", "command.close_panel", ""),
    ("", "", ""),
    ("theme-light", "command.theme_light", ""),
    ("theme-dark", "command.theme_dark", ""),
    ("theme-system", "command.theme_system", ""),
];

// Projects landing (home) variants: only actions that work without an open
// project. Anything session-scoped would sit there looking clickable but
// doing nothing, which reads as a bug.
const HOME_FILE_ITEMS: &[MenuItem] = &[
    ("new-project", "projects.new", "Ctrl+N"),
    ("new-window", "command.new_window", ""),
    ("import-project", "projects.import", ""),
    ("scratch", "command.scratch", "Ctrl+Shift+N"),
    ("settings", "command.settings", "Ctrl+,"),
    ("", "", ""), // separator
    ("quit", "menu.quit", ""),
];

const HOME_EDIT_ITEMS: &[MenuItem] = &[
    ("search", "command.search", "Ctrl+K"),
    ("commands", "menu.commands", "Ctrl+P"),
];

const HOME_VIEW_ITEMS: &[MenuItem] = &[
    ("theme-light", "command.theme_light", ""),
    ("theme-dark", "command.theme_dark", ""),
    ("theme-system", "command.theme_system", ""),
];

const HELP_ITEMS: &[MenuItem] = &[
    ("check-updates", "settings.check_updates", ""),
    ("", "", ""),
    ("docs", "menu.docs", ""),
    ("star-us", "menu.star_us", ""),
    ("issues", "menu.issues", ""),
];

/// Brand string used when no project is open. Keep in sync with
/// `src-tauri` `project_commands::APP_WINDOW_TITLE`.
pub(crate) const APP_WINDOW_TITLE: &str = "wisp science";

/// Window title shown in the custom Windows titlebar, taskbar, and Alt-Tab.
pub(crate) fn app_window_title(project_name: Option<&str>) -> String {
    match project_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("{APP_WINDOW_TITLE} \u{2014} {name}"),
        None => APP_WINDOW_TITLE.to_string(),
    }
}

#[component]
pub(super) fn WindowTitlebar(
    locale: RwSignal<Locale>,
    has_current_project: Signal<bool>,
    home: Signal<bool>,
    brand: Signal<String>,
    on_action: Callback<&'static str>,
) -> impl IntoView {
    let open = create_rw_signal(None::<&'static str>);

    let run = {
        let on_action = on_action.clone();
        Callback::new(move |action: &'static str| {
            open.set(None);
            match action {
                "quit" => spawn_local(async { window_control("close").await }),
                "docs" => {
                    open_external_url("https://github.com/xuzhougeng/wisp-science#readme".into())
                }
                "star-us" => open_external_url("https://github.com/xuzhougeng/wisp-science".into()),
                "issues" => {
                    open_external_url("https://github.com/xuzhougeng/wisp-science/issues".into())
                }
                other => on_action.call(other),
            }
        })
    };

    // (id, label key, session-page items, home-page items)
    let menus: &[(&'static str, &'static str, &[MenuItem], &[MenuItem])] = &[
        ("file", "menu.file", FILE_ITEMS, HOME_FILE_ITEMS),
        ("edit", "menu.edit", EDIT_ITEMS, HOME_EDIT_ITEMS),
        ("view", "menu.view", VIEW_ITEMS, HOME_VIEW_ITEMS),
        ("help", "menu.help", HELP_ITEMS, HELP_ITEMS),
    ];

    window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        if ev.key() != "Escape" || ev.default_prevented() || crate::text::ime_composing(ev) {
            return;
        }
        if open.get().is_some() {
            ev.prevent_default();
            open.set(None);
        }
    });

    view! {
        <header class="window-titlebar">
            <div class="window-brand" data-testid="window-snap-drag"
                on:mousedown=begin_window_move>
                <span class="window-brand-icon"></span>
                <span class="window-brand-name" data-testid="window-brand-title"
                    attr:title=move || brand.get()>{move || brand.get()}</span>
                <span class="window-brand-version">{concat!("v", env!("CARGO_PKG_VERSION"))}</span>
            </div>
            <nav class="window-menu" aria-label="Application menu">
                {menus.iter().map(|(id, label_key, items, home_items)| {
                    let id = *id;
                    let label_key = *label_key;
                    let items = *items;
                    let home_items = *home_items;
                    let run = run.clone();
                    view! {
                        <div class="window-menu-group">
                            <button type="button" class="window-menu-btn"
                                class:open=move || open.get() == Some(id)
                                aria-haspopup="menu"
                                aria-expanded=move || open.get() == Some(id)
                                on:click=move |ev| {
                                    ev.stop_propagation();
                                    open.update(|cur| *cur = if *cur == Some(id) { None } else { Some(id) });
                                }>
                                {move || t(locale.get(), label_key)}
                            </button>
                            {move || (open.get() == Some(id)).then(|| {
                                let run = run.clone();
                                let items = if home.get() { home_items } else { items };
                                view! {
                                    <div class="window-menu-drop" role="menu" on:click=|ev| ev.stop_propagation()>
                                        {items.iter().map(|(action, key, shortcut)| {
                                            let run = run.clone();
                                            if action.is_empty() {
                                                view! { <div class="window-menu-sep"></div> }.into_view()
                                            } else {
                                                let action = *action;
                                                let key = *key;
                                                let shortcut = *shortcut;
                                                view! {
                                                    <button type="button" role="menuitem"
                                                        disabled=move || matches!(action, "export-current-project" | "import-codex" | "import-claude") && !has_current_project.get()
                                                        on:click=move |_| run.call(action)>
                                                        <span>{move || t(locale.get(), key)}</span>
                                                        {(!shortcut.is_empty()).then(|| view! {
                                                            <kbd>{shortcut}</kbd>
                                                        })}
                                                    </button>
                                                }.into_view()
                                            }
                                        }).collect_view()}
                                    </div>
                                }
                            })}
                        </div>
                    }
                }).collect_view()}
            </nav>
            {move || open.get().is_some().then(|| view! {
                <div class="window-menu-backdrop" on:click=move |_| open.set(None)></div>
            })}
            <div class="window-drag" data-testid="window-snap-drag"
                on:mousedown=begin_window_move></div>
            <div class="window-controls">
                <button type="button" aria-label="Minimize"
                    on:click=move |_| spawn_local(async { window_control("minimize").await })>"−"</button>
                <button type="button" id="titlebar-maximize" data-testid="window-maximize"
                    aria-label="Maximize"
                    on:click=move |_| spawn_local(async { window_control("toggle-maximize").await })>"□"</button>
                <button type="button" class="window-close" aria-label="Close"
                    on:click=move |_| spawn_local(async { window_control("close").await })>"×"</button>
            </div>
        </header>
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptionPointerDown {
    Ignore,
    ToggleMaximize,
    ArmDrag,
}

fn caption_pointer_down(button: i16, detail: i32) -> CaptionPointerDown {
    if button != 0 {
        return CaptionPointerDown::Ignore;
    }
    if detail >= 2 {
        CaptionPointerDown::ToggleMaximize
    } else {
        CaptionPointerDown::ArmDrag
    }
}

fn begin_window_move(ev: web_sys::MouseEvent) {
    match caption_pointer_down(ev.button(), ev.detail()) {
        CaptionPointerDown::Ignore => {}
        CaptionPointerDown::ToggleMaximize => {
            ev.prevent_default();
            spawn_local(async { window_control("toggle-maximize").await });
        }
        CaptionPointerDown::ArmDrag => {
            ev.prevent_default();
            let start_x = f64::from(ev.client_x());
            let start_y = f64::from(ev.client_y());
            spawn_local(async move { arm_caption_drag(start_x, start_y).await });
        }
    }
}

#[cfg(test)]
mod caption_gesture_tests {
    use super::*;

    /// Typical Windows `SM_CXDRAG` / `SM_CYDRAG`. Keep in sync with `api.js`.
    const CAPTION_DRAG_THRESHOLD_PX: f64 = 4.0;

    fn caption_drag_ready(dx: f64, dy: f64) -> bool {
        dx.abs() >= CAPTION_DRAG_THRESHOLD_PX || dy.abs() >= CAPTION_DRAG_THRESHOLD_PX
    }

    #[test]
    fn left_click_arms_drag_instead_of_starting_a_move() {
        assert_eq!(caption_pointer_down(0, 1), CaptionPointerDown::ArmDrag);
    }

    #[test]
    fn double_click_toggles_maximize() {
        assert_eq!(
            caption_pointer_down(0, 2),
            CaptionPointerDown::ToggleMaximize
        );
        assert_eq!(
            caption_pointer_down(0, 3),
            CaptionPointerDown::ToggleMaximize
        );
    }

    #[test]
    fn other_buttons_are_ignored() {
        assert_eq!(caption_pointer_down(1, 1), CaptionPointerDown::Ignore);
        assert_eq!(caption_pointer_down(2, 2), CaptionPointerDown::Ignore);
    }

    #[test]
    fn drag_starts_only_after_the_windows_threshold() {
        assert!(!caption_drag_ready(3.0, 0.0));
        assert!(!caption_drag_ready(0.0, -3.0));
        assert!(caption_drag_ready(4.0, 0.0));
        assert!(caption_drag_ready(0.0, -4.0));
        assert!(caption_drag_ready(3.0, 4.0));
    }

    #[test]
    fn app_window_title_uses_the_project_name() {
        assert_eq!(app_window_title(None), APP_WINDOW_TITLE);
        assert_eq!(app_window_title(Some("")), APP_WINDOW_TITLE);
        assert_eq!(app_window_title(Some("   ")), APP_WINDOW_TITLE);
        assert_eq!(
            app_window_title(Some("fkbp1a-aortic-ring-assay")),
            "wisp science \u{2014} fkbp1a-aortic-ring-assay"
        );
        assert_eq!(
            app_window_title(Some("  fkbp1a  ")),
            "wisp science \u{2014} fkbp1a"
        );
    }
}
