//! Disposable native renderer isolation probe. No Store, MCP connection,
//! single-instance plugin, normal Wisp profile, or remote run is constructed.
//! Run: cargo run -p wisp-tauri --example mcp_app_isolation_smoke --offline
//! Optional: WISP_ISOLATION_CYCLES=100. Profiles remain in the printed temporary directory.
use serde_json::{json, Value};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{Manager, Webview};
use wisp_dto::{McpAppChildBootstrap, McpAppChildBounds, McpAppChildRequest};
#[path = "../src/mcp_app_children.rs"]
#[allow(dead_code)]
mod mcp_app_children;
#[path = "../src/ui_health.rs"]
#[allow(dead_code)]
mod ui_health;
#[path = "../src/workspace_surface.rs"]
#[allow(dead_code)]
mod workspace_surface;
use mcp_app_children::{Child, McpAppChildren};
use workspace_surface::{WorkspaceManager, WorkspaceSurface};

#[derive(Default)]
struct AppState {
    beats: Mutex<HashMap<String, u64>>,
    actions: Mutex<Vec<String>>,
    child_events: Mutex<Vec<String>>,
    stopped: Mutex<Vec<String>>,
}
impl AppState {
    fn active_frame(&self, label: &str) -> Option<String> {
        (label == "main").then(|| "fake-session".into())
    }
}
mod agent_turn {
    pub async fn stop_agent(
        state: tauri::State<'_, super::AppState>,
        id: Option<String>,
    ) -> Result<(), String> {
        state
            .stopped
            .lock()
            .unwrap()
            .push(id.expect("stop must be scoped"));
        Ok(())
    }
}
#[tauri::command]
fn smoke_tick(window: WorkspaceSurface, state: tauri::State<'_, AppState>) {
    *state
        .beats
        .lock()
        .unwrap()
        .entry(window.label().into())
        .or_default() += 1;
}
#[tauri::command]
fn smoke_control(window: WorkspaceSurface, state: tauri::State<'_, AppState>, action: String) {
    state
        .actions
        .lock()
        .unwrap()
        .push(format!("{}:{action}", window.label()));
}
#[tauri::command]
fn mcp_app_child_bootstrap(
    webview: Webview,
    app: tauri::AppHandle,
) -> Result<McpAppChildBootstrap, String> {
    let child = mcp_app_children::caller_child(&app, &webview)?;
    Ok(McpAppChildBootstrap {
        handle: child.handle,
        instance_id: child.instance_id,
        payload: (*child.payload).clone(),
        host_context: json!({}),
        version: "smoke".into(),
        server_tools_available: false,
    })
}
#[tauri::command]
fn mcp_app_child_ready(webview: Webview, app: tauri::AppHandle) -> Result<(), String> {
    let child = mcp_app_children::caller_child(&app, &webview)?;
    mcp_app_children::set_ready(&app, &child.handle.child_label)?;
    Ok(())
}
#[tauri::command]
fn mcp_app_child_request(
    webview: Webview,
    app: tauri::AppHandle,
    request: McpAppChildRequest,
) -> Result<Value, String> {
    let _ = mcp_app_children::caller_child(&app, &webview)?;
    app.state::<AppState>()
        .child_events
        .lock()
        .unwrap()
        .push(request.method.clone());
    let result = if request.method == "ui/initialize" {
        json!({"hostInfo":{"name":"wisp-science"},"hostCapabilities":{},"protocolVersion":"2026-01-26"})
    } else {
        json!({})
    };
    Ok(json!({"jsonrpc":"2.0", "id":request.id, "result":result}))
}
const MAIN: &str = r#"<!doctype html><body>
<button id="sidebar">Sidebar</button><button id="new-session">New session</button><button id="stop">Stop</button><input id="input">
<script>
for(const el of document.querySelectorAll('button,input')) el.addEventListener(el.tagName==='INPUT'?'input':'click',()=>window.__TAURI_INTERNALS__.invoke('smoke_control',{action:el.id}));
setInterval(()=>{window.__TAURI_INTERNALS__.invoke('smoke_tick');window.__TAURI_INTERNALS__.invoke('ui_heartbeat');},100);
</script></body>"#;
const GUEST: &str = r#"<!doctype html><body>Disposable blocked guest<script>
addEventListener('message',e=>{if(e.data?.method==='smoke/block'){parent.postMessage({jsonrpc:'2.0',method:'block-start',params:{}},'*');setTimeout(()=>{const end=performance.now()+20000;while(performance.now()<end){};parent.postMessage({jsonrpc:'2.0',method:'block-end',params:{}},'*')},100)}});
parent.postMessage({jsonrpc:'2.0',method:'ui/initialize',id:1,params:{}},'*');
</script></body>"#;
struct Assets;
impl tauri::Assets<tauri::Wry> for Assets {
    fn get(&self, key: &tauri::utils::assets::AssetKey) -> Option<Cow<'_, [u8]>> {
        let asset = match key.as_ref() {
            "mcp-app/shell.html" | "/mcp-app/shell.html" => {
                include_str!("../../ui/mcp-app/shell.html")
            }
            "mcp-app/shell.js" | "/mcp-app/shell.js" => include_str!("../../ui/mcp-app/shell.js"),
            "mcp_app_protocol.js" | "/mcp_app_protocol.js" => {
                include_str!("../../ui/src/mcp_app_protocol.js")
            }
            _ => MAIN,
        };
        Some(Cow::Borrowed(asset.as_bytes()))
    }
    fn iter(&self) -> Box<tauri::utils::assets::AssetsIter<'_>> {
        Box::new(std::iter::empty())
    }
    fn csp_hashes(
        &self,
        _: &tauri::utils::assets::AssetKey,
    ) -> Box<dyn Iterator<Item = tauri::utils::assets::CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}
