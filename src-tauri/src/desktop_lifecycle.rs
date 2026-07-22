#[cfg(target_os = "windows")]
use tauri::{
    menu::{Menu, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, Emitter, WebviewUrl, WebviewWindowBuilder,
};
use tauri::{AppHandle, Manager};

#[cfg(target_os = "windows")]
pub(crate) const PET_WINDOW_LABEL: &str = "pet";

#[cfg(any(target_os = "windows", test))]
pub(crate) fn should_hide_workspace_on_close(window_label: &str) -> bool {
    window_label == "main"
}

pub(crate) fn should_activate_workspace_window(window_label: &str) -> bool {
    window_label != "pet"
}

pub(crate) fn activate_workspace(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();

    for (label, window) in app.webview_windows() {
        if should_activate_workspace_window(&label) {
            let _ = window.show();
            let _ = window.unminimize();
        }
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.set_focus();
    }
}

#[cfg(target_os = "windows")]
fn default_pet_position(app: &AppHandle) -> Option<(f64, f64)> {
    let monitor = app.primary_monitor().ok().flatten()?;
    let origin = monitor.position();
    let size = monitor.size();
    Some((
        f64::from(origin.x) + f64::from(size.width.saturating_sub(152)),
        f64::from(origin.y) + f64::from(size.height.saturating_sub(230)),
    ))
}

#[cfg(target_os = "windows")]
fn ensure_pet_window(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(PET_WINDOW_LABEL).is_some() {
        return Ok(());
    }
    let url = WebviewUrl::App("index.html?pet=desktop".into());
    let mut builder = WebviewWindowBuilder::new(app, PET_WINDOW_LABEL, url)
        .title("Wisp pet")
        .inner_size(128.0, 140.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .closable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false);
    if let Some((x, y)) = default_pet_position(app) {
        builder = builder.position(x, y);
    }
    builder.build().map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn sync_pet_window(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if !enabled {
        if let Some(window) = app.get_webview_window(PET_WINDOW_LABEL) {
            let _ = window.hide();
        }
        return Ok(());
    }
    ensure_pet_window(app)?;
    let _ = app.emit_to(PET_WINDOW_LABEL, "pet-config-changed", ());
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn sync_pet_window(_app: &tauri::AppHandle, _enabled: bool) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
pub(crate) fn set_pet_window_visible(app: tauri::AppHandle, visible: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if visible {
            ensure_pet_window(&app)?;
        }
        if let Some(window) = app.get_webview_window(PET_WINDOW_LABEL) {
            if visible {
                window.show().map_err(|error| error.to_string())?;
            } else {
                window.hide().map_err(|error| error.to_string())?;
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (app, visible);
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn install_windows_shell(app: &mut App) -> tauri::Result<()> {
    let show = MenuItemBuilder::with_id("tray-show", "Open Wisp Science").build(app)?;
    let quit = MenuItemBuilder::with_id("tray-quit", "Quit").build(app)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("wisp-tray")
        .menu(&menu)
        .tooltip("Wisp Science")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray-show" => activate_workspace(app),
            "tray-quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                activate_workspace(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;

    if let Some(main) = app.get_webview_window("main") {
        let app_handle = app.handle().clone();
        main.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if should_hide_workspace_on_close("main") {
                    // Hide only the main window (it lives on in the tray).
                    // Per-project windows close independently (#420).
                    api.prevent_close();
                    if let Some(main) = app_handle.get_webview_window("main") {
                        let _ = main.hide();
                    }
                }
            }
        });
    }
    Ok(())
}
