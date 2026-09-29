//! Windows snap for the custom title bar.
//!
//! Undecorated WebView2 windows do not participate in Aero Snap or Windows 11
//! Snap Layouts: the child webview eats `WM_NCHITTEST`, and JS `startDragging`
//! moves the window without a caption-style `SC_MOVE`.
//!
//! This module:
//! - starts a caption move (`SC_MOVE | HTCAPTION`) so drag-to-edge snap works
//!   (the UI arms that move only after the pointer travels `SM_CXDRAG`, so a
//!   title-bar double-click can still maximize / restore);
//! - places a transparent native child over the maximize button that returns
//!   `HTMAXBUTTON`, which is what Windows 11 uses for the Snap Layouts flyout.
//!
//! Geometry is platform-agnostic so tests do not need a real HWND.

use crate::workspace_surface::WorkspaceManager;
use crate::workspace_surface::WorkspaceSurface;

/// Must match `.window-titlebar { height }` in `ui/src/styles/base.css`.
pub const TITLEBAR_HEIGHT: u32 = 38;
/// Must match `.window-controls button { width }` in `ui/src/styles/base.css`.
pub const CONTROL_BUTTON_WIDTH: u32 = 46;
pub const CONTROL_BUTTON_COUNT: u32 = 3;
/// Close is 1, maximize is 2, minimize is 3, counting from the right edge.
pub const MAXIMIZE_INDEX_FROM_RIGHT: u32 = 2;
/// `SC_MOVE | HTCAPTION` — native move that participates in Aero Snap.
pub const SYSCOMMAND_MOVE_CAPTION: usize = 0xF012;

/// Logical maximize-button rect `(x, y, w, h)` inside the window client area.
pub fn maximize_button_rect(inner_width: u32, inner_height: u32) -> Option<(u32, u32, u32, u32)> {
    let min_width = CONTROL_BUTTON_WIDTH.saturating_mul(CONTROL_BUTTON_COUNT);
    if inner_width < min_width || inner_height < TITLEBAR_HEIGHT {
        return None;
    }
    let x =
        inner_width.saturating_sub(CONTROL_BUTTON_WIDTH.saturating_mul(MAXIMIZE_INDEX_FROM_RIGHT));
    Some((x, 0, CONTROL_BUTTON_WIDTH, TITLEBAR_HEIGHT))
}

pub fn should_install_snap(window_label: &str) -> bool {
    window_label != "pet"
}

#[tauri::command]
pub fn start_window_move(window: WorkspaceSurface) -> Result<(), String> {
    #[cfg(windows)]
    {
        return start_caption_move(&window);
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Ok(())
    }
}

