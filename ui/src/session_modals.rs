use crate::app_support::{
    compose_icon, js_error_text, show_toast, FileEntryModal, FolderModal, SessionTransfer,
    SessionTransferMode,
};
use crate::bindings::invoke_checked;
use crate::dto::*;
use crate::i18n::localize_backend;
use crate::i18n::{t, tf, Locale};
use crate::text::{
    dom_value, event_target_checked, event_target_value, md_document_to_html, parent_path,
};
use crate::window_capture_escape;
use leptos::*;
use serde_wasm_bindgen::to_value;
use wasm_bindgen::JsCast;

#[derive(Clone, Copy)]
pub(crate) struct SessionTransferOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) session_transfer: RwSignal<Option<SessionTransfer>>,
    pub(crate) session_transfer_busy: RwSignal<bool>,
    pub(crate) session_transfer_error: RwSignal<Option<String>>,
    pub(crate) project_info: RwSignal<Option<ProjectInfo>>,
    pub(crate) proj_list: RwSignal<Vec<ProjectSummary>>,
}

#[component]
pub(crate) fn SessionTransferOverlay(
    state: SessionTransferOverlayState,
    on_save: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let SessionTransferOverlayState {
        locale,
        session_transfer,
        session_transfer_busy,
        session_transfer_error,
        project_info,
        proj_list,
    } = state;
    view! {
        {move || session_transfer.get().map(|transfer| {
            let active_project_id = project_info
                .get()
                .map(|project| project.id)
                .unwrap_or_default();
            let include_active = transfer.from_demo;
            let targets = proj_list
                .get()
                .into_iter()
                .filter(|project| include_active || project.id != active_project_id)
                .collect::<Vec<_>>();
            let has_target = !targets.is_empty() && !transfer.target_project_id.is_empty();
            let target_project_id = transfer.target_project_id.clone();
            let title_key = if transfer.from_demo {
                "session.copy_demo_title"
            } else if transfer.mode == SessionTransferMode::Copy {
                "session.copy_title"
            } else {
                "session.move_title"
            };
            let action_key = if transfer.mode == SessionTransferMode::Copy {
                "session.copy_action"
            } else {
                "session.move_action"
            };
            let hint_key = if transfer.from_demo {
                "session.copy_demo_hint"
            } else {
                "session.transfer_hint"
            };
            let empty_key = if transfer.from_demo {
                "session.no_target_project_demo"
            } else {
                "session.no_target_project"
            };
            view! {
            <div class="overlay">
                <div class="modal session-transfer-modal">
                    <h2>{move || t(locale.get(), title_key)}</h2>
                    <div class="hint">{tf(locale.get(), hint_key, &[("title", &transfer.title)])}</div>
                    <label>
                        {move || t(locale.get(), "session.target_project")}
                        <select
                            disabled=move || session_transfer_busy.get()
                            on:change=move |ev| {
                                let value = event_target_value(&ev);
                                session_transfer.update(|transfer| {
                                    if let Some(transfer) = transfer {
                                        transfer.target_project_id = value;
                                    }
                                });
                            }>
                            {targets.into_iter().map(|project| {
                                let selected = project.id == target_project_id;
                                view! {
                                    <option value=project.id prop:selected=selected>{project.name}</option>
                                }
                            }).collect_view()}
                        </select>
                    </label>
                    {(!has_target).then(|| view! {
                        <div class="hint session-transfer-error">{move || t(locale.get(), empty_key)}</div>
                    })}
                    {move || session_transfer_error.get().map(|error| view! {
                        <div class="hint session-transfer-error">{error}</div>
                    })}
                    <div class="row">
                        <button type="button"
                            disabled=move || session_transfer_busy.get()
                            on:click=move |_| {
                                session_transfer.set(None);
                                session_transfer_error.set(None);
                            }>{move || t(locale.get(), "settings.cancel")}</button>
                        <button type="button" class="primary"
                            disabled=move || !has_target || session_transfer_busy.get()
                            on:click=move |ev| on_save.call(ev)>{move || t(locale.get(), action_key)}</button>
                    </div>
                </div>
            </div>
        }.into_view()
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct RenameSessionOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) rename_session_target: RwSignal<Option<(String, String)>>,
    pub(crate) rename_session_input: RwSignal<String>,
}

#[component]
pub(crate) fn RenameSessionOverlay(
    state: RenameSessionOverlayState,
    on_renamed: Callback<(String, String)>,
) -> impl IntoView {
    let RenameSessionOverlayState {
        locale,
        rename_session_target,
        rename_session_input,
    } = state;
    view! {
        {move || rename_session_target.get().map(|(id, _)| {
            let id_key = id.clone();
            let id_btn = id.clone();
            view! {
            <div class="overlay">
                <div class="modal">
                    <h2>{move || t(locale.get(), "session.rename_title")}</h2>
                    <label>
                        <input
                            id="rename-session-input"
                            type="text"
                            autofocus=true
                            prop:value=move || rename_session_input.get()
                            on:input=move |ev| rename_session_input.set(dom_value(&ev))
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if (ev.ctrl_key() || ev.meta_key())
                                    && ev.key().eq_ignore_ascii_case("a")
                                {
                                    ev.prevent_default();
                                    if let Some(target) = ev.target() {
                                        if let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() {
                                            input.select();
                                        }
                                    }
                                    return;
                                }
                                if ev.key() == "Enter" {
                                    ev.prevent_default();
                                    let title = rename_session_input.get().trim().to_string();
                                    if title.is_empty() { return; }
                                    let id = id_key.clone();
                                    rename_session_target.set(None);
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "id": id.clone(), "title": title.clone() })).unwrap();
                                        if invoke_checked("rename_session", arg).await.is_ok() {
                                            on_renamed.call((id, title));
                                        }
                                    });
                                }
                            }
                        />
                    </label>
                    <div class="row">
                        <button on:click=move |_| rename_session_target.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button class="primary" on:click=move |_| {
                            let title = rename_session_input.get().trim().to_string();
                            if title.is_empty() { return; }
                            let id = id_btn.clone();
                            rename_session_target.set(None);
                            spawn_local(async move {
                                let arg = to_value(&serde_json::json!({ "id": id.clone(), "title": title.clone() })).unwrap();
                                if invoke_checked("rename_session", arg).await.is_ok() {
                                    on_renamed.call((id, title));
                                }
                            });
                        }>{move || t(locale.get(), "settings.save")}</button>
                    </div>
                </div>
            </div>
        }.into_view()
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FolderModalOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) folder_modal: RwSignal<Option<FolderModal>>,
    pub(crate) folder_modal_input: RwSignal<String>,
}