fn wait(check: impl Fn() -> bool, what: &str, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if check() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(format!("timeout: {what}"))
}
fn create(app: &tauri::AppHandle, serial: u64) -> Result<Child, String> {
    let main = app.workspace_surface("main").ok_or("primary missing")?;
    let child = {
        let state = app.state::<McpAppChildren>();
        let mut r = state.registry.lock().unwrap();
        let e = r.epoch("main");
        let (child, retired) = r.reserve(
            "main",
            &e,
            serial,
            "mcp-app:fake-session:ui://smoke",
            json!({"tool":{"name":"smoke"},"resource":{"uri":"ui://smoke","text":GUEST}}),
            json!({}),
            McpAppChildBounds {
                x: 350.,
                y: 100.,
                width: 400.,
                height: 400.,
                viewport_width: 900.,
                viewport_height: 650.,
                visible: true,
                revision: 1,
            },
            None,
        )?;
        assert!(retired.children.is_empty());
        child
    };
    tauri::async_runtime::block_on(mcp_app_children::create_native(app, &main, &child))?;
    wait(
        || {
            app.state::<McpAppChildren>()
                .registry
                .lock()
                .unwrap()
                .get(&child.handle.child_label)
                .is_ok_and(|c| c.ready)
        },
        "child ready",
        Duration::from_secs(30),
    )?;
    Ok(child)
}
fn close(app: &tauri::AppHandle, child: &Child) -> Result<(), String> {
    let state = app.state::<McpAppChildren>();
    let retired = state.registry.lock().unwrap().close(
        "main",
        &child.handle.owner_epoch,
        child.handle.mount_serial,
        &child.instance_id,
        wisp_dto::McpAppChildCloseReason::Suspend,
    );
    mcp_app_children::retire_native(app, retired.children);
    wait(
        || app.get_webview(&child.handle.child_label).is_none(),
        "native child close",
        Duration::from_secs(3),
    )?;
    if child.ensure_live().is_ok() {
        return Err("retired identity remains live".into());
    }
    Ok(())
}
#[cfg(windows)]
fn browser_pid(view: &Webview) -> Result<u32, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    view.with_webview(move |platform| unsafe {
        let mut pid = 0;
        let r = platform
            .controller()
            .CoreWebView2()
            .and_then(|v| v.BrowserProcessId(&mut pid));
        let _ = tx.send(r.map(|_| pid).map_err(|e| e.to_string()));
    })
    .map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?
}
fn run(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    wait(
        || state.beats.lock().unwrap().len() == 2,
        "primary and sibling heartbeat",
        Duration::from_secs(30),
    )?;
    let main = app.workspace_surface("main").unwrap();
    let child = create(app, 1)?;
    // This is the regression Tauri's WebviewWindow extractor used to miss.
    if app.workspace_surfaces().len() != 2 {
        return Err("workspace lookup includes children or loses parents".into());
    }
    let view = app.get_webview(&child.handle.child_label).unwrap();
    #[cfg(windows)]
    {
        let primary_pid = browser_pid(main.webview())?;
        let child_pid = browser_pid(&view)?;
        println!("browser PID primary={primary_pid}, child={child_pid}");
        if primary_pid == child_pid {
            return Err("child and primary share the browser environment".into());
        }
        use std::os::windows::process::CommandExt;
        let script = format!("Get-CimInstance Win32_Process | Where-Object {{ $_.ParentProcessId -eq {primary_pid} -or $_.ParentProcessId -eq {child_pid} }} | Select-Object ProcessId,ParentProcessId,CommandLine | ConvertTo-Json -Compress");
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &script])
            .creation_flags(0x08000000)
            .output()
            .map_err(|e| e.to_string())?;
        println!(
            "dedicated process tree: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    view.eval("window.__TAURI_INTERNALS__.invoke('smoke_control',{action:'forged'}).then(()=>window.__TAURI_INTERNALS__.invoke('mcp_app_child_request',{request:{id:null,method:'security-failed',params:{}}}),()=>window.__TAURI_INTERNALS__.invoke('mcp_app_child_request',{request:{id:null,method:'security-denied',params:{}}}));").map_err(|e| e.to_string())?;
    wait(
        || {
            state
                .child_events
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "security-denied")
        },
        "child workspace command denied",
        Duration::from_secs(5),
    )?;
    view.eval(
        "document.querySelector('iframe').contentWindow.postMessage({method:'smoke/block'},'*');",
    )
    .map_err(|e| e.to_string())?;
    wait(
        || {
            state
                .child_events
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "block-start")
        },
        "guest block start",
        Duration::from_secs(5),
    )?;
    std::thread::sleep(Duration::from_millis(250));
    for _ in 0..10 {
        let count = state.actions.lock().unwrap().len();
        let started = Instant::now();
        main.eval("(()=>{for(const id of ['sidebar','new-session','stop'])document.getElementById(id).click();const input=document.getElementById('input');input.value='responsive';input.dispatchEvent(new Event('input'));})();").map_err(|e| e.to_string())?;
        wait(
            || state.actions.lock().unwrap().len() == count + 4,
            "primary DOM control responses",
            Duration::from_secs(1),
        )?;
        println!(
            "four primary DOM controls responded in {}ms during guest block",
            started.elapsed().as_millis()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    if state
        .child_events
        .lock()
        .unwrap()
        .iter()
        .any(|m| m == "block-end")
    {
        return Err("guest was not blocked during interaction checks".into());
    }
    ui_health::stop_window_agent(&main);
    ui_health::stop_window_agent(&app.workspace_surface("proj-smoke").unwrap());
    wait(
        || state.stopped.lock().unwrap().len() == 1,
        "scoped native stop",
        Duration::from_secs(1),
    )?;
    if state.stopped.lock().unwrap()[0] != "fake-session" {
        return Err("stop leaked to sibling".into());
    }
    let started = Instant::now();
    close(app, &child)?;
    println!(
        "blocked child closed in {}ms",
        started.elapsed().as_millis()
    );
    let cycles = std::env::var("WISP_ISOLATION_CYCLES")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5);
    for serial in 2..=cycles + 1 {
        let c = create(app, serial)?;
        close(app, &c)?;
        if serial % 10 == 0 {
            println!("cycles completed: {serial}");
        }
    }
    let children_root = app.state::<McpAppChildren>().profile_root.clone().unwrap();
    wait(
        || {
            std::fs::read_dir(&children_root)
                .map(|boots| {
                    boots.flatten().all(|boot| {
                        std::fs::read_dir(boot.path())
                            .map(|dirs| dirs.count() == 0)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        },
        "profile cleanup drained",
        Duration::from_secs(35),
    )?;
    println!("profile cleanup drained: zero child profiles; registry and controllers bounded");
    if app.webviews().len() != 2 {
        return Err("orphan native controller".into());
    }
    println!("PASS: isolated guest block, primary JS/DOM controls, scoped Stop, stale IPC denial; {} create/destroy cycles. Manual DPI/focus and real SFL acceptance still required.", cycles);
    Ok(())
}
fn main() {
    if !cfg!(windows) {
        eprintln!("Windows-only native probe");
        return;
    }
    // WebView2 adds several cache directories below each child profile. Keep
    // their paths short even when the checkout lives in a deep worktree.
    let root = tempfile::Builder::new()
        .prefix("wisp-mcp-")
        .tempdir()
        .expect("disposable WebView profile root")
        .keep();
    println!("disposable profiles: {}", root.display());
    let mut children = McpAppChildren::default();
    children.profile_root = Some(root.join("children"));
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = "science.wisp.isolation-smoke".into();
    context.config_mut().build.dev_url = None;
    context.set_assets(Box::new(Assets));
    tauri::Builder::default()
        .manage(AppState::default())
        .manage(children)
        .invoke_handler(|invoke| {
            if !mcp_app_children::child_command_allowed(
                invoke.message.webview().label(),
                invoke.message.command(),
            ) {
                invoke.resolver.reject("denied child command");
                return true;
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                smoke_tick,
                smoke_control,
                ui_health::ui_heartbeat,
                mcp_app_child_bootstrap,
                mcp_app_child_ready,
                mcp_app_child_request
            ];
            handler(invoke)
        })
        .setup(move |app| {
            for label in ["main", "proj-smoke"] {
                tauri::WebviewWindowBuilder::new(
                    app,
                    label,
                    tauri::WebviewUrl::App("index.html".into()),
                )
                .title("Disposable Wisp isolation test")
                .visible(false)
                .inner_size(900., 650.)
                .data_directory(root.join(label))
                .build()?;
            }
            let app = app.handle().clone();
            std::thread::spawn(move || {
                let result = run(&app);
                if let Err(error) = &result {
                    eprintln!("FAIL: {error}");
                }
                app.exit(if result.is_ok() { 0 } else { 1 });
            });
            Ok(())
        })
        .run(context)
        .expect("run native isolation probe");
}
