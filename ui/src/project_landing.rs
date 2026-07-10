use crate::app_support::{refresh_folders, refresh_sessions, ProjectsScreen};
use crate::bindings::invoke;
use crate::dto::*;
use crate::i18n::Locale;
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::collections::HashSet;
use wasm_bindgen::JsValue;

#[derive(Clone, Copy)]
pub(super) struct ProjectLandingState {
    pub(super) show_projects: RwSignal<bool>,
    pub(super) demo_mode: RwSignal<bool>,
    pub(super) items: RwSignal<Vec<ChatItem>>,
    pub(super) active_session: RwSignal<Option<String>>,
    pub(super) collapsed_folders: RwSignal<HashSet<String>>,
    pub(super) sessions: RwSignal<Vec<SessionInfo>>,
    pub(super) folders: RwSignal<Vec<FolderInfo>>,
    pub(super) project_info: RwSignal<Option<ProjectInfo>>,
    pub(super) demos: RwSignal<Vec<DemoInfo>>,
    pub(super) modal_artifact: RwSignal<Option<(String, String, String)>>,
    pub(super) locale: RwSignal<Locale>,
    pub(super) running: RwSignal<HashSet<String>>,
    pub(super) approval_pending: RwSignal<HashSet<String>>,
    pub(super) command_palette_open: RwSignal<bool>,
}
#[component]
pub(super) fn ProjectLanding(
    state: ProjectLandingState,
    load_session: Callback<String>,
    open_settings: Callback<Option<String>>,
) -> impl IntoView {
    let ProjectLandingState {
        show_projects,
        demo_mode,
        items,
        active_session,
        collapsed_folders,
        sessions,
        folders,
        project_info,
        demos,
        modal_artifact,
        locale,
        running,
        approval_pending,
        command_palette_open,
    } = state;

    move || show_projects.get().then(|| {
    let open = Callback::new(move |id: String| {
        show_projects.set(false);
        demo_mode.set(false);
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
            let _ = invoke("open_project", arg).await;
            // Reset the chat view for the newly-opened project, then reload
            // its project info + session list (reuses the existing helpers).
            items.set(vec![]);
            active_session.set(None);
            collapsed_folders.set(HashSet::new());
            refresh_sessions(sessions);
            refresh_folders(folders);
            let v = invoke("get_project_info", JsValue::UNDEFINED).await;
            if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) {
                project_info.set(Some(p));
            }
        });
    });
    let open_session = load_session.clone();
    let on_open_session = Callback::new(move |(project_id, session_id): (String, String)| {
        show_projects.set(false);
        demo_mode.set(false);
        let open_session = open_session.clone();
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "id": project_id })).unwrap();
            let _ = invoke("open_project", arg).await;
            // Project swap must land before loading the session (it switches
            // the backend's active project + session frame out from under us).
            open_session.call(session_id);
            refresh_sessions(sessions);
            let v = invoke("get_project_info", JsValue::UNDEFINED).await;
            if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) {
                project_info.set(Some(p));
            }
        });
    });
    let on_open_demo = Callback::new(move |_: ()| {
        show_projects.set(false);
        demo_mode.set(true);
        items.set(vec![]);
        active_session.set(None);
        spawn_local(async move {
            let v = invoke("list_demos", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DemoInfo>>(v) { demos.set(list); }
        });
    });
    let on_open_artifact = Callback::new(move |(path, name, kind): (String, String, String)| {
        modal_artifact.set(Some((path, name, kind)));
    });
    let on_open_settings = Callback::new(move |_: ()| open_settings.call(None));
    view! {
        <ProjectsScreen
            locale=locale
            running=running
            approval_pending=approval_pending.read_only()
            on_open=open
            on_open_session=on_open_session
            on_open_artifact=on_open_artifact
            on_open_settings=on_open_settings
            on_open_demo=on_open_demo
            on_search=Callback::new(move |_| command_palette_open.set(true))
        />
    }
})
}
