use crate::app_support::{js_error_text, ProjectsScreen};
use crate::bindings::{invoke, invoke_checked};
use crate::dto::*;
use crate::i18n::Locale;
use leptos::*;
use std::collections::HashSet;
use wasm_bindgen::JsValue;

#[derive(Clone, Copy)]
pub(super) struct ProjectLandingState {
    pub(super) show_projects: RwSignal<bool>,
    pub(super) demo_mode: RwSignal<bool>,
    pub(super) items: RwSignal<Vec<ChatItem>>,
    pub(super) active_session: RwSignal<Option<String>>,
    pub(super) casual_session: RwSignal<Option<String>>,
    pub(super) project_open_error: RwSignal<Option<String>>,
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
    open_project: Callback<String>,
    open_project_session: Callback<(String, String)>,
    open_settings: Callback<Option<String>>,
    open_library: Callback<()>,
) -> impl IntoView {
    let ProjectLandingState {
        show_projects,
        demo_mode,
        items,
        active_session,
        casual_session,
        project_open_error,
        demos,
        modal_artifact,
        locale,
        running,
        approval_pending,
        command_palette_open,
    } = state;

    move || {
        show_projects.get().then(|| {
            let on_open_demo = Callback::new(move |_: ()| {
                project_open_error.set(None);
                show_projects.set(false);
                demo_mode.set(true);
                casual_session.set(None); // the demo view leaves any casual chat
                items.set(vec![]);
                active_session.set(None);
                spawn_local(async move {
                    let v = invoke("list_demos", JsValue::UNDEFINED).await;
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DemoInfo>>(v) {
                        demos.set(list);
                    }
                });
            });
            // Ephemeral chat owned by no visible project: leave the landing only
            // once the backend handshakes a session id, so a failure keeps the
            // landing (and its error banner) on screen.
            let on_open_casual = Callback::new(move |_: ()| {
                project_open_error.set(None);
                spawn_local(async move {
                    match invoke_checked("new_casual_session", JsValue::UNDEFINED).await {
                        Ok(v) => match v.as_string() {
                            Some(id) => {
                                casual_session.set(Some(id.clone()));
                                active_session.set(Some(id));
                                items.set(vec![]);
                                show_projects.set(false);
                            }
                            None => {
                                project_open_error.set(Some(
                                    "Could not start a casual chat: empty session id.".to_string(),
                                ));
                            }
                        },
                        Err(error) => {
                            project_open_error.set(Some(format!(
                                "Could not start a casual chat: {}",
                                js_error_text(error)
                            )));
                        }
                    }
                });
            });
            let on_open_artifact =
                Callback::new(move |(path, name, kind): (String, String, String)| {
                    modal_artifact.set(Some((path, name, kind)));
                });
            let on_open_settings = Callback::new(move |_: ()| open_settings.call(None));
            view! {
                <ProjectsScreen
                    locale=locale
                    running=running
                    approval_pending=approval_pending.read_only()
                    open_error=project_open_error
                    on_open=open_project
                    on_open_session=open_project_session
                    on_open_artifact=on_open_artifact
                    on_open_settings=on_open_settings
                    on_open_library=open_library
                    on_open_demo=on_open_demo
                    on_open_casual=on_open_casual
                    on_search=Callback::new(move |_| command_palette_open.set(true))
                />
            }
        })
    }
}