#[component]
pub(crate) fn FolderModalOverlay(
    state: FolderModalOverlayState,
    on_save: Callback<FolderModal>,
) -> impl IntoView {
    let FolderModalOverlayState {
        locale,
        folder_modal,
        folder_modal_input,
    } = state;
    view! {
        {move || folder_modal.get().map(|mode| {
            let mode_save = mode.clone();
            let mode_enter = mode.clone();
            let title_key = match &mode {
                FolderModal::Create => "folder.new_title",
                FolderModal::Rename(_) => "folder.rename_prompt",
            };
            let label_key = match &mode {
                FolderModal::Create => "folder.new_prompt",
                FolderModal::Rename(_) => "folder.new_prompt",
            };
            view! {
            <div class="overlay">
                <div class="modal">
                    <h2>{move || t(locale.get(), title_key)}</h2>
                    <label>
                        {move || t(locale.get(), label_key)}
                        // Chromium treats autocomplete="off" as a hint and still
                        // lists previously typed values. An unrecognized token
                        // opts the field out of that saved-info popup.
                        <input
                            id="folder-modal-input"
                            type="text"
                            name="wisp-group-name"
                            autocomplete="nope"
                            autocorrect="off"
                            autocapitalize="none"
                            spellcheck="false"
                            autofocus=true
                            prop:value=move || folder_modal_input.get()
                            on:input=move |ev| folder_modal_input.set(dom_value(&ev))
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ev.key() == "Enter" {
                                    ev.prevent_default();
                                    on_save.call(mode_enter.clone());
                                }
                            }
                        />
                    </label>
                    <div class="row">
                        <button on:click=move |_| folder_modal.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button class="primary" on:click=move |_| on_save.call(mode_save.clone())>{move || t(locale.get(), "settings.save")}</button>
                    </div>
                </div>
            </div>
        }.into_view()
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FileEntryOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) file_entry_modal: RwSignal<Option<FileEntryModal>>,
    pub(crate) file_entry_input: RwSignal<String>,
    pub(crate) file_entry_busy: RwSignal<bool>,
    pub(crate) file_entry_error: RwSignal<Option<String>>,
    pub(crate) file_cwd: RwSignal<String>,
}

#[component]
pub(crate) fn FileEntryOverlay(
    state: FileEntryOverlayState,
    on_save: Callback<FileEntryModal>,
) -> impl IntoView {
    let FileEntryOverlayState {
        locale,
        file_entry_modal,
        file_entry_input,
        file_entry_busy,
        file_entry_error,
        file_cwd,
    } = state;
    view! {
        {move || file_entry_modal.get().map(|mode| {
            let mode_save = mode.clone();
            let mode_enter = mode.clone();
            let (title_key, action_key, location) = match &mode {
                FileEntryModal::CreateFile => (
                    "files.new_file",
                    "files.create",
                    file_cwd.get_untracked(),
                ),
                FileEntryModal::CreateDirectory => (
                    "files.new_directory",
                    "files.create",
                    file_cwd.get_untracked(),
                ),
                FileEntryModal::Rename { path, is_dir } => (
                    if *is_dir { "files.rename_directory" } else { "files.rename_file" },
                    "files.rename",
                    parent_path(path),
                ),
            };
            view! {
                <div class="overlay">
                    <div class="modal file-entry-modal">
                        <h2>{move || t(locale.get(), title_key)}</h2>
                        <div class="hint file-entry-location">
                            {move || tf(locale.get(), "files.location", &[("path", &location)])}
                        </div>
                        <label>
                            {move || t(locale.get(), "files.name")}
                            <input
                                id="file-entry-modal-input"
                                type="text"
                                autofocus=true
                                disabled=move || file_entry_busy.get()
                                prop:value=move || file_entry_input.get()
                                on:input=move |ev| {
                                    file_entry_input.set(dom_value(&ev));
                                    file_entry_error.set(None);
                                }
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ev.key() == "Enter" {
                                        ev.prevent_default();
                                        on_save.call(mode_enter.clone());
                                    }
                                }
                            />
                        </label>
                        {move || file_entry_error.get().map(|error| view! {
                            <div class="settings-error" role="alert">{error}</div>
                        })}
                        <div class="row">
                            <button disabled=move || file_entry_busy.get() on:click=move |_| {
                                file_entry_modal.set(None);
                                file_entry_error.set(None);
                            }>{move || t(locale.get(), "settings.cancel")}</button>
                            <button class="primary" disabled=move || file_entry_busy.get()
                                on:click=move |_| on_save.call(mode_save.clone())>
                                {move || if file_entry_busy.get() {
                                    t(locale.get(), "files.working")
                                } else {
                                    t(locale.get(), action_key)
                                }}
                            </button>
                        </div>
                    </div>
                </div>
            }.into_view()
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TurnUndoOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) turn_undo_dialog: RwSignal<Option<TurnUndoDialog>>,
    pub(crate) turn_undo_busy: RwSignal<bool>,
    pub(crate) turn_undo_error: RwSignal<Option<String>>,
}

#[component]
pub(crate) fn TurnUndoOverlay(
    state: TurnUndoOverlayState,
    on_confirm: Callback<()>,
) -> impl IntoView {
    let TurnUndoOverlayState {
        locale,
        turn_undo_dialog,
        turn_undo_busy,
        turn_undo_error,
    } = state;
    view! {
        {move || turn_undo_dialog.get().map(|dialog| {
            let restore_files = dialog.preview.restore_files.clone();
            let remove_files = dialog.preview.remove_files.clone();
            let remove_artifacts = dialog.preview.remove_artifacts.clone();
            let unsupported_files = dialog.preview.unsupported_files.clone();
            let conflicts = dialog.preview.conflicts.clone();
            let has_text_changes = !restore_files.is_empty() || !remove_files.is_empty();
            let can_confirm = conflicts.is_empty();
            view! {
                <div class="overlay">
                    <div
                        class="modal confirm-modal turn-undo-modal"
                        role="dialog"
                        aria-modal="true"
                        aria-labelledby="turn-undo-title"
                        data-testid="turn-undo-modal"
                    >
                        <h2 id="turn-undo-title">{move || t(locale.get(), "undo.title")}</h2>
                        <div class="turn-undo-scroll">
                            <p class="turn-undo-body">{move || t(locale.get(), "undo.body")}</p>
                            <div class="turn-undo-warning">
                                {move || t(locale.get(), "undo.binary_warning")}
                            </div>
                            {(!has_text_changes).then(|| view! {
                                <p class="turn-undo-empty">
                                    {move || t(locale.get(), "undo.no_text_changes")}
                                </p>
                            })}
                            {(!restore_files.is_empty()).then(|| view! {
                                <section class="turn-undo-section">
                                    <h3>{move || t(locale.get(), "undo.restore_files")}</h3>
                                    <ul>{restore_files.into_iter().map(|path| view! {
                                        <li><code>{path}</code></li>
                                    }).collect_view()}</ul>
                                </section>
                            })}
                            {(!remove_files.is_empty()).then(|| view! {
                                <section class="turn-undo-section">
                                    <h3>{move || t(locale.get(), "undo.remove_files")}</h3>
                                    <ul>{remove_files.into_iter().map(|path| view! {
                                        <li><code>{path}</code></li>
                                    }).collect_view()}</ul>
                                </section>
                            })}
                            {(!remove_artifacts.is_empty()).then(|| view! {
                                <section class="turn-undo-section">
                                    <h3>{move || t(locale.get(), "undo.remove_artifacts")}</h3>
                                    <ul>{remove_artifacts.into_iter().map(|name| view! {
                                        <li><code>{name}</code></li>
                                    }).collect_view()}</ul>
                                </section>
                            })}
                            {(!unsupported_files.is_empty()).then(|| view! {
                                <section class="turn-undo-section unsupported">
                                    <h3>{move || t(locale.get(), "undo.unsupported_files")}</h3>
                                    <ul>{unsupported_files.into_iter().map(|path| view! {
                                        <li><code>{path}</code></li>
                                    }).collect_view()}</ul>
                                </section>
                            })}
                            {(!conflicts.is_empty()).then(|| view! {
                                <section class="turn-undo-section conflicts">
                                    <h3>{move || t(locale.get(), "undo.conflicts")}</h3>
                                    <ul>{conflicts.into_iter().map(|path| view! {
                                        <li><code>{path}</code></li>
                                    }).collect_view()}</ul>
                                </section>
                            })}
                            {move || turn_undo_error.get().map(|error| view! {
                                <div class="turn-undo-error" role="alert">{error}</div>
                            })}
                        </div>
                        <div class="row">
                            <button
                                disabled=move || turn_undo_busy.get()
                                on:click=move |_| {
                                    turn_undo_dialog.set(None);
                                    turn_undo_error.set(None);
                                }
                            >
                                {move || t(locale.get(), "settings.cancel")}
                            </button>
                            <button
                                class="primary"
                                disabled=move || turn_undo_busy.get() || !can_confirm
                                on:click=move |_| on_confirm.call(())
                            >
                                {move || if turn_undo_busy.get() {
                                    t(locale.get(), "undo.working")
                                } else {
                                    t(locale.get(), "undo.confirm")
                                }}
                            </button>
                        </div>
                    </div>
                </div>
            }.into_view()
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct EditConfirmOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) edit_confirm: RwSignal<Option<usize>>,
    pub(crate) can_branch: Signal<bool>,
}

#[component]
pub(crate) fn EditConfirmOverlay(
    state: EditConfirmOverlayState,
    on_branch: Callback<usize>,
    on_rewind: Callback<usize>,
) -> impl IntoView {
    let EditConfirmOverlayState {
        locale,
        edit_confirm,
        can_branch,
    } = state;
    view! {
        {move || edit_confirm.get().map(|ui_index| {
            view! {
                <div class="overlay">
                    <div
                        class="modal confirm-modal"
                        role="dialog"
                        aria-modal="true"
                        aria-labelledby="edit-confirm-title"
                        data-testid="edit-confirm-modal"
                    >
                        <h2 id="edit-confirm-title">{move || t(locale.get(), "msg.edit_confirm_title")}</h2>
                        <div class="hint">{move || t(locale.get(), "msg.edit_confirm_hint")}</div>
                        <div class="row">
                            <button on:click=move |_| edit_confirm.set(None)>
                                {move || t(locale.get(), "settings.cancel")}
                            </button>
                            {move || can_branch.get().then(|| view! { <button on:click=move |_| {
                                edit_confirm.set(None);
                                on_branch.call(ui_index);
                            }>
                                {move || t(locale.get(), "msg.branch")}
                            </button> })}
                            <button class="primary" class:danger=true on:click=move |_| {
                                edit_confirm.set(None);
                                on_rewind.call(ui_index);
                            }>
                                {move || t(locale.get(), "msg.edit")}
                            </button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ModelSwitchConfirmOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) model_switch_confirm: RwSignal<Option<(String, String, bool)>>,
}

#[component]
pub(crate) fn ModelSwitchConfirmOverlay(
    state: ModelSwitchConfirmOverlayState,
    on_switch: Callback<(String, bool)>,
) -> impl IntoView {
    let ModelSwitchConfirmOverlayState {
        locale,
        model_switch_confirm,
    } = state;
    view! {
        {move || model_switch_confirm.get().map(|(id, label, ignores_images)| {
            let switch_yes = on_switch.clone();
            let yes_id = id.clone();
            let dont_ask_again = create_rw_signal(false);
            let hint_key = if ignores_images {
                "models.switch_confirm_image_hint"
            } else {
                "models.switch_confirm_hint"
            };
            let yes_key = if ignores_images {
                "models.switch_ignore_images"
            } else {
                "models.switch_yes"
            };
            view! {
                <div class="overlay" data-testid="model-switch-confirm-overlay">
                    <div class="modal confirm-modal model-switch-confirm" data-testid="model-switch-confirm">
                        <h2>{move || t(locale.get(), "models.switch_confirm_title")}</h2>
                        <div class="hint">{move || tf(
                            locale.get(),
                            hint_key,
                            &[("model", &label)],
                        )}</div>
                        <label class="confirm-option" data-testid="model-switch-dont-ask">
                            <input type="checkbox"
                                prop:checked=move || dont_ask_again.get()
                                on:change=move |ev| dont_ask_again.set(event_target_checked(&ev)) />
                            <span>{move || t(locale.get(), "models.switch_dont_ask")}</span>
                        </label>
                        <div class="row">
                            <button type="button" on:click=move |_| model_switch_confirm.set(None)>
                                {move || t(locale.get(), "models.switch_no")}
                            </button>
                            <button type="button" class="primary" on:click=move |_| {
                                let skip_future = dont_ask_again.get_untracked();
                                model_switch_confirm.set(None);
                                switch_yes.call((yes_id.clone(), skip_future));
                            }>{move || t(locale.get(), yes_key)}</button>
                        </div>
                    </div>
                </div>
            }.into_view()
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ProjSettingsOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) show_proj_settings: RwSignal<bool>,
    pub(crate) proj_settings: RwSignal<ProjectSettings>,
    pub(crate) proj_settings_busy: RwSignal<bool>,
}

#[component]
pub(crate) fn ProjSettingsOverlay(
    state: ProjSettingsOverlayState,
    on_save: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let ProjSettingsOverlayState {
        locale,
        show_proj_settings,
        proj_settings,
        proj_settings_busy,
    } = state;
    // Retention is stored per project and saved immediately on change; empty
    // means the automatic sweep stays off.
    let retention_succeeded = create_rw_signal(String::new());
    let retention_failed = create_rw_signal(String::new());
    let retention_orphan = create_rw_signal(String::new());
    create_effect(move |_| {
        if !show_proj_settings.get() {
            return;
        }
        spawn_local(async move {
            if let Ok(value) = invoke_checked(
                "get_project_run_retention",
                wasm_bindgen::JsValue::UNDEFINED,
            )
            .await
            {
                if let Ok(retention) = serde_wasm_bindgen::from_value::<serde_json::Value>(value) {
                    let field = |key: &str| {
                        retention
                            .get(key)
                            .and_then(|value| value.as_i64())
                            .map(|days| days.to_string())
                            .unwrap_or_default()
                    };
                    retention_succeeded.set(field("run_retention_days"));
                    retention_failed.set(field("failed_run_retention_days"));
                    retention_orphan.set(field("orphan_file_retention_days"));
                }
            }
        });
    });
    let save_retention = move || {
        let parse = |value: &str| value.trim().parse::<i64>().ok();
        let args = to_value(&serde_json::json!({
            "runRetentionDays": parse(&retention_succeeded.get_untracked()),
            "failedRunRetentionDays": parse(&retention_failed.get_untracked()),
            "orphanFileRetentionDays": parse(&retention_orphan.get_untracked()),
        }))
        .unwrap();
        spawn_local(async move {
            if let Err(error) = invoke_checked("set_project_run_retention", args).await {
                show_toast(&localize_backend(
                    locale.get_untracked(),
                    &js_error_text(error),
                ));
            }
        });
    };
    view! {
        {move || show_proj_settings.get().then(|| view! {
            <div class="overlay">
                <div class="modal proj-settings-modal">
                    <div class="ps-head">
                        <h2>{move || t(locale.get(), "proj_settings.title")}</h2>
                        <button type="button" class="ps-close"
                            title=move || t(locale.get(), "settings.cancel")
                            on:click=move |_| show_proj_settings.set(false)>{compose_icon("close")}</button>
                    </div>
                    <label>
                        <span class="ps-label">{move || t(locale.get(), "proj_settings.name")}</span>
                        <input prop:value=move || proj_settings.get().name
                            on:input=move |ev| { let v = event_target_value(&ev); proj_settings.update(|s| s.name = v); } />
                    </label>
                    <label>
                        <span class="ps-label">{move || t(locale.get(), "proj_settings.description")}</span>
                        <span class="ps-hint">{move || t(locale.get(), "proj_settings.description_hint")}</span>
                        <textarea class="ps-textarea" rows="2"
                            prop:value=move || proj_settings.get().description
                            on:input=move |ev| { let v = event_target_value(&ev); proj_settings.update(|s| s.description = v); }></textarea>
                    </label>
                    <label>
                        <span class="ps-label">{move || t(locale.get(), "proj_settings.agent_context")}</span>
                        <span class="ps-hint">{move || t(locale.get(), "proj_settings.agent_context_hint")}</span>
                        <textarea class="ps-textarea ps-ctx" rows="8"
                            prop:value=move || proj_settings.get().agent_context
                            on:input=move |ev| { let v = event_target_value(&ev); proj_settings.update(|s| s.agent_context = v); }></textarea>
                    </label>
                    <label>
                        <span class="ps-label">{move || t(locale.get(), "proj_settings.retention")}</span>
                        <span class="ps-hint">{move || t(locale.get(), "proj_settings.retention_hint")}</span>
                        <div class="ps-retention-row">
                            <div class="ps-retention-item">
                                <span>{move || t(locale.get(), "proj_settings.retention_succeeded")}</span>
                                <span class="ps-retention-field">
                                    <input type="number" min="1" max="3650" inputmode="numeric"
                                        class="ps-retention" data-testid="retention-succeeded"
                                        placeholder=move || t(locale.get(), "proj_settings.retention_off")
                                        prop:value=move || retention_succeeded.get()
                                        on:input=move |ev| retention_succeeded.set(event_target_value(&ev))
                                        on:change=move |_| save_retention() />
                                    <span class="ps-retention-unit">{move || t(locale.get(), "proj_settings.retention_days_unit")}</span>
                                </span>
                            </div>
                            <div class="ps-retention-item">
                                <span>{move || t(locale.get(), "proj_settings.retention_failed")}</span>
                                <span class="ps-retention-field">
                                    <input type="number" min="1" max="3650" inputmode="numeric"
                                        class="ps-retention" data-testid="retention-failed"
                                        placeholder=move || t(locale.get(), "proj_settings.retention_off")
                                        prop:value=move || retention_failed.get()
                                        on:input=move |ev| retention_failed.set(event_target_value(&ev))
                                        on:change=move |_| save_retention() />
                                    <span class="ps-retention-unit">{move || t(locale.get(), "proj_settings.retention_days_unit")}</span>
                                </span>
                            </div>
                            <div class="ps-retention-item">
                                <span>{move || t(locale.get(), "proj_settings.retention_orphan")}</span>
                                <span class="ps-retention-field">
                                    <input type="number" min="1" max="3650" inputmode="numeric"
                                        class="ps-retention" data-testid="retention-orphan"
                                        placeholder=move || t(locale.get(), "proj_settings.retention_off")
                                        prop:value=move || retention_orphan.get()
                                        on:input=move |ev| retention_orphan.set(event_target_value(&ev))
                                        on:change=move |_| save_retention() />
                                    <span class="ps-retention-unit">{move || t(locale.get(), "proj_settings.retention_days_unit")}</span>
                                </span>
                            </div>
                        </div>
                    </label>
                    <div class="row">
                        <button type="button" disabled=move || proj_settings_busy.get()
                            on:click=move |_| show_proj_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button type="button" class="primary"
                            disabled=move || proj_settings_busy.get() || proj_settings.get().name.trim().is_empty()
                            on:click=move |ev| on_save.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                    </div>
                </div>
            </div>
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct BranchMergeOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) open: RwSignal<Option<String>>,
    pub(crate) preview: RwSignal<Option<SessionBranchMergePreview>>,
    pub(crate) draft: RwSignal<String>,
    pub(crate) busy: RwSignal<bool>,
    pub(crate) error: RwSignal<Option<String>>,
    pub(crate) guidance_open: RwSignal<bool>,
    pub(crate) guidance: RwSignal<String>,
}

#[component]
pub(crate) fn BranchMergeDetailOverlay(
    locale: RwSignal<Locale>,
    detail: RwSignal<Option<(String, String)>>,
) -> impl IntoView {
    view! {
        {move || detail.get().map(|(title, text)| {
            view! {
                <div class="overlay branch-merge-detail-overlay" data-testid="branch-merge-detail-overlay">
                    <div class="modal artifact-modal branch-merge-detail-modal" role="dialog" aria-modal="true">
                        <div class="am-head branch-merge-detail-head">
                            <span class="am-name">{title}</span>
                            <span class="branch-merge-detail-kind">{t(locale.get(), "branch.merged_result")}</span>
                            <div class="spacer"></div>
                            <button type="button" class="icon-btn"
                                title=move || t(locale.get(), "right.close")
                                aria-label=move || t(locale.get(), "right.close")
                                on:click=move |_| detail.set(None)>{compose_icon("close")}</button>
                        </div>
                        <div class="am-figure branch-merge-detail-preview">
                            <article class="rp-heavy md branch-merge-detail-content"
                                inner_html=md_document_to_html(&text)></article>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

fn branch_message_role(locale: Locale, role: &str) -> String {
    t(
        locale,
        match role {
            "user" => "branch.role_user",
            "assistant" => "branch.role_assistant",
            "tool" => "branch.role_tool",
            _ => "branch.role_system",
        },
    )
}

#[component]
pub(crate) fn BranchMergeOverlay(
    state: BranchMergeOverlayState,
    on_merge: Callback<(String, String, String)>,
    on_generate: Callback<(String, String, Option<String>, Option<String>)>,
) -> impl IntoView {
    let BranchMergeOverlayState {
        locale,
        open,
        preview,
        draft,
        busy,
        error,
        guidance_open,
        guidance,
    } = state;
    window_capture_escape(move || {
        if !guidance_open.get_untracked() {
            return false;
        }
        guidance_open.set(false);
        guidance.set(String::new());
        true
    });

    view! {
        {move || open.get().map(|_| {
            let current = preview.get();
            let close = move |_| {
                if busy.get_untracked() {
                    return;
                }
                open.set(None);
                preview.set(None);
                draft.set(String::new());
                error.set(None);
                guidance_open.set(false);
                guidance.set(String::new());
            };
            view! {
                <div class="overlay branch-comparison-overlay" data-testid="branch-merge-overlay">
                    <div class="modal exploration-diff-modal branch-comparison-modal" role="dialog" aria-modal="true">
                        <div class="ps-head">
                            <div>
                                <h2>{t(locale.get(), "branch.merge_title")}</h2>
                                <span class="exploration-modal-status">{t(locale.get(), "branch.merge_hint")}</span>
                            </div>
                            <button type="button" class="ps-close" disabled=move || busy.get()
                                aria-label=move || t(locale.get(), "settings.cancel")
                                on:click=close>{compose_icon("close")}</button>
                        </div>
                        {move || error.get().map(|message| view! {
                            <div class="exploration-error" role="alert">{message}</div>
                        })}
                        {if let Some(current) = current {
                            let guard_hash = current.guard_hash.clone();
                            let branch_id = current.branch_session_id.clone();
                            let regenerate_branch_id = branch_id.clone();
                            let regenerate_guard_hash = guard_hash.clone();
                            let merge_branch_id = branch_id.clone();
                            let merge_guard_hash = guard_hash.clone();
                            let guided_branch_id = branch_id.clone();
                            let guided_guard_hash = guard_hash.clone();
                            let messages = current.messages.into_iter().map(|message| view! {
                                <div class=format!("branch-delta-message {}", message.role)>
                                    <span>{branch_message_role(locale.get(), &message.role)}</span>
                                    <div>{message.text}</div>
                                </div>
                            }).collect_view();
                            view! {
                                <div class="branch-comparison-meta">
                                    <strong>{current.branch_title}</strong>
                                    <span>{tf(locale.get(), "branch.checkpoint", &[("n", &(current.checkpoint_user_index + 1).to_string())])}</span>
                                    <span>{tf(locale.get(), "branch.new_messages", &[("n", &current.new_message_count.to_string())])}</span>
                                </div>
                                <div class="branch-merge-body">
                                    <section class="branch-merge-delta" data-testid="branch-merge-delta">
                                        <strong>{t(locale.get(), "branch.branch_work")}</strong>
                                        <div class="branch-candidate-messages">{messages}</div>
                                    </section>
                                    <label class="branch-merge-editor">
                                        <strong>{t(locale.get(), "branch.summary_draft")}</strong>
                                        <span>{t(locale.get(), "branch.summary_edit_hint")}</span>
                                        <textarea rows="12" prop:value=move || draft.get()
                                            on:input=move |event| draft.set(event_target_value(&event))></textarea>
                                    </label>
                                </div>
                                <div class="row exploration-actions">
                                    <button type="button" disabled=move || busy.get() on:click=close>
                                        {move || t(locale.get(), "settings.cancel")}
                                    </button>
                                    <button type="button" data-testid="branch-regenerate"
                                        disabled=move || busy.get()
                                        on:click=move |_| on_generate.call((regenerate_branch_id.clone(), regenerate_guard_hash.clone(), None, None))>
                                        {t(locale.get(), "branch.regenerate")}
                                    </button>
                                    <button type="button" data-testid="branch-guided-generate"
                                        disabled=move || busy.get() || draft.get().trim().is_empty()
                                        on:click=move |_| {
                                            guidance.set(String::new());
                                            guidance_open.set(true);
                                        }>
                                        {t(locale.get(), "branch.guided_generate")}
                                    </button>
                                    <span class="spacer"></span>
                                    <button type="button" class="primary" data-testid="branch-merge-action"
                                        disabled=move || busy.get() || draft.get().trim().is_empty()
                                        on:click=move |_| on_merge.call((merge_branch_id.clone(), merge_guard_hash.clone(), draft.get_untracked()))>
                                        {move || if busy.get() { t(locale.get(), "branch.merging") } else { t(locale.get(), "branch.merge") }}
                                    </button>
                                </div>
                                {move || guidance_open.get().then(|| {
                                    let branch_id = guided_branch_id.clone();
                                    let guard_hash = guided_guard_hash.clone();
                                    view! {
                                        <div class="overlay exploration-confirm-overlay" data-testid="branch-guidance-overlay">
                                            <div class="modal branch-guidance-modal" role="dialog" aria-modal="true">
                                                <div class="ps-head">
                                                    <div>
                                                        <h2>{t(locale.get(), "branch.guidance_title")}</h2>
                                                        <span class="exploration-modal-status">{t(locale.get(), "branch.guidance_hint")}</span>
                                                    </div>
                                                    <button type="button" class="ps-close" disabled=move || busy.get()
                                                        aria-label=move || t(locale.get(), "settings.cancel")
                                                        on:click=move |_| {
                                                            guidance_open.set(false);
                                                            guidance.set(String::new());
                                                        }>{compose_icon("close")}</button>
                                                </div>
                                                <label class="branch-guidance-editor">
                                                    <strong>{t(locale.get(), "branch.guidance_label")}</strong>
                                                    <textarea rows="7" maxlength="8000"
                                                        placeholder=move || t(locale.get(), "branch.guidance_placeholder")
                                                        prop:value=move || guidance.get()
                                                        on:input=move |event| guidance.set(event_target_value(&event))></textarea>
                                                </label>
                                                <div class="row exploration-actions">
                                                    <button type="button" disabled=move || busy.get() on:click=move |_| {
                                                        guidance_open.set(false);
                                                        guidance.set(String::new());
                                                    }>{t(locale.get(), "settings.cancel")}</button>
                                                    <span class="spacer"></span>
                                                    <button type="button" class="primary" data-testid="branch-guidance-action"
                                                        disabled=move || busy.get() || guidance.get().trim().is_empty()
                                                        on:click=move |_| {
                                                            let user_guidance = guidance.get_untracked();
                                                            let current_version = draft.get_untracked();
                                                            guidance_open.set(false);
                                                            on_generate.call((branch_id.clone(), guard_hash.clone(), Some(current_version), Some(user_guidance)));
                                                        }>
                                                        {move || if busy.get() { t(locale.get(), "branch.generating") } else { t(locale.get(), "branch.generate") }}
                                                    </button>
                                                </div>
                                            </div>
                                        </div>
                                    }
                                })}
                            }.into_view()
                        } else {
                            view! { <div class="exploration-loading">{move || t(locale.get(), "loading")}</div> }.into_view()
                        }}
                    </div>
                </div>
            }
        })}
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExplorationOverlay {
    Start {
        source_frame_id: String,
        turn_index: usize,
    },
    Preview {
        exploration_id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExplorationDiffTab {
    Files,
    Artifacts,
    Runs,
    Decisions,
    ExternalEffects,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExplorationConfirm {
    Promote {
        exploration_id: String,
        expected_guard_hash: String,
    },
    Discard {
        exploration_id: String,
    },
    ManualResolution {
        exploration_id: String,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct ExplorationOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) overlay: RwSignal<Option<ExplorationOverlay>>,
    pub(crate) name: RwSignal<String>,
    pub(crate) preview: RwSignal<Option<ExplorationPromotionPreview>>,
    pub(crate) busy: RwSignal<bool>,
    pub(crate) error: RwSignal<Option<String>>,
}

fn exploration_status(locale: Locale, status: &str) -> String {
    let key = match status {
        "creating" => "exploration.status_creating",
        "active" => "exploration.status_active",
        "promoting" => "exploration.status_promoting",
        _ => "exploration.status_failed",
    };
    t(locale, key)
}

fn exploration_empty(locale: Locale) -> View {
    view! { <div class="exploration-diff-empty">{t(locale, "exploration.diff_empty")}</div> }
        .into_view()
}

fn exploration_blocker_message(locale: Locale, blocker: &PromotionBlocker) -> String {
    let key = match blocker.code.as_str() {
        "MainlineAdvanced" => "exploration.blocker_mainline_advanced",
        "ExternalReferenceChanged" => "exploration.blocker_external_reference_changed",
        "ExplorationBusy" => "exploration.blocker_busy",
        "ExplorationNotPromotable" => "exploration.blocker_not_promotable",
        _ => return blocker.message.clone(),
    };
    t(locale, key)
}

fn exploration_diff_body(
    locale: Locale,
    tab: ExplorationDiffTab,
    preview: &ExplorationPromotionPreview,
) -> View {
    match tab {
        ExplorationDiffTab::Files => {
            let branch_rows = preview
                .diff
                .files
                .iter()
                .map(|file| {
                    view! {
                        <div class="exploration-diff-row">
                            <span class=format!("exploration-delta-kind {}", file.kind)>{file.kind.clone()}</span>
                            <code>{file.path.clone()}</code>
                        </div>
                    }
                })
                .collect_view();
            let mainline_rows = preview
                .mainline_changes
                .files
                .iter()
                .map(|file| {
                    view! {
                        <div class="exploration-diff-row mainline">
                            <span class=format!("exploration-delta-kind {}", file.kind)>{file.kind.clone()}</span>
                            <code>{file.path.clone()}</code>
                        </div>
                    }
                })
                .collect_view();
            view! {
                <section class="exploration-diff-section">
                    <h3>{t(locale, "exploration.diff_branch_changes")}</h3>
                    {if preview.diff.files.is_empty() { exploration_empty(locale) } else { branch_rows.into_view() }}
                </section>
                {(!preview.mainline_changes.files.is_empty()).then(|| view! {
                    <section class="exploration-diff-section conflict">
                        <h3>{t(locale, "exploration.diff_mainline_changes")}</h3>
                        {mainline_rows}
                    </section>
                })}
            }
            .into_view()
        }
        ExplorationDiffTab::Artifacts => {
            if preview.diff.artifacts.is_empty() {
                return exploration_empty(locale);
            }
            preview
                .diff
                .artifacts
                .iter()
                .map(|artifact| {
                    view! {
                        <div class="exploration-diff-row stacked">
                            <strong>{artifact.logical_key.clone()}</strong>
                            <code>{artifact.after_version_id.clone()}</code>
                        </div>
                    }
                })
                .collect_view()
                .into_view()
        }
        ExplorationDiffTab::Runs => {
            if preview.diff.runs.is_empty() {
                return exploration_empty(locale);
            }
            preview
                .diff
                .runs
                .iter()
                .map(|run| {
                    view! {
                        <div class="exploration-diff-row stacked">
                            <strong>{run.title.clone()}</strong>
                            <span>{run.status.clone()}</span>
                        </div>
                    }
                })
                .collect_view()
                .into_view()
        }
        ExplorationDiffTab::Decisions => {
            if preview.diff.decisions.is_empty() && preview.diff.research_edges.is_empty() {
                return exploration_empty(locale);
            }
            let decisions = preview
                .diff
                .decisions
                .iter()
                .map(|decision| {
                    view! {
                        <div class="exploration-diff-row stacked">
                            <strong>{decision.title.clone()}</strong>
                            <span>{decision.kind.clone()}</span>
                        </div>
                    }
                })
                .collect_view();
            let edges = preview
                .diff
                .research_edges
                .iter()
                .map(|edge| {
                    view! {
                        <div class="exploration-diff-row stacked">
                            <strong>{edge.relation.clone()}</strong>
                            <code>{format!("{} → {}", edge.source_id, edge.target_id)}</code>
                        </div>
                    }
                })
                .collect_view();
            view! { {decisions}{edges} }.into_view()
        }
        ExplorationDiffTab::ExternalEffects => {
            if preview.diff.external_effects.is_empty()
                && preview.diff.external_resources.is_empty()
            {
                return exploration_empty(locale);
            }
            let effects = preview
                .diff
                .external_effects
                .iter()
                .map(|effect| {
                    view! {
                        <div class="exploration-diff-row stacked external">
                            <strong>{effect.target_summary.clone()}</strong>
                            <span>{format!("{} · {}", effect.effect_kind, effect.recoverability)}</span>
                        </div>
                    }
                })
                .collect_view();
            let resources = preview
                .diff
                .external_resources
                .iter()
                .map(|resource| {
                    view! {
                        <div class="exploration-diff-row stacked">
                            <strong>{resource.kind.clone()}</strong>
                            <code>{resource.uri.clone()}</code>
                        </div>
                    }
                })
                .collect_view();
            view! { {effects}{resources} }.into_view()
        }
    }
}

#[component]
pub(crate) fn ExplorationOverlayView(
    state: ExplorationOverlayState,
    on_start: Callback<(String, usize, String)>,
    on_promote: Callback<(String, String)>,
    on_discard: Callback<String>,
    on_open_manual_resolution: Callback<String>,
    on_finish_manual_resolution: Callback<String>,
) -> impl IntoView {
    let ExplorationOverlayState {
        locale,
        overlay,
        name,
        preview,
        busy,
        error,
    } = state;
    let tab = create_rw_signal(ExplorationDiffTab::Files);
    let confirm = create_rw_signal::<Option<ExplorationConfirm>>(None);
    window_capture_escape(move || {
        if confirm.get_untracked().is_none() {
            return false;
        }
        confirm.set(None);
        true
    });

    view! {
        {move || overlay.get().map(|mode| match mode {
            ExplorationOverlay::Start {
                source_frame_id,
                turn_index,
            } => {
                let source_for_start = source_frame_id.clone();
                view! {
                    <div class="overlay exploration-overlay" data-testid="exploration-start-overlay">
                        <div class="modal exploration-start-modal" role="dialog" aria-modal="true">
                            <div class="ps-head">
                                <h2>{t(locale.get(), "exploration.start_title")}</h2>
                                <button type="button" class="ps-close" disabled=move || busy.get()
                                    aria-label=move || t(locale.get(), "settings.cancel")
                                    on:click=move |_| overlay.set(None)>{compose_icon("close")}</button>
                            </div>
                            <p class="exploration-modal-copy">{t(locale.get(), "exploration.start_hint")}</p>
                            <label>
                                {t(locale.get(), "exploration.name")}
                                <input data-testid="exploration-name" prop:value=move || name.get()
                                    disabled=move || busy.get()
                                    on:input=move |event| name.set(event_target_value(&event)) />
                            </label>
                            {move || error.get().map(|message| view! {
                                <div class="exploration-error" role="alert">{message}</div>
                            })}
                            <div class="row">
                                <button type="button" disabled=move || busy.get()
                                    on:click=move |_| overlay.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                                <button type="button" class="primary" data-testid="exploration-create"
                                    disabled=move || busy.get() || name.get().trim().is_empty()
                                    on:click=move |_| on_start.call((source_for_start.clone(), turn_index, name.get_untracked()))>
                                    {move || if busy.get() { t(locale.get(), "loading") } else { t(locale.get(), "exploration.create") }}
                                </button>
                            </div>
                        </div>
                    </div>
                }.into_view()
            }
            ExplorationOverlay::Preview { exploration_id } => {
                let current = preview.get();
                let id_for_discard = exploration_id.clone();
                view! {
                    <div class="overlay exploration-overlay" data-testid="exploration-diff-overlay">
                        <div class="modal exploration-diff-modal" role="dialog" aria-modal="true">
                            <div class="ps-head">
                                <div>
                                    <h2>{current.as_ref().map(|value| value.exploration.name.clone()).unwrap_or_else(|| t(locale.get(), "exploration.diff_title"))}</h2>
                                    {current.as_ref().map(|value| view! {
                                        <span class="exploration-modal-status">{exploration_status(locale.get(), &value.exploration.status)}</span>
                                    })}
                                </div>
                                <button type="button" class="ps-close" disabled=move || busy.get()
                                    aria-label=move || t(locale.get(), "settings.cancel")
                                    on:click=move |_| overlay.set(None)>{compose_icon("close")}</button>
                            </div>
                            {move || error.get().map(|message| view! {
                                <div class="exploration-error" role="alert">{message}</div>
                            })}
                            {if let Some(current) = current {
                                let eligible = current.eligibility.eligible;
                                let status = current.exploration.status.clone();
                                let promote_id = current.exploration.id.clone();
                                let promote_guard = current.eligibility.expected_guard_hash.clone();
                                let blockers = current.eligibility.reasons.clone();
                                let manual_resolution_available = current.eligibility.manual_resolution_available;
                                let manual_open_id = current.exploration.id.clone();
                                let manual_finish_id = current.exploration.id.clone();
                                view! {
                                    {(!eligible).then(|| view! {
                                        <div class="exploration-eligibility blocked" data-testid="exploration-promotion-blocked">
                                            <strong>{t(locale.get(), "exploration.cannot_promote")}</strong>
                                            {blockers.into_iter().map(|reason| {
                                                let code = reason.code.clone();
                                                let message = exploration_blocker_message(locale.get(), &reason);
                                                view! { <span data-blocker-code=code>{message}</span> }
                                            }).collect_view()}
                                        </div>
                                    })}
                                    {manual_resolution_available.then(|| view! {
                                        <section class="exploration-manual-resolution" data-testid="exploration-manual-resolution">
                                            <strong>{t(locale.get(), "exploration.manual_title")}</strong>
                                            <span>{t(locale.get(), "exploration.manual_body")}</span>
                                            <span class="warning">{t(locale.get(), "exploration.manual_warning")}</span>
                                            <div class="row">
                                                <button type="button" disabled=move || busy.get()
                                                    data-testid="exploration-open-manual-folders"
                                                    on:click=move |_| on_open_manual_resolution.call(manual_open_id.clone())>
                                                    {move || t(locale.get(), "exploration.manual_open_folders")}
                                                </button>
                                                <button type="button" class="primary" disabled=move || busy.get()
                                                    data-testid="exploration-finish-manual"
                                                    on:click=move |_| confirm.set(Some(ExplorationConfirm::ManualResolution {
                                                        exploration_id: manual_finish_id.clone(),
                                                    }))>
                                                    {move || t(locale.get(), "exploration.manual_finish")}
                                                </button>
                                            </div>
                                        </section>
                                    })}
                                    <div class="exploration-tabs" role="tablist">
                                        {[
                                            (ExplorationDiffTab::Files, "exploration.tab_files", current.diff.files.len()),
                                            (ExplorationDiffTab::Artifacts, "exploration.tab_artifacts", current.diff.artifacts.len()),
                                            (ExplorationDiffTab::Runs, "exploration.tab_runs", current.diff.runs.len()),
                                            (ExplorationDiffTab::Decisions, "exploration.tab_decisions", current.diff.decisions.len() + current.diff.research_edges.len()),
                                            (ExplorationDiffTab::ExternalEffects, "exploration.tab_effects", current.diff.external_effects.len() + current.diff.external_resources.len()),
                                        ].into_iter().map(|(value, key, count)| view! {
                                            <button type="button" role="tab" class:active=move || tab.get() == value
                                                aria-selected=move || (tab.get() == value).to_string()
                                                on:click=move |_| tab.set(value)>{format!("{} {count}", t(locale.get(), key))}</button>
                                        }).collect_view()}
                                    </div>
                                    <div class="exploration-diff-body" data-testid="exploration-diff-body"
                                        data-exploration-id=current.diff.exploration_id.clone()>
                                        {move || exploration_diff_body(locale.get(), tab.get(), &current)}
                                    </div>
                                    <div class="row exploration-actions">
                                        {(status == "active").then(|| view! {
                                            <button type="button" class="danger-text" disabled=move || busy.get()
                                                on:click=move |_| confirm.set(Some(ExplorationConfirm::Discard { exploration_id: id_for_discard.clone() }))>
                                                {move || t(locale.get(), "exploration.discard")}
                                            </button>
                                        })}
                                        <span class="spacer"></span>
                                        <button type="button" class="primary" data-testid="exploration-promote"
                                            disabled=move || busy.get() || !eligible || status != "active"
                                            on:click=move |_| confirm.set(Some(ExplorationConfirm::Promote {
                                                exploration_id: promote_id.clone(),
                                                expected_guard_hash: promote_guard.clone(),
                                            }))>{move || t(locale.get(), "exploration.promote")}</button>
                                    </div>
                                }.into_view()
                            } else {
                                view! { <div class="exploration-loading">{move || t(locale.get(), "loading")}</div> }.into_view()
                            }}
                        </div>
                    </div>
                }.into_view()
            }
        })}
        {move || confirm.get().map(|choice| {
            let choice_for_confirm = choice.clone();
            let promote = matches!(choice, ExplorationConfirm::Promote { .. });
            let manual = matches!(choice, ExplorationConfirm::ManualResolution { .. });
            let title_key = if promote {
                "exploration.promote_confirm_title"
            } else if manual {
                "exploration.manual_confirm_title"
            } else {
                "exploration.discard_confirm_title"
            };
            let body_key = if promote {
                "exploration.promote_confirm_body"
            } else if manual {
                "exploration.manual_confirm_body"
            } else {
                "exploration.discard_confirm_body"
            };
            let action_key = if promote {
                "exploration.promote"
            } else if manual {
                "exploration.manual_finish"
            } else {
                "exploration.discard"
            };
            view! {
                <div class="overlay exploration-confirm-overlay" data-testid="exploration-confirm-overlay">
                    <div class="modal confirm-modal exploration-confirm-modal" role="alertdialog" aria-modal="true">
                        <h2>{t(locale.get(), title_key)}</h2>
                        <div class="hint">{t(locale.get(), body_key)}</div>
                        <div class="row">
                            <button type="button" on:click=move |_| confirm.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                            <button type="button" class="primary" class:danger=!promote data-testid="exploration-confirm-action"
                                on:click=move |_| {
                                    confirm.set(None);
                                    match choice_for_confirm.clone() {
                                        ExplorationConfirm::Promote { exploration_id, expected_guard_hash } => on_promote.call((exploration_id, expected_guard_hash)),
                                        ExplorationConfirm::Discard { exploration_id } => on_discard.call(exploration_id),
                                        ExplorationConfirm::ManualResolution { exploration_id } => on_finish_manual_resolution.call(exploration_id),
                                    }
                                }>{t(locale.get(), action_key)}</button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}
