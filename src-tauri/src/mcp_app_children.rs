//! Native child-view ownership. No database, tool execution or plugin credentials.
//! The registry is also used by the disposable native smoke, so lifecycle tests
//! exercise the production implementation rather than a second window manager.
use crate::workspace_surface::{WorkspaceManager, WorkspaceSurface};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager, Webview};
use wisp_dto::{McpAppChildBounds, McpAppChildCloseReason, McpAppChildDelivery, McpAppChildHandle};

pub(crate) const CHILD_PREFIX: &str = "mcp-app-child-";
pub(crate) const STALE: &str = "stale-instance: this MCP App view has been closed or replaced";

#[derive(Clone)]
pub(crate) struct Child {
    pub owner: String,
    pub instance_id: String,
    pub handle: McpAppChildHandle,
    pub payload: Arc<Value>,
    pub host_context: Value,
    pub bounds: McpAppChildBounds,
    pub alive: Arc<AtomicBool>,
    pub ready: bool,
    pub profile: Option<PathBuf>,
    pub bridge_generation: Option<u64>,
}

impl Child {
    pub(crate) fn ensure_live(&self) -> Result<(), String> {
        if self.alive.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(STALE.into())
        }
    }
}

struct Owner {
    epoch: String,
    serial: u64,
    closed: bool,
}
impl Default for Owner {
    fn default() -> Self {
        Self {
            epoch: uuid::Uuid::new_v4().to_string(),
            serial: 0,
            closed: false,
        }
    }
}

#[derive(Clone)]
struct Binding {
    serial: u64,
    bridge_generation: Option<u64>,
}

#[derive(Default)]
pub(crate) struct Registry {
    owners: HashMap<String, Owner>,
    children: HashMap<String, Child>,
    // Logical ownership survives a Tab switch. Closing one view must not revoke
    // the same session's bridge while another window still has that App open.
    bindings: HashMap<(String, String), Binding>,
}

#[derive(Default)]
pub(crate) struct Retirement {
    pub children: Vec<Child>,
    pub releases: Vec<(String, Option<u64>)>,
}

impl Registry {
    pub(crate) fn epoch(&mut self, owner: &str) -> String {
        self.owners.entry(owner.into()).or_default().epoch.clone()
    }

