//! Authenticated loopback adapter for SwiftUI/WinUI settings. Rendering remains
//! native; existing Tauri commands retain their validation and runtime ownership.
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde_json::Value;
use std::{collections::HashMap, fs::OpenOptions, io::Write, sync::Arc, time::Duration};
use tauri::{
    ipc::{CallbackFn, InvokeBody, InvokeResponse, InvokeResponseBody},
    Manager,
};
use tokio::sync::{oneshot, Mutex};
use wisp_dto::native_settings::{HostDescriptor, Request, Response, COMMANDS, SCHEMA};

#[derive(Clone)]
pub(crate) struct Broker {
    pub(crate) app: tauri::AppHandle,
    token: String,
    pub(crate) conversations: Arc<crate::native_conversations::Conversations>,
    contexts: Arc<Mutex<HashMap<Option<String>, String>>>,
}

pub(crate) fn requested(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().any(|arg| arg == "--native-settings-host")
}

pub(crate) fn start(app: &tauri::AppHandle) -> Result<(), String> {
    if app.try_state::<HostDescriptor>().is_some() {
        return Ok(());
    }
    let state = app.state::<crate::AppState>();
    let root = state.app_data.clone();
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|e| e.to_string())?;
    let endpoint = format!(
        "http://{}/invoke",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let descriptor = HostDescriptor {
        schema: SCHEMA.into(),
        endpoint,
        token: token.clone(),
        database: root.join("wisp.sqlite").to_string_lossy().into_owned(),
        pid: std::process::id(),
    };
    let staging = root.join(format!(".native-settings-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&staging).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(&descriptor).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&staging, root.join("native-settings.json")).map_err(|e| e.to_string())?;
    let broker = Broker {
        app: app.clone(),
        token,
        conversations: Arc::new(crate::native_conversations::Conversations::default()),
        contexts: Default::default(),
    };
    let router = Router::new()
        .route("/invoke", post(invoke))
        .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
        .with_state(broker);
    app.manage(descriptor);
    tauri::async_runtime::spawn(async move {
        if let Ok(listener) = tokio::net::TcpListener::from_std(listener) {
            let _ = axum::serve(listener, router).await;
        }
    });
    Ok(())
}

pub(crate) fn capabilities() -> Value {
    serde_json::json!({
        "commands": COMMANDS,
        "schema": SCHEMA,
        "conversations": wisp_dto::native_conversations::COMMANDS,
        "conversation_schema": wisp_dto::native_conversations::SCHEMA,
        "projects": wisp_dto::native_projects::COMMANDS,
        "project_schema": wisp_dto::native_projects::SCHEMA,
        "library": wisp_dto::native_library::COMMANDS,
        "library_schema": wisp_dto::native_library::SCHEMA,
        "calendar": wisp_dto::native_calendar::COMMANDS,
        "calendar_schema": wisp_dto::native_calendar::SCHEMA,
        "journey": wisp_dto::native_journey::COMMANDS,
        "journey_schema": wisp_dto::native_journey::SCHEMA,
    })
}

fn authorize(headers: &HeaderMap, expected: &str) -> bool {
    // A browser origin is never a native settings client. No CORS is enabled.
    !headers.contains_key("origin")
        && headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == format!("Bearer {expected}"))
}

async fn invoke(
    State(broker): State<Broker>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Result<Json<Response>, StatusCode> {
    if !authorize(&headers, &broker.token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let result = dispatch(&broker, &request).await;
    Ok(Json(Response {
        schema: SCHEMA.into(),
        id: request.id,
        result: result.as_ref().ok().cloned(),
        error: result.err(),
    }))
}

async fn dispatch(broker: &Broker, request: &Request) -> Result<Value, String> {
    if request.schema != SCHEMA || request.id.trim().is_empty() || !request.args.is_object() {
        return Err("Invalid native settings request".into());
    }
    if request.command == "native_settings_capabilities" {
        return Ok(capabilities());
    }
    if wisp_dto::native_projects::COMMANDS.contains(&request.command.as_str()) {
        let state = broker.app.state::<crate::AppState>();
        if wisp_dto::native_projects::returns_project_summary(&request.command) {
            let id =
                crate::native_projects::execute(&state.store, &state.app_data, request).await?;
            return serde_json::to_value(crate::build_project_summary(&state, &id).await)
                .map_err(|error| error.to_string());
        }
        return crate::native_projects::execute_folders(&state.store, request).await;
    }
    if wisp_dto::native_library::COMMANDS.contains(&request.command.as_str()) {
        let state = broker.app.state::<crate::AppState>();
        return crate::native_library::execute(&state.library, request).await;
    }
    if wisp_dto::native_calendar::COMMANDS.contains(&request.command.as_str()) {
        let state = broker.app.state::<crate::AppState>();
        return crate::native_calendar::execute(&state.store, request).await;
    }
    if wisp_dto::native_journey::COMMANDS.contains(&request.command.as_str()) {
        let state = broker.app.state::<crate::AppState>();
        return crate::native_journey::execute(&state.store, request).await;
    }
    if wisp_dto::native_conversations::COMMANDS.contains(&request.command.as_str()) {
        return crate::native_conversations::dispatch(broker, request).await;
    }
    if !COMMANDS.contains(&request.command.as_str()) {
        return Err("Command is not available to native settings".into());
    }
    if matches!(
        request.command.as_str(),
        "native_terminal_snapshot" | "write_terminal" | "close_terminal"
    ) {
        let id = request
            .args
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or("Missing authentication terminal id")?;
        let snapshot = broker
            .app
            .state::<crate::terminal_sessions::TerminalManager>()
            .native_auth_snapshot(id, request.project_id.as_deref())?;
        if request.command == "native_terminal_snapshot" {
            return serde_json::to_value(snapshot).map_err(|e| e.to_string());
        }
    }
    if request.command == "native_download_update" {
        crate::app_updates::download_update(
            broker.app.state(),
            tauri::ipc::Channel::new(|_| Ok(())),
        )
        .await?;
        return Ok(Value::Null);
    }
    invoke_command(
        broker,
        request.project_id.clone(),
        &request.command,
        request.args.clone(),
    )
    .await
}

pub(crate) async fn invoke_command(
    broker: &Broker,
    project_id: Option<String>,
    command: &str,
    args: Value,
) -> Result<Value, String> {
    let label = {
        let mut contexts = broker.contexts.lock().await;
        if let Some(label) = contexts.get(&project_id) {
            label.clone()
        } else {
            if contexts.len() >= 32 {
                return Err("Too many settings project contexts; restart the desktop host".into());
            }
            let active = if let Some(id) = &project_id {
                Some(
                    crate::project_commands::load_active_project(
                        &broker.app.state::<crate::AppState>(),
                        id,
                    )
                    .await?
                    .0,
                )
            } else {
                None
            };
            let label = format!("native-settings-{}", uuid::Uuid::new_v4().simple());
            let handle = broker.app.clone();
            let window_label = label.clone();
            let (tx, rx) = oneshot::channel();
            broker
                .app
                .run_on_main_thread(move || {
                    let result = tauri::WebviewWindowBuilder::new(
                        &handle,
                        &window_label,
                        tauri::WebviewUrl::App("native-host.html".into()),
                    )
                    .title("Wisp native settings host")
                    .visible(false)
                    .skip_taskbar(true)
                    .on_navigation(crate::guard_webview_navigation)
                    .build()
                    .map_err(|e| e.to_string());
                    if result.is_ok() {
                        if let Some(active) = active {
                            handle
                                .state::<crate::AppState>()
                                .set_active(&window_label, active);
                        }
                    }
                    let _ = tx.send(result.map(|_| ()));
                })
                .map_err(|e| e.to_string())?;
            rx.await.map_err(|_| "Settings host closed")??;
            contexts.insert(project_id.clone(), label.clone());
            label
        }
    };
    let webview = broker
        .app
        .get_webview(&label)
        .ok_or("Settings context closed")?;
    // Native callers enter from Rust, not JavaScript; use the configured local
    // app origin even before the inert context document finishes loading.
    let url = if cfg!(debug_assertions) && !cfg!(feature = "custom-protocol") {
        broker
            .app
            .config()
            .build
            .dev_url
            .clone()
            .unwrap_or(tauri::Url::parse("tauri://localhost").unwrap())
    } else {
        #[cfg(windows)]
        let origin = "http://tauri.localhost";
        #[cfg(not(windows))]
        let origin = "tauri://localhost";
        tauri::Url::parse(origin).unwrap()
    };
    let (tx, rx) = oneshot::channel();
    webview.on_message(
        tauri::webview::InvokeRequest {
            cmd: command.to_owned(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url,
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: broker.app.invoke_key().to_owned(),
        },
        Box::new(move |_, _, response, _, _| {
            let result = match response {
                InvokeResponse::Ok(InvokeResponseBody::Json(json)) => {
                    serde_json::from_str(&json).map_err(|_| "Invalid command response".into())
                }
                InvokeResponse::Ok(InvokeResponseBody::Raw(_)) => {
                    Err("Unsupported binary settings response".into())
                }
                InvokeResponse::Err(error) => Err(error
                    .0
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| error.0.to_string())),
            };
            let _ = tx.send(result);
        }),
    );
    if command == "send_message" {
        // The host owns the turn; HTTP acceptance has already returned. Keep
        // observing completion even if the UI disconnects or the turn is long.
        return rx.await.map_err(|_| "Conversation host closed")?;
    }
    tokio::time::timeout(Duration::from_secs(660), rx)
        .await
        .map_err(|_| "Settings command timed out; refresh to check the result before retrying")?
        .map_err(|_| "Settings host closed")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_auth_rejects_browser_origins_and_missing_or_wrong_tokens() {
        let mut headers = HeaderMap::new();
        assert!(!authorize(&headers, "secret"));
        headers.insert("authorization", "Bearer wrong".parse().unwrap());
        assert!(!authorize(&headers, "secret"));
        headers.insert("authorization", "Bearer secret".parse().unwrap());
        assert!(authorize(&headers, "secret"));
        headers.insert("origin", "http://localhost".parse().unwrap());
        assert!(!authorize(&headers, "secret"));
    }
    #[test]
    fn only_explicit_host_launch_enables_native_settings() {
        assert!(!requested(["wisp".into()]));
        assert!(requested(["wisp".into(), "--native-settings-host".into()]));
        assert!(!COMMANDS.contains(&"send_message"));
        assert!(!COMMANDS.contains(&"shell"));
        assert!(!COMMANDS.contains(&"native_project_create"));
        assert!(!COMMANDS.contains(&"native_library_delete"));
        assert!(!COMMANDS.contains(&"native_research_calendar"));
        assert!(!COMMANDS.contains(&"native_research_journey"));
        let advertised = capabilities();
        assert_eq!(advertised["projects"][0], "native_project_create");
        assert_eq!(
            advertised["project_schema"],
            wisp_dto::native_projects::SCHEMA
        );
        assert_eq!(advertised["library"][0], "native_library_search");
        assert_eq!(advertised["library"][1], "native_library_delete");
        assert_eq!(
            advertised["library_schema"],
            wisp_dto::native_library::SCHEMA
        );
        assert_eq!(advertised["calendar"][0], "native_research_calendar");
        assert_eq!(
            advertised["calendar_schema"],
            wisp_dto::native_calendar::SCHEMA
        );
        assert_eq!(advertised["journey"][0], "native_research_journey");
        assert_eq!(
            advertised["journey_schema"],
            wisp_dto::native_journey::SCHEMA
        );
        assert!(advertised["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|command| {
                command != "native_project_create"
                    && command != "native_library_search"
                    && command != "native_library_delete"
                    && command != "native_research_calendar"
                    && command != "native_research_journey"
            }));
    }
}
