use crate::app_support::ProjectsScreen;
use crate::bindings::invoke;
use crate::dto::*;
use crate::i18n::Locale;
use leptos::*;
use std::collections::HashSet;
use wasm_bindgen::JsValue;

#[derive(Clone, Copy)]
pub(super) struct ProjectLandingState {
    pub(super) home_dialog_open: RwSignal<bool>,
    pub(super) home_calendar_open: RwSignal<bool>,
    pub(super) show_projects: RwSignal<bool>,
    pub(super) demo_mode: RwSignal<bool>,
    pub(super) items: RwSignal<Vec<ChatItem>>,
    pub(super) active_session: RwSignal<Option<String>>,
    pub(super) project_open_error: RwSignal<Option<String>>,
    pub(super) demos: RwSignal<Vec<DemoInfo>>,
    pub(super) modal_artifact: RwSignal<Option<(String, String, String)>>,
    pub(super) locale: RwSignal<Locale>,
    pub(super) running: RwSignal<HashSet<String>>,
    pub(super) approval_pending: RwSignal<HashSet<String>>,
    pub(super) sync_actions_available: RwSignal<bool>,
    pub(super) command_palette_open: RwSignal<bool>,
    pub(super) project_transfer: RwSignal<Option<ProjectTransferProgress>>,
    pub(super) privacy_mode_active: RwSignal<bool>,
    pub(super) privacy_hidden_project_ids: RwSignal<HashSet<String>>,
    /// One-shot requests from the titlebar File menu; ProjectsScreen owns the
    /// actual dialogs and resets these flags.
    pub(super) menu_new_project: RwSignal<bool>,
    pub(super) menu_import_project: RwSignal<bool>,
}
#[component]
pub(super) fn ProjectLanding(
    state: ProjectLandingState,
    open_project: Callback<String>,
    open_project_folder: Callback<String>,
    open_project_session: Callback<(String, String)>,
    open_project_journey: Callback<(String, i64)>,
    open_scratch: Callback<()>,
    open_settings: Callback<Option<String>>,
    open_library: Callback<()>,
    open_project_export: Callback<(String, String)>,
) -> impl IntoView {
    let ProjectLandingState {
        home_calendar_open,
        home_dialog_open,
        show_projects,
        demo_mode,
        items,
        active_session,
        project_open_error,
        demos,
        modal_artifact,
        locale,
        running,
        approval_pending,
        sync_actions_available,
        command_palette_open,
        project_transfer,
        privacy_mode_active,
        privacy_hidden_project_ids,
        menu_new_project,
        menu_import_project,
    } = state;

    move || {
        show_projects.get().then(|| {
            let on_open_demo = Callback::new(move |_: ()| {
                project_open_error.set(None);
                show_projects.set(false);
                demo_mode.set(true);
                items.set(vec![]);
                active_session.set(None);
                spawn_local(async move {
                    let v = invoke("list_demos", JsValue::UNDEFINED).await;
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DemoInfo>>(v) {
                        demos.set(list);
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
                    calendar_open=home_calendar_open
                    dialog_open=home_dialog_open
                    running=running
                    approval_pending=approval_pending.read_only()
                    sync_actions_available=sync_actions_available.read_only()
                    open_error=project_open_error
                    on_open=open_project
                    on_open_folder=open_project_folder
                    on_open_session=open_project_session
                    on_open_journey=open_project_journey
                    on_open_artifact=on_open_artifact
                    on_open_settings=on_open_settings
                    on_open_library=open_library
                    on_open_demo=on_open_demo
                    on_open_scratch=open_scratch
                    on_search=Callback::new(move |_| command_palette_open.set(true))
                    on_export_project=open_project_export
                    project_transfer=project_transfer
                    privacy_mode_active=privacy_mode_active
                    privacy_hidden_project_ids=privacy_hidden_project_ids
                    menu_new_project=menu_new_project
                    menu_import_project=menu_import_project
                />
            }
        })
    }
}