    fn retire_matching(&mut self, predicate: impl Fn(&Child) -> bool) -> Vec<Child> {
        let labels: Vec<_> = self
            .children
            .values()
            .filter(|c| predicate(c))
            .map(|c| c.handle.child_label.clone())
            .collect();
        labels
            .into_iter()
            .filter_map(|label| self.children.remove(&label))
            .inspect(|c| {
                c.alive.store(false, Ordering::SeqCst);
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reserve(
        &mut self,
        owner: &str,
        epoch: &str,
        serial: u64,
        instance_id: &str,
        payload: Value,
        context: Value,
        bounds: McpAppChildBounds,
        bridge_generation: Option<u64>,
    ) -> Result<(Child, Retirement), String> {
        let o = self.owners.get_mut(owner).ok_or(STALE)?;
        if o.epoch != epoch || serial == 0 || serial <= o.serial {
            return Err(STALE.into());
        }
        o.serial = serial;
        o.closed = false;
        let retired = self.retire_matching(|c| c.owner == owner);
        let child = Child {
            owner: owner.into(),
            instance_id: instance_id.into(),
            handle: McpAppChildHandle {
                owner_epoch: epoch.into(),
                mount_serial: serial,
                child_label: format!("{CHILD_PREFIX}{}", uuid::Uuid::new_v4()),
            },
            payload: Arc::new(payload),
            host_context: context,
            bounds,
            alive: Arc::new(AtomicBool::new(true)),
            ready: false,
            profile: None,
            bridge_generation,
        };
        self.bindings.insert(
            (owner.into(), instance_id.into()),
            Binding {
                serial,
                bridge_generation,
            },
        );
        self.children
            .insert(child.handle.child_label.clone(), child.clone());
        Ok((
            child,
            Retirement {
                children: retired,
                releases: vec![],
            },
        ))
    }

    pub(crate) fn get(&self, label: &str) -> Result<Child, String> {
        let c = self.children.get(label).ok_or(STALE)?;
        c.ensure_live()?;
        Ok(c.clone())
    }

    pub(crate) fn get_by_instance(&self, owner: &str, instance: &str) -> Result<Child, String> {
        self.children
            .values()
            .find(|c| c.owner == owner && c.instance_id == instance)
            .cloned()
            .ok_or_else(|| STALE.into())
    }

    pub(crate) fn has_binding(&self, instance: &str) -> bool {
        self.bindings.keys().any(|(_, id)| id == instance)
    }

    pub(crate) fn suspend_owner(&mut self, owner: &str) -> Retirement {
        Retirement {
            children: self.retire_matching(|c| c.owner == owner),
            releases: vec![],
        }
    }

    pub(crate) fn close(
        &mut self,
        owner: &str,
        epoch: &str,
        serial: u64,
        instance: &str,
        reason: McpAppChildCloseReason,
    ) -> Retirement {
        let Some(o) = self.owners.get_mut(owner).filter(|o| o.epoch == epoch) else {
            return Retirement::default();
        };
        // A close can arrive before a slow open reaches reserve(). Fence it out.
        if serial >= o.serial {
            o.serial = serial;
            o.closed = true;
        }
        let children = self.retire_matching(|c| {
            c.owner == owner && c.handle.mount_serial == serial && c.instance_id == instance
        });
        let mut releases = vec![];
        if reason == McpAppChildCloseReason::UserClose {
            let key = (owner.into(), instance.into());
            if self.bindings.get(&key).is_some_and(|b| b.serial == serial) {
                let old = self.bindings.remove(&key).unwrap();
                if !self.bindings.keys().any(|(_, i)| i == instance) {
                    releases.push((instance.into(), old.bridge_generation));
                }
            }
        }
        Retirement { children, releases }
    }

    pub(crate) fn reset_owner(&mut self, owner: &str, destroy: bool) -> Retirement {
        let children = self.retire_matching(|c| c.owner == owner);
        let mut releases = vec![];
        if destroy {
            self.owners.remove(owner);
            let keys: Vec<_> = self
                .bindings
                .keys()
                .filter(|(o, _)| o == owner)
                .cloned()
                .collect();
            for key in keys {
                let old = self.bindings.remove(&key).unwrap();
                if !self.bindings.keys().any(|(_, i)| i == &key.1) {
                    releases.push((key.1, old.bridge_generation));
                }
            }
        } else {
            self.owners.insert(owner.into(), Owner::default());
        }
        Retirement { children, releases }
    }

    pub(crate) fn remove_frame(&mut self, frame_prefix: &str) -> Retirement {
        self.bindings
            .retain(|(_, id), _| !id.starts_with(frame_prefix));
        Retirement {
            children: self.retire_matching(|c| c.instance_id.starts_with(frame_prefix)),
            releases: vec![],
        }
    }
}

pub(crate) struct McpAppChildren {
    pub registry: Mutex<Registry>,
    pub actions:
        Mutex<HashMap<String, (String, tokio::sync::oneshot::Sender<Result<Value, String>>)>>,
    pub native_ops: tokio::sync::Mutex<()>,
    boot_id: String,
    pub profile_root: Option<PathBuf>,
}
impl Default for McpAppChildren {
    fn default() -> Self {
        Self {
            registry: Mutex::new(Registry::default()),
            actions: Mutex::new(HashMap::new()),
            native_ops: tokio::sync::Mutex::new(()),
            boot_id: uuid::Uuid::new_v4().to_string(),
            profile_root: None,
        }
    }
}

/// Explicit allowlist for application commands. Core/plugin commands are gated
/// separately by capabilities scoped to webview labels (no window inheritance).
pub(crate) fn child_command_allowed(label: &str, command: &str) -> bool {
    !label.starts_with(CHILD_PREFIX)
        || matches!(
            command,
            "mcp_app_child_bootstrap"
                | "mcp_app_child_ready"
                | "mcp_app_child_request"
                | "mcp_app_child_action_reply"
        )
}

pub(crate) fn child_navigation_allowed(url: &url::Url) -> bool {
    // Do not let a plugin turn its trusted shell into arbitrary app assets or a
    // remote document with access to the shell's IPC endpoints.
    url.path() == "/mcp-app/shell.html"
        && ((url.scheme() == "tauri" && url.host_str() == Some("localhost"))
            || (matches!(url.scheme(), "http" | "https")
                && url.host_str() == Some("tauri.localhost"))
            || (cfg!(debug_assertions)
                && url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
                && url.port() == Some(1421)))
}

pub(crate) fn deliver(
    app: &AppHandle,
    child: &Child,
    message: &McpAppChildDelivery,
) -> Result<(), String> {
    let view = app.get_webview(&child.handle.child_label).ok_or(STALE)?;
    // JSON serialization is the only interpolation; there is no plugin-supplied
    // JavaScript callback name. This never broadcasts via the event bus.
    let json = serde_json::to_string(message).map_err(|e| e.to_string())?;
    view.eval(format!("window.__wispMcpReceive?.({json});"))
        .map_err(|e| e.to_string())
}

pub(crate) fn retire_native(app: &AppHandle, children: Vec<Child>) {
    if children.is_empty() {
        return;
    }
    let Some(state) = app.try_state::<McpAppChildren>() else {
        return;
    };
    for child in children {
        let app = app.clone();
        // Invalidate immediately, hide immediately, and finish native close even
        // if the guest's event loop never handles resource-teardown.
        state
            .actions
            .lock()
            .unwrap()
            .retain(|_, (label, _)| label != &child.handle.child_label);
        tauri::async_runtime::spawn(async move {
            let Some(state) = app.try_state::<McpAppChildren>() else {
                return;
            };
            {
                let _native = state.native_ops.lock().await;
                if let Some(view) = app.get_webview(&child.handle.child_label) {
                    let _ = view.hide();
                }
                let _ = deliver(
                    &app,
                    &child,
                    &McpAppChildDelivery {
                        kind: "teardown".into(),
                        request_id: None,
                        method: None,
                        params: Value::Null,
                    },
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            {
                let Some(state) = app.try_state::<McpAppChildren>() else {
                    return;
                };
                let _native = state.native_ops.lock().await;
                if let Some(view) = app.get_webview(&child.handle.child_label) {
                    let _ = view.close();
                }
            }
            if let Some(profile) = child.profile {
                cleanup_profile(profile).await;
            }
        });
    }
}

async fn cleanup_profile(profile: PathBuf) {
    // Only a generated leaf in this feature's per-boot directory is eligible.
    if !profile
        .file_name()
        .and_then(|p| p.to_str())
        .is_some_and(|p| p.starts_with(CHILD_PREFIX))
        || profile
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|p| p.to_str())
            .and_then(|p| uuid::Uuid::parse_str(p).ok())
            .is_none()
    {
        return;
    }
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let p = profile.clone();
        let done =
            tauri::async_runtime::spawn_blocking(move || match std::fs::remove_dir_all(&p) {
                Ok(()) => true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
                Err(_) => false,
            })
            .await
            .unwrap_or(false);
        if done {
            return;
        }
    }
    tracing::warn!(target: "wisp", "MCP App profile is still locked; deferred cleanup (no user data removed)");
}

/// Validate/clamp in CSS space before converting using the primary WebView's
/// real physical client size. This handles monitor DPI and page zoom together.
/// `origin` is parent-window-client-relative; Wisp passes (0, 0) because the
/// primary document fills the client area.
pub(crate) fn physical_bounds(
    b: &McpAppChildBounds,
    size: tauri::PhysicalSize<u32>,
    origin: tauri::PhysicalPosition<i32>,
) -> Option<tauri::Rect> {
    if !b.visible
        || [
            b.x,
            b.y,
            b.width,
            b.height,
            b.viewport_width,
            b.viewport_height,
        ]
        .iter()
        .any(|n| !n.is_finite())
        || b.viewport_width <= 0.0
        || b.viewport_height <= 0.0
        || b.width <= 0.0
        || b.height <= 0.0
        || size.width == 0
        || size.height == 0
    {
        return None;
    }
    let left = b.x.max(0.0).min(b.viewport_width);
    let top = b.y.max(0.0).min(b.viewport_height);
    let right = (b.x + b.width).max(left).min(b.viewport_width);
    let bottom = (b.y + b.height).max(top).min(b.viewport_height);
    let sx = size.width as f64 / b.viewport_width;
    let sy = size.height as f64 / b.viewport_height;
    let (x, y, r, d) = (
        (left * sx).ceil() as i32,
        (top * sy).ceil() as i32,
        (right * sx).floor() as i32,
        (bottom * sy).floor() as i32,
    );
    if r <= x || d <= y {
        return None;
    }
    Some(tauri::Rect {
        position: tauri::PhysicalPosition::new(origin.x + x, origin.y + y).into(),
        size: tauri::PhysicalSize::new((r - x) as u32, (d - y) as u32).into(),
    })
}

pub(crate) fn apply_bounds(app: &AppHandle, child: &Child) -> Result<(), String> {
    child.ensure_live()?;
    let view = app.get_webview(&child.handle.child_label).ok_or(STALE)?;
    let owner = app.workspace_surface(&child.owner).ok_or(STALE)?;
    // Child set_bounds is parent-window-client-relative (Tauri 2.12 / wry
    // WebView2). The primary document fills that client area. Do not add
    // Webview::position() of the primary webview: for a webview-window it
    // returns the window inner position, which is desktop-relative.
    let bounds = physical_bounds(
        &child.bounds,
        owner.webview().size().map_err(|e| e.to_string())?,
        tauri::PhysicalPosition::new(0, 0),
    );
    if !child.ready
        || !owner.is_visible().unwrap_or(false)
        || owner.is_minimized().unwrap_or(true)
        || bounds.is_none()
    {
        view.hide().map_err(|e| e.to_string())?;
    } else if let Some(bounds) = bounds {
        view.set_bounds(bounds).map_err(|e| e.to_string())?;
        // Recheck after native setters so a concurrent close cannot reshow it.
        child.ensure_live()?;
        view.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) async fn create_native(
    app: &AppHandle,
    owner: &WorkspaceSurface,
    child: &Child,
) -> Result<(), String> {
    let state = app.state::<McpAppChildren>();
    child.ensure_live()?;
    let root = match &state.profile_root {
        Some(root) => root.clone(),
        None => app
            .path()
            .app_cache_dir()
            .map_err(|e| e.to_string())?
            .join("mcp-app-webviews"),
    }
    .join(&state.boot_id);
    let profile = root.join(&child.handle.child_label);
    {
        let state = app.state::<McpAppChildren>();
        let mut r = state.registry.lock().unwrap();
        r.children
            .get_mut(&child.handle.child_label)
            .ok_or(STALE)?
            .profile = Some(profile.clone());
    }
    std::fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
    let label = child.handle.child_label.clone();
    let window = owner.webview().window();
    let alive = child.alive.clone();
    // Serialize with hide/close. Do not hold this lock while waiting for the
    // shell to become ready, and do not hold the registry lock here.
    let state = app.state::<McpAppChildren>();
    let _native = state.native_ops.lock().await;
    child.ensure_live()?;
    let created_label = label.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        if !alive.load(Ordering::SeqCst) {
            return Err(STALE.into());
        }
        let builder = tauri::webview::WebviewBuilder::new(
            label,
            tauri::WebviewUrl::App("mcp-app/shell.html".into()),
        )
        .data_directory(profile)
        .focused(false)
        .disable_drag_drop_handler()
        .on_navigation(child_navigation_allowed)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_download(|_, _| false);
        let view = window
            .add_child(
                builder,
                tauri::LogicalPosition::new(-10000.0, -10000.0),
                tauri::LogicalSize::new(1.0, 1.0),
            )
            .map_err(|e| e.to_string())?;
        view.hide().map_err(|e| e.to_string())?;
        if !alive.load(Ordering::SeqCst) {
            let _ = view.close();
            return Err(STALE.into());
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Err(error) = result {
        return Err(error);
    }
    if child.ensure_live().is_err() {
        if let Some(view) = app.get_webview(&created_label) {
            let _ = view.hide();
            let _ = view.close();
        }
        return Err(STALE.into());
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) async fn create_native(
    _: &AppHandle,
    _: &WorkspaceSurface,
    _: &Child,
) -> Result<(), String> {
    Err("Native MCP App children are enabled on Windows only.".into())
}

pub(crate) fn caller_child(app: &AppHandle, webview: &Webview) -> Result<Child, String> {
    let child = app
        .state::<McpAppChildren>()
        .registry
        .lock()
        .unwrap()
        .get(webview.label())?;
    if webview.window().label() != child.owner {
        return Err(STALE.into());
    }
    Ok(child)
}

pub(crate) fn update_geometry(
    app: &AppHandle,
    owner: &str,
    handle: &McpAppChildHandle,
    bounds: McpAppChildBounds,
    context: Value,
) -> Result<bool, String> {
    let child = {
        let state = app.state::<McpAppChildren>();
        let mut r = state.registry.lock().unwrap();
        let c = r.children.get_mut(&handle.child_label).ok_or(STALE)?;
        if c.owner != owner || c.handle != *handle {
            return Err(STALE.into());
        }
        if bounds.revision <= c.bounds.revision {
            return Ok(false);
        }
        c.bounds = bounds;
        c.host_context = context;
        c.clone()
    };
    apply_bounds(app, &child)?;
    let _ = deliver(
        app,
        &child,
        &McpAppChildDelivery {
            kind: "host-context".into(),
            request_id: None,
            method: None,
            params: child.host_context.clone(),
        },
    );
    Ok(true)
}

pub(crate) fn set_ready(app: &AppHandle, label: &str) -> Result<Child, String> {
    let child = {
        let state = app.state::<McpAppChildren>();
        let mut r = state.registry.lock().unwrap();
        let c = r.children.get_mut(label).ok_or(STALE)?;
        c.ready = true;
        c.clone()
    };
    apply_bounds(app, &child)?;
    Ok(child)
}

pub(crate) fn hide_owner(app: &AppHandle, owner: &str) {
    let children: Vec<_> = app
        .state::<McpAppChildren>()
        .registry
        .lock()
        .unwrap()
        .children
        .values()
        .filter(|c| c.owner == owner)
        .cloned()
        .collect();
    for c in children {
        if let Some(w) = app.get_webview(&c.handle.child_label) {
            let _ = w.hide();
        }
    }
    // Bounds are stale until the primary document has measured the new client
    // area/DPI. Do not restore using the pre-resize coordinates.
    if let Some(view) = app.get_webview(owner) {
        let _ = view.eval("window.dispatchEvent(new Event('wisp-mcp-native-resize')); ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn open(
        r: &mut Registry,
        owner: &str,
        epoch: &str,
        serial: u64,
        instance: &str,
    ) -> Result<(Child, Retirement), String> {
        r.reserve(
            owner,
            epoch,
            serial,
            instance,
            Value::Null,
            Value::Null,
            McpAppChildBounds::default(),
            Some(serial),
        )
    }
    #[test]
    fn close_before_open_and_old_page_cannot_resurrect_a_child() {
        let mut r = Registry::default();
        let epoch = r.epoch("main");
        r.close("main", &epoch, 1, "app", McpAppChildCloseReason::Suspend);
        assert!(open(&mut r, "main", &epoch, 1, "app").is_err());
        let (a, _) = open(&mut r, "main", &epoch, 2, "app").unwrap();
        r.reset_owner("main", false);
        assert!(a.ensure_live().is_err());
        assert!(open(&mut r, "main", &epoch, 3, "app").is_err());
    }
    #[test]
    fn replacement_is_bounded_and_stale_close_cannot_revoke_new_binding() {
        let mut r = Registry::default();
        let e = r.epoch("main");
        let (a, _) = open(&mut r, "main", &e, 1, "app").unwrap();
        let (b, retired) = open(&mut r, "main", &e, 2, "app").unwrap();
        assert_eq!(retired.children.len(), 1);
        assert!(a.ensure_live().is_err());
        let retirement = r.close("main", &e, 1, "app", McpAppChildCloseReason::UserClose);
        assert!(retirement.releases.is_empty());
        assert!(r.get(&b.handle.child_label).is_ok());
        assert_eq!(r.children.len(), 1);
    }
    #[test]
    fn suspension_keeps_binding_and_two_owners_release_only_the_last_reference() {
        let mut r = Registry::default();
        let a = r.epoch("main");
        let b = r.epoch("proj-b");
        open(&mut r, "main", &a, 1, "app").unwrap();
        open(&mut r, "proj-b", &b, 1, "app").unwrap();
        assert!(r
            .close("main", &a, 1, "app", McpAppChildCloseReason::Suspend)
            .releases
            .is_empty());
        assert!(r
            .close("main", &a, 1, "app", McpAppChildCloseReason::UserClose)
            .releases
            .is_empty());
        assert_eq!(
            r.close("proj-b", &b, 1, "app", McpAppChildCloseReason::UserClose)
                .releases,
            vec![("app".into(), Some(1))]
        );
    }
    #[test]
    fn ipc_and_navigation_are_fail_closed() {
        assert!(child_command_allowed(
            "mcp-app-child-x",
            "mcp_app_child_request"
        ));
        for cmd in [
            "stop_agent",
            "read_file",
            "update_mcp_app_context",
            "call_mcp_app_tool",
            "close_mcp_app",
            "open_mcp_app_child",
        ] {
            assert!(!child_command_allowed("mcp-app-child-x", cmd));
        }
        assert!(child_navigation_allowed(
            &"http://tauri.localhost/mcp-app/shell.html".parse().unwrap()
        ));
        for url in [
            "http://tauri.localhost/index.html",
            "https://example.com/mcp-app/shell.html",
            "file:///mcp-app/shell.html",
        ] {
            assert!(!child_navigation_allowed(&url.parse().unwrap()));
        }
    }
    #[test]
    fn dpi_zoom_clipping_and_invalid_rects() {
        let b = McpAppChildBounds {
            x: 100.0,
            y: 60.0,
            width: 300.0,
            height: 200.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
            visible: true,
            revision: 1,
        };
        for scale in [1.0, 1.5, 2.0] {
            let rect = physical_bounds(
                &b,
                tauri::PhysicalSize::new((800.0 * scale) as u32, (600.0 * scale) as u32),
                tauri::PhysicalPosition::new(0, 0),
            )
            .unwrap();
            assert_eq!(
                rect.position.to_physical::<i32>(1.0).x,
                (100.0 * scale) as i32
            );
            assert_eq!(
                rect.size.to_physical::<u32>(1.0).width,
                (300.0 * scale) as u32
            );
        }
        // Parent-relative origin is 0 when the primary document fills the
        // window client area. A desktop-relative inner position (e.g. 1920,1080)
        // must not be mixed in by apply_bounds.
        let origin_zero = physical_bounds(
            &b,
            tauri::PhysicalSize::new(800, 600),
            tauri::PhysicalPosition::new(0, 0),
        )
        .unwrap();
        assert_eq!(origin_zero.position.to_physical::<i32>(1.0).x, 100);
        assert_eq!(origin_zero.position.to_physical::<i32>(1.0).y, 60);
        assert!(physical_bounds(
            &McpAppChildBounds {
                width: f64::NAN,
                ..b
            },
            tauri::PhysicalSize::new(800, 600),
            tauri::PhysicalPosition::new(0, 0)
        )
        .is_none());
    }

    #[test]
    fn repeated_switches_leave_one_child_and_session_delete_invalidates_calls() {
        let mut registry = Registry::default();
        let epoch = registry.epoch("main");
        for n in 1..=100 {
            let (child, retired) =
                open(&mut registry, "main", &epoch, n, "mcp-app:session-a:ui://a").unwrap();
            assert!(retired.children.len() <= 1);
            assert_eq!(registry.children.len(), 1);
            let retired = registry.suspend_owner("main");
            assert_eq!(retired.children.len(), 1);
            assert!(child.ensure_live().is_err());
            assert!(retired.releases.is_empty());
        }
        assert_eq!(registry.bindings.len(), 1);
        let (child, _) = open(
            &mut registry,
            "main",
            &epoch,
            101,
            "mcp-app:session-a:ui://a",
        )
        .unwrap();
        registry.remove_frame("mcp-app:session-a:");
        assert!(child.ensure_live().is_err());
        assert!(registry.bindings.is_empty());
        assert!(registry.children.is_empty());
    }

    #[test]
    fn capabilities_never_inherit_permissions_from_native_window() {
        for json in [
            include_str!("../capabilities/default.json"),
            include_str!("../capabilities/pet.json"),
            include_str!("../capabilities/terminal.json"),
            include_str!("../capabilities/mcp-app-child.json"),
        ] {
            let capability: Value = serde_json::from_str(json).unwrap();
            assert!(capability.get("windows").is_none());
            assert!(capability["webviews"].is_array());
        }
        let child: Value =
            serde_json::from_str(include_str!("../capabilities/mcp-app-child.json")).unwrap();
        assert_eq!(child["permissions"], serde_json::json!([]));
    }
}
