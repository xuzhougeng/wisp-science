//! Windows integrated title bar: brand, File/Edit/View/Help menus, window controls.

use crate::bindings::{open_external_url, window_control};
use crate::i18n::{t, Locale};
use leptos::{ev, window_event_listener, *};
use wasm_bindgen::JsCast;

type MenuItem = (&'static str, &'static str, &'static str); // action, i18n key, shortcut

const FILE_ITEMS: &[MenuItem] = &[
    ("new", "command.new_session", "Ctrl+N"),
    ("projects", "command.projects", ""),
    ("files", "command.files", ""),
    ("export-current-project", "command.export_current_project", ""),
    ("settings", "command.settings", "Ctrl+,"),
    ("", "", ""), // separator
    ("quit", "menu.quit", ""),
];

const EDIT_ITEMS: &[MenuItem] = &[
    ("search", "command.search", "Ctrl+K"),
    ("commands", "menu.commands", "Ctrl+P"),
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

const HELP_ITEMS: &[MenuItem] = &[
    ("check-updates", "settings.check_updates", ""),
    ("", "", ""),
    ("docs", "menu.docs", ""),
    ("star-us", "menu.star_us", ""),
    ("issues", "menu.issues", ""),
];

#[component]
pub(super) fn WindowTitlebar(
    locale: RwSignal<Locale>,
    has_current_project: Signal<bool>,
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
                "star-us" => {
                    open_external_url("https://github.com/xuzhougeng/wisp-science".into())
                }
                "issues" => {
                    open_external_url("https://github.com/xuzhougeng/wisp-science/issues".into())
                }
                other => on_action.call(other),
            }
        })
    };

    let menus: &[(&'static str, &'static str, &[MenuItem])] = &[
        ("file", "menu.file", FILE_ITEMS),
        ("edit", "menu.edit", EDIT_ITEMS),
        ("view", "menu.view", VIEW_ITEMS),
        ("help", "menu.help", HELP_ITEMS),
    ];

    window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        if ev.key() != "Escape" || ev.default_prevented() || ev.is_composing() {
            return;
        }
        if open.get().is_some() {
            ev.prevent_default();
            open.set(None);
        }
    });

    view! {
        <header class="window-titlebar" data-tauri-drag-region>
            <div class="window-brand" data-tauri-drag-region>
                <span class="window-brand-icon"></span>
                <span>"wisp-science"</span>
            </div>
            <nav class="window-menu" aria-label="Application menu">
                {menus.iter().map(|(id, label_key, items)| {
                    let id = *id;
                    let label_key = *label_key;
                    let items = *items;
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
                                                        disabled=move || action == "export-current-project" && !has_current_project.get()
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
            <div class="window-drag" data-tauri-drag-region></div>
            <div class="window-controls">
                <button type="button" aria-label="Minimize"
                    on:click=move |_| spawn_local(async { window_control("minimize").await })>"−"</button>
                <button type="button" aria-label="Maximize"
                    on:click=move |_| spawn_local(async { window_control("toggle-maximize").await })>"□"</button>
                <button type="button" class="window-close" aria-label="Close"
                    on:click=move |_| spawn_local(async { window_control("close").await })>"×"</button>
            </div>
        </header>
    }
}