pub fn install_for_window(window: &WorkspaceSurface) {
    if !should_install_snap(window.label()) {
        return;
    }
    // A Win32 window belongs to the thread that creates it. Project windows are
    // built on tokio workers, which never pump messages: a child owned there
    // would hang every cross-thread SendMessage from the UI thread (#1379).
    // Runs inline when already on the main thread (setup).
    #[cfg(windows)]
    {
        let target = window.clone();
        let dispatched = window.run_on_main_thread(move || {
            if let Err(error) = attach_maximize_overlay(&target) {
                tracing::warn!(label = %target.label(), %error, "windows snap overlay was not installed");
            }
        });
        if let Err(error) = dispatched {
            tracing::warn!(label = %window.label(), %error, "windows snap overlay was not scheduled");
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

#[cfg(windows)]
fn start_caption_move(window: &WorkspaceSurface) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_SYSCOMMAND};

    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    let hwnd_bits = hwnd.0 as isize;
    let window = window.clone();
    window
        .clone()
        .run_on_main_thread(move || {
            // Native caption drag from a maximized window restores first so
            // the user can pull it off the screen edge.
            if window.is_maximized().unwrap_or(false) {
                let _ = window.unmaximize();
            }
            let hwnd = HWND(hwnd_bits as *mut core::ffi::c_void);
            unsafe {
                let _ = ReleaseCapture();
                let _ = SendMessageW(
                    hwnd,
                    WM_SYSCOMMAND,
                    Some(WPARAM(SYSCOMMAND_MOVE_CAPTION)),
                    Some(LPARAM(0)),
                );
            }
        })
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn attach_maximize_overlay(window: &WorkspaceSurface) -> Result<(), String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    use tauri::Manager;
    use windows::core::w;
    use windows::Win32::Foundation::{SetLastError, HWND, LPARAM, LRESULT, WIN32_ERROR, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetClassInfoExW, GetWindowLongPtrW,
        LoadCursorW, RegisterClassExW, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos,
        ShowWindow, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HTMAXBUTTON, HWND_TOP,
        IDC_ARROW, LWA_ALPHA, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNA, WM_CREATE,
        WM_DESTROY, WM_LBUTTONDOWN, WM_NCHITTEST, WM_NCLBUTTONDOWN, WNDCLASSEXW, WS_CHILD,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_VISIBLE,
    };

    const CLASS: windows::core::PCWSTR = w!("WispSnapMaxButton");

    struct OverlayState {
        app: tauri::AppHandle,
        label: String,
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_CREATE => {
                let create = lparam.0 as *const CREATESTRUCTW;
                if !create.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
                }
                LRESULT(0)
            }
            WM_NCHITTEST => LRESULT(HTMAXBUTTON as isize),
            WM_NCLBUTTONDOWN | WM_LBUTTONDOWN => {
                toggle_parent_maximize(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if raw != 0 {
                    drop(Box::from_raw(raw as *mut OverlayState));
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    fn state<'a>(hwnd: HWND) -> Option<&'a OverlayState> {
        let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
        if raw == 0 {
            None
        } else {
            Some(unsafe { &*(raw as *const OverlayState) })
        }
    }

    fn toggle_parent_maximize(hwnd: HWND) {
        let Some(state) = state(hwnd) else {
            return;
        };
        let Some(window) = state.app.workspace_surface(&state.label) else {
            return;
        };
        if window.is_maximized().unwrap_or(false) {
            let _ = window.unmaximize();
        } else {
            let _ = window.maximize();
        }
    }

    fn logical_inner_size(window: &WorkspaceSurface) -> Option<(u32, u32, f64)> {
        let scale = window.scale_factor().ok()?;
        let size = window.inner_size().ok()?;
        let width = (f64::from(size.width) / scale).round() as u32;
        let height = (f64::from(size.height) / scale).round() as u32;
        Some((width, height, scale))
    }

    fn place(overlay: HWND, window: &WorkspaceSurface) {
        let Some((width, height, scale)) = logical_inner_size(window) else {
            return;
        };
        match maximize_button_rect(width, height) {
            Some((x, y, w, h)) => {
                let px = (f64::from(x) * scale).round() as i32;
                let py = (f64::from(y) * scale).round() as i32;
                let pw = (f64::from(w) * scale).round() as i32;
                let ph = (f64::from(h) * scale).round() as i32;
                unsafe {
                    let _ = ShowWindow(overlay, SW_SHOWNA);
                    let _ = SetWindowPos(
                        overlay,
                        Some(HWND_TOP),
                        px,
                        py,
                        pw,
                        ph,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                }
            }
            None => unsafe {
                let _ = ShowWindow(overlay, SW_HIDE);
            },
        }
    }

    fn overlays() -> std::sync::MutexGuard<'static, HashMap<String, isize>> {
        static OVERLAYS: OnceLock<Mutex<HashMap<String, isize>>> = OnceLock::new();
        OVERLAYS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    let parent = window.hwnd().map_err(|error| error.to_string())?;
    let label = window.label().to_string();

    if let Some(existing) = overlays().remove(&label) {
        unsafe {
            let _ = DestroyWindow(HWND(existing as *mut core::ffi::c_void));
        }
    }

    let instance = unsafe { GetModuleHandleW(None).map_err(|error| error.to_string())? };
    unsafe {
        let mut info = std::mem::zeroed::<WNDCLASSEXW>();
        if GetClassInfoExW(Some(instance.into()), CLASS, &mut info).is_err() {
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            // The failed GetClassInfoExW above leaves ERROR_CLASS_DOES_NOT_EXIST
            // behind; clear it so a failure below reports its own error.
            SetLastError(WIN32_ERROR(0));
            if RegisterClassExW(&class) == 0 {
                return Err(format!(
                    "could not register snap overlay class: {}",
                    windows::core::Error::from_win32()
                ));
            }
        }
    }

    let state = Box::new(OverlayState {
        app: window.app_handle().clone(),
        label: label.clone(),
    });
    let state_ptr = Box::into_raw(state);

    let overlay = unsafe {
        SetLastError(WIN32_ERROR(0));
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE,
            CLASS,
            w!(""),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            CONTROL_BUTTON_WIDTH as i32,
            TITLEBAR_HEIGHT as i32,
            Some(parent),
            None,
            Some(instance.into()),
            Some(state_ptr as *const core::ffi::c_void),
        )
    }
    .map_err(|error| {
        unsafe {
            drop(Box::from_raw(state_ptr));
        }
        error.to_string()
    })?;

    unsafe {
        let _ = SetLayeredWindowAttributes(overlay, Default::default(), 1, LWA_ALPHA);
    }
    place(overlay, window);

    let overlay_bits = overlay.0 as isize;
    overlays().insert(label.clone(), overlay_bits);

    let event_window = window.clone();
    let event_label = label;
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Resized(_)
        | tauri::WindowEvent::ScaleFactorChanged { .. }
        | tauri::WindowEvent::Moved(_) => {
            place(HWND(overlay_bits as *mut core::ffi::c_void), &event_window)
        }
        tauri::WindowEvent::Destroyed => {
            overlays().remove(&event_label);
        }
        _ => {}
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximize_button_sits_left_of_close() {
        let (x, y, w, h) = maximize_button_rect(1100, 760).expect("room for controls");
        assert_eq!((y, w, h), (0, 46, 38));
        assert_eq!(x, 1100 - 46 * 2);
        assert_eq!(x + w, 1100 - 46, "must not cover the close button");
        assert!(x >= 46, "must not cover the minimize button");
    }

    #[test]
    fn maximize_button_is_absent_when_the_window_is_too_narrow() {
        assert_eq!(maximize_button_rect(100, 760), None);
        assert_eq!(maximize_button_rect(1100, 20), None);
        assert_eq!(
            maximize_button_rect(CONTROL_BUTTON_WIDTH * CONTROL_BUTTON_COUNT - 1, 760),
            None
        );
    }

    #[test]
    fn caption_move_uses_the_aero_snap_syscommand() {
        assert_eq!(SYSCOMMAND_MOVE_CAPTION, 0xF012);
    }

    #[test]
    fn pet_window_does_not_get_a_snap_overlay() {
        assert!(!should_install_snap("pet"));
        assert!(should_install_snap("main"));
        assert!(should_install_snap("proj-abc"));
    }
}
