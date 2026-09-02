#[cfg(target_os = "windows")]
use tauri::{
    menu::{Menu, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, WebviewUrl, WebviewWindowBuilder,
};
use tauri::{AppHandle, Emitter, Manager};

pub(crate) const PET_WINDOW_LABEL: &str = "pet";

#[cfg(target_os = "windows")]
pub(crate) const PET_WINDOW_WIDTH: u32 = 128;
#[cfg(target_os = "windows")]
pub(crate) const PET_WINDOW_HEIGHT: u32 = 176;

#[cfg(target_os = "windows")]
const PET_WINDOW_RIGHT_MARGIN: u32 = 24;
#[cfg(target_os = "windows")]
const PET_WINDOW_BOTTOM_MARGIN: u32 = 90;

#[cfg(any(target_os = "windows", test))]
pub(crate) fn should_hide_workspace_on_close(window_label: &str) -> bool {
    window_label == "main"
}

pub(crate) fn should_activate_workspace_window(window_label: &str) -> bool {
    window_label != "pet"
}

#[cfg(target_os = "windows")]
const TRAY_ID: &str = "wisp-tray";

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, PartialEq, Eq)]
enum TrayAction {
    Show,
    Restart,
    Quit,
}

#[cfg(any(target_os = "windows", test))]
fn tray_action(id: &str) -> Option<TrayAction> {
    match id {
        "tray-show" => Some(TrayAction::Show),
        "tray-restart" => Some(TrayAction::Restart),
        "tray-quit" => Some(TrayAction::Quit),
        _ => None,
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrayLocale {
    En,
    Zh,
}

#[cfg(any(target_os = "windows", test))]
impl TrayLocale {
    fn from_tag(tag: &str) -> Self {
        match tag.trim() {
            "zh" | "zh-CN" | "zh-TW" => Self::Zh,
            _ => Self::En,
        }
    }
}

#[cfg(any(target_os = "windows", test))]
struct TrayLabels {
    show: &'static str,
    restart: &'static str,
    quit: &'static str,
}

#[cfg(any(target_os = "windows", test))]
fn tray_labels(locale: TrayLocale) -> TrayLabels {
    match locale {
        TrayLocale::Zh => TrayLabels {
            show: "打开 Wisp Science",
            restart: "重启",
            quit: "退出",
        },
        TrayLocale::En => TrayLabels {
            show: "Open Wisp Science",
            restart: "Restart",
            quit: "Quit",
        },
    }
}

#[cfg(target_os = "windows")]
fn build_tray_menu<M: Manager<tauri::Wry>>(
    app: &M,
    locale: TrayLocale,
) -> tauri::Result<Menu<tauri::Wry>> {
    let labels = tray_labels(locale);
    let show = MenuItemBuilder::with_id("tray-show", labels.show).build(app)?;
    let restart = MenuItemBuilder::with_id("tray-restart", labels.restart).build(app)?;
    let quit = MenuItemBuilder::with_id("tray-quit", labels.quit).build(app)?;
    Menu::with_items(app, &[&show, &restart, &quit])
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

/// Restore one workspace window and optionally navigate it to a session.
///
/// Notification callbacks use the window that produced the notification. If
/// that project window was closed in the meantime, the long-lived main window
/// is a safe fallback that can switch to the target project in-place.
pub(crate) fn activate_workspace_window(
    app: &AppHandle,
    preferred_label: &str,
    target: Option<serde_json::Value>,
) {
    #[cfg(target_os = "macos")]
    let _ = app.show();

    let window = app
        .get_webview_window(preferred_label)
        .or_else(|| app.get_webview_window("main"));
    let Some(window) = window else {
        return;
    };
    let _ = window.show();
    let _ = window.unminimize();
    if let Some(target) = target {
        // `emit` broadcasts to every window (all open projects would jump to
        // this session); `emit_to` keeps the navigation in the one window
        // being activated.
        let _ = window.emit_to(window.label(), "open-session", target);
    }
    let _ = window.set_focus();
}

#[cfg(target_os = "windows")]
fn default_pet_position(app: &AppHandle) -> Option<(f64, f64)> {
    let monitor = app.primary_monitor().ok().flatten()?;
    let origin = monitor.position();
    let size = monitor.size();
    Some((
        f64::from(origin.x)
            + f64::from(
                size.width
                    .saturating_sub(PET_WINDOW_WIDTH + PET_WINDOW_RIGHT_MARGIN),
            ),
        f64::from(origin.y)
            + f64::from(
                size.height
                    .saturating_sub(PET_WINDOW_HEIGHT + PET_WINDOW_BOTTOM_MARGIN),
            ),
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
        .inner_size(f64::from(PET_WINDOW_WIDTH), f64::from(PET_WINDOW_HEIGHT))
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
        .visible(false)
        .on_navigation(crate::guard_webview_navigation);
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
pub(crate) fn apply_windows_tray_locale(app: &AppHandle, locale_tag: &str) -> Result<(), String> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    let menu = build_tray_menu(app, TrayLocale::from_tag(locale_tag)).map_err(|e| e.to_string())?;
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn install_windows_shell(app: &mut App, locale_tag: &str) -> tauri::Result<()> {
    let menu = build_tray_menu(app.handle(), TrayLocale::from_tag(locale_tag))?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("Wisp Science")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match tray_action(event.id().as_ref()) {
            Some(TrayAction::Show) => activate_workspace(app),
            Some(TrayAction::Restart) => app.request_restart(),
            Some(TrayAction::Quit) => app.exit(0),
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

#[cfg(test)]
mod tests {
    use super::{tray_action, tray_labels, TrayAction, TrayLocale};

    #[test]
    fn windows_tray_actions_include_restart() {
        assert_eq!(tray_action("tray-show"), Some(TrayAction::Show));
        assert_eq!(tray_action("tray-restart"), Some(TrayAction::Restart));
        assert_eq!(tray_action("tray-quit"), Some(TrayAction::Quit));
        assert_eq!(tray_action("unknown"), None);
    }

    #[test]
    fn windows_tray_labels_follow_saved_locale() {
        let zh = tray_labels(TrayLocale::from_tag("zh-CN"));
        assert_eq!(zh.show, "打开 Wisp Science");
        assert_eq!(zh.restart, "重启");
        assert_eq!(zh.quit, "退出");

        let en = tray_labels(TrayLocale::from_tag("en"));
        assert_eq!(en.show, "Open Wisp Science");
        assert_eq!(en.restart, "Restart");
        assert_eq!(en.quit, "Quit");

        assert_eq!(TrayLocale::from_tag("zh"), TrayLocale::Zh);
        assert_eq!(TrayLocale::from_tag("zh-TW"), TrayLocale::Zh);
        assert_eq!(TrayLocale::from_tag(""), TrayLocale::En);
        assert_eq!(TrayLocale::from_tag("fr"), TrayLocale::En);
    }
}
