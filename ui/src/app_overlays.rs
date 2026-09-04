use crate::app_support::{
    classify_ssh_failure, js_error_text, refresh_execution_contexts, refresh_remote_dir,
    show_probe_stopped_toast, show_toast, show_warning_toast, ssh_connectivity_gap,
    ssh_context_known_good, ssh_fail_cause_keys, AvailableUpdate, SshCheckPhase,
    SshConnectivityModal, SshFailKind, UpdateCheckModal,
};
use crate::bindings::{download_app_update, invoke, invoke_checked, open_external_url};
use crate::dto::*;
use crate::i18n::{localize_backend, t, tf, Locale};
use crate::text::{event_target_checked, event_target_value, format_bytes, md_to_html};
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::collections::HashSet;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

fn project_transfer_stage_label(locale: Locale, stage: &str) -> String {
    let key = match stage {
        "selecting_export_destination" => "projects.transfer.selecting_export_destination",
        "selecting_import_destination" => "projects.transfer.selecting_import_destination",
        "selecting_archive" => "projects.transfer.selecting_archive",
        "preparing" => "projects.transfer.preparing",
        "scanning" => "projects.transfer.scanning",
        "writing" => "projects.transfer.writing",
        "validating" => "projects.transfer.validating",
        "publishing" => "projects.transfer.publishing",
        "reading" => "projects.transfer.reading",
        "extracting" => "projects.transfer.extracting",
        "registering" => "projects.transfer.registering",
        _ => "projects.transfer.preparing",
    };
    t(locale, key)
}

#[derive(Clone, Copy)]
pub(crate) struct TurnMemoryOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) proposal: RwSignal<Option<TurnMemoryProposal>>,
    pub(crate) editor: RwSignal<String>,
    pub(crate) scope: RwSignal<String>,
    pub(crate) replace_id: RwSignal<String>,
    pub(crate) busy: RwSignal<bool>,
    pub(crate) error: RwSignal<Option<String>>,
}

#[component]
pub(crate) fn TurnMemoryOverlay(
    state: TurnMemoryOverlayState,
    on_confirm: Callback<()>,
) -> impl IntoView {
    let TurnMemoryOverlayState {
        locale,
        proposal,
        editor,
        scope,
        replace_id,
        busy,
        error,
    } = state;
    view! {
        {move || proposal.get().map(|draft| {
            let failure_analysis = draft.trigger == "tool_failures";
            let hint = if failure_analysis {
                tf(
                    locale.get(),
                    "memory.proposal.failure_hint",
                    &[
                        ("failed", &draft.failed_tool_calls.to_string()),
                        ("total", &draft.tool_calls.to_string()),
                        ("rate", &format!("{:.1}", draft.failure_rate)),
                    ],
                )
            } else if draft.trigger == "explicit" {
                t(locale.get(), "memory.proposal.explicit_hint")
            } else {
                t(locale.get(), "memory.proposal.manual_hint")
            };
            let global_memories = draft.global_memories.clone();
            view! {
                <div class="overlay turn-memory-overlay" data-testid="turn-memory-overlay">
                    <div class="modal turn-memory-modal" role="dialog" aria-modal="true"
                        aria-labelledby="turn-memory-title">
                        <h2 id="turn-memory-title">{move || t(
                            locale.get(),
                            if failure_analysis {
                                "memory.proposal.failure_title"
                            } else {
                                "memory.proposal.title"
                            },
                        )}</h2>
                        <p class="hint">{hint}</p>
                        <label class="turn-memory-field">
                            <span>{move || t(locale.get(), "memory.proposal.scope")}</span>
                            <select data-testid="turn-memory-scope"
                                prop:value=move || scope.get()
                                disabled=move || busy.get()
                                on:change=move |event| scope.set(event_target_value(&event))>
                                <option value="project" prop:selected=move || scope.get() == "project">
                                    {move || t(locale.get(), "memory.proposal.scope_project")}
                                </option>
                                <option value="global" prop:selected=move || scope.get() == "global">
                                    {move || t(locale.get(), "memory.proposal.scope_global")}
                                </option>
                            </select>
                        </label>
                        {move || {
                            let memories = global_memories.clone();
                            (scope.get() == "global" && !memories.is_empty()).then(move || view! {
                                <label class="turn-memory-field">
                                    <span>{move || t(locale.get(), "memory.proposal.replace")}</span>
                                    <select data-testid="turn-memory-replace"
                                        prop:value=move || replace_id.get()
                                        disabled=move || busy.get()
                                        on:change=move |event| replace_id.set(event_target_value(&event))>
                                        <option value="">{move || t(locale.get(), "memory.proposal.add_new")}</option>
                                        <For each=move || memories.clone()
                                            key=|memory| memory.id.clone() let:memory>
                                            <option value=memory.id.clone()>
                                                {memory.content}
                                            </option>
                                        </For>
                                    </select>
                                    <span class="hint">{move || t(locale.get(), "memory.proposal.replace_hint")}</span>
                                </label>
                            })
                        }}
                        <label class="turn-memory-field">
                            <span>{move || t(locale.get(), "memory.proposal.content")}</span>
                            <textarea data-testid="turn-memory-content"
                                prop:value=move || editor.get()
                                disabled=move || busy.get()
                                on:input=move |event| editor.set(event_target_value(&event))></textarea>
                        </label>
                        {move || error.get().map(|message| view! {
                            <div class="settings-status fail" role="alert">{message}</div>
                        })}
                        <div class="row">
                            <button type="button" data-testid="turn-memory-cancel"
                                disabled=move || busy.get()
                                on:click=move |_| {
                                    proposal.set(None);
                                    editor.set(String::new());
                                    replace_id.set(String::new());
                                    error.set(None);
                                }>{move || t(locale.get(), "settings.cancel")}</button>
                            <button type="button" class="primary" data-testid="turn-memory-confirm"
                                disabled=move || busy.get() || editor.get().trim().is_empty()
                                on:click=move |_| on_confirm.call(())>
                                {move || if busy.get() {
                                    t(locale.get(), "memory.proposal.saving")
                                } else {
                                    t(locale.get(), "memory.proposal.confirm")
                                }}
                            </button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ProjectTransferOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) project_transfer: RwSignal<Option<ProjectTransferProgress>>,
}

#[component]
pub(crate) fn ProjectTransferOverlay(state: ProjectTransferOverlayState) -> impl IntoView {
    let ProjectTransferOverlayState {
        locale,
        project_transfer,
    } = state;
    view! {
        {move || project_transfer.get().map(|transfer| {
            let complete = transfer.is_complete();
            let failed = transfer.is_failed();
            let terminal = complete || failed;
            let title = if failed {
                t(
                    locale.get(),
                    if transfer.direction == "export" {
                        "projects.transfer.export_failed"
                    } else {
                        "projects.transfer.import_failed"
                    },
                )
            } else if complete {
                t(
                    locale.get(),
                    if transfer.direction == "export" {
                        "projects.transfer.export_complete"
                    } else {
                        "projects.transfer.import_complete"
                    },
                )
            } else if transfer.direction == "export" {
                t(locale.get(), "projects.transfer.export_title")
            } else {
                t(locale.get(), "projects.transfer.import_title")
            };
            let stage = if terminal {
                String::new()
            } else {
                project_transfer_stage_label(locale.get(), &transfer.stage)
            };
            let byte_progress = transfer.total_bytes.map(|total| {
                format!(
                    "{} / {}",
                    format_bytes(transfer.completed_bytes),
                    format_bytes(total),
                )
            });
            let file_progress = transfer.total_files.map(|total| {
                tf(
                    locale.get(),
                    "projects.transfer.files",
                    &[
                        ("done", &transfer.completed_files.to_string()),
                        ("total", &total.to_string()),
                    ],
                )
            });
            let detail = transfer.current_path.clone();
            let error = transfer.error.clone();
            let import_hint = (transfer.stage == "selecting_import_destination")
                .then(|| t(locale.get(), "projects.transfer.import_destination_hint"));
            let max = transfer.total_bytes.unwrap_or(1).to_string();
            let value = transfer.total_bytes.map(|_| transfer.completed_bytes.to_string());
            view! {
                <div class="project-transfer-card" class:failed=failed
                    data-testid="project-transfer-progress"
                    role="status" aria-live="polite">
                    <div class="project-transfer-head">
                        <h2>{title}</h2>
                        {terminal.then(|| view! {
                            <button type="button" class="project-transfer-dismiss"
                                aria-label=move || t(locale.get(), "projects.transfer.done")
                                on:click=move |_| project_transfer.set(None)>
                                <span aria-hidden="true">"×"</span>
                            </button>
                        })}
                    </div>
                    {(!terminal).then(|| view! {
                        <div class="project-transfer-progress">
                            <span class="project-transfer-stage">{stage}</span>
                            <progress max=max value=value></progress>
                            <div class="project-transfer-meta">
                                {byte_progress.map(|progress| view! { <span>{progress}</span> })}
                                {file_progress.map(|progress| view! { <span>{progress}</span> })}
                            </div>
                        </div>
                    })}
                    {import_hint.map(|hint| view! {
                        <div class="project-transfer-hint">{hint}</div>
                    })}
                    {detail.map(|path| view! {
                        <div class="project-transfer-path" title=path.clone()>{path}</div>
                    })}
                    {error.map(|message| view! {
                        <div class="project-transfer-error" role="alert">{message}</div>
                    })}
                </div>
            }
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ProjectExportPromptState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) prompt: RwSignal<Option<(String, String)>>,
}

#[component]
pub(crate) fn ProjectExportPrompt(
    state: ProjectExportPromptState,
    on_export_zip: Callback<String>,
    on_copy_path: Callback<String>,
) -> impl IntoView {
    let ProjectExportPromptState { locale, prompt } = state;
    view! {
        {move || prompt.get().map(|(project_id, workspace_dir)| {
            let export_id = project_id.clone();
            let copy_path = workspace_dir.clone();
            view! {
                <div class="overlay" data-testid="project-export-options">
                    <div class="modal confirm-modal project-export-options-modal"
                        role="dialog" aria-modal="true"
                        aria-label=move || t(locale.get(), "projects.export_options_title")>
                        <h2>{move || t(locale.get(), "projects.export_options_title")}</h2>
                        <p class="project-export-zip-hint">
                            {move || t(locale.get(), "projects.export_zip_hint")}
                        </p>
                        <div class="project-copy-folder-option">
                            <strong>{move || t(locale.get(), "projects.copy_folder_title")}</strong>
                            <p>{move || t(locale.get(), "projects.copy_folder_hint")}</p>
                            <code title=workspace_dir.clone()>{workspace_dir}</code>
                            <button type="button" class="btn-ghost"
                                on:click=move |_| on_copy_path.call(copy_path.clone())>
                                {move || t(locale.get(), "projects.copy_folder_path")}
                            </button>
                        </div>
                        <div class="row">
                            <button type="button" on:click=move |_| prompt.set(None)>
                                {move || t(locale.get(), "settings.cancel")}
                            </button>
                            <button type="button" class="primary" on:click=move |_| {
                                on_export_zip.call(export_id.clone());
                            }>
                                {move || t(locale.get(), "projects.export_zip")}
                            </button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

/// Confirm before handing a link from agent output to the system browser: the
/// destination is model-authored, so the user sees the full URL and decides.
#[component]
pub(crate) fn ExternalLinkConfirm(
    locale: RwSignal<Locale>,
    pending: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        {move || pending.get().map(|url| {
            let target = url.clone();
            view! {
                <div class="overlay" data-testid="external-link-confirm">
                    <div class="modal confirm-modal external-link-modal"
                        role="dialog" aria-modal="true"
                        aria-label=move || t(locale.get(), "link.confirm_title")>
                        <h2>{move || t(locale.get(), "link.confirm_title")}</h2>
                        <div class="hint">{move || t(locale.get(), "link.confirm_hint")}</div>
                        <code class="external-link-url" data-testid="external-link-url">{url}</code>
                        <div class="row">
                            <button type="button" on:click=move |_| pending.set(None)>
                                {move || t(locale.get(), "settings.cancel")}
                            </button>
                            <button type="button" class="primary" data-testid="external-link-open"
                                on:click=move |_| {
                                    pending.set(None);
                                    open_external_url(target.clone());
                                }>
                                {move || t(locale.get(), "link.confirm_open")}
                            </button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

pub(crate) fn present_browser_tab_cleanup(
    pending: RwSignal<Option<BrowserTabCleanupPrompt>>,
    queue: RwSignal<Vec<BrowserTabCleanupPrompt>>,
    selected: RwSignal<HashSet<(String, i64)>>,
    error: RwSignal<Option<String>>,
    prompt: BrowserTabCleanupPrompt,
) {
    if prompt.tabs.is_empty() {
        return;
    }
    let turn_id = prompt.turn_id.clone();
    let select_all = |prompt: &BrowserTabCleanupPrompt| {
        selected.set(
            prompt
                .tabs
                .iter()
                .map(|tab| (tab.session.clone(), tab.tab_id))
                .collect(),
        );
        error.set(None);
    };
    if pending
        .get_untracked()
        .as_ref()
        .is_some_and(|current| current.turn_id == turn_id)
    {
        select_all(&prompt);
        pending.set(Some(prompt));
        return;
    }
    if pending.get_untracked().is_some() {
        queue.update(|queue| {
            queue.retain(|item| item.turn_id != turn_id);
            queue.push(prompt);
        });
        return;
    }
    select_all(&prompt);
    pending.set(Some(prompt));
}

pub(crate) fn advance_browser_tab_cleanup(
    pending: RwSignal<Option<BrowserTabCleanupPrompt>>,
    queue: RwSignal<Vec<BrowserTabCleanupPrompt>>,
    selected: RwSignal<HashSet<(String, i64)>>,
    error: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
) {
    busy.set(false);
    error.set(None);
    pending.set(None);
    let next = queue.try_update(|queue| {
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    });
    if let Some(prompt) = next.flatten() {
        present_browser_tab_cleanup(pending, queue, selected, error, prompt);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct BrowserTabCleanupOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) pending: RwSignal<Option<BrowserTabCleanupPrompt>>,
    pub(crate) selected: RwSignal<HashSet<(String, i64)>>,
    pub(crate) busy: RwSignal<bool>,
    pub(crate) error: RwSignal<Option<String>>,
}

#[component]
pub(crate) fn BrowserTabCleanupOverlay(
    state: BrowserTabCleanupOverlayState,
    on_keep: Callback<()>,
    on_close: Callback<Vec<BrowserTabCleanupItem>>,
) -> impl IntoView {
    let BrowserTabCleanupOverlayState {
        locale,
        pending,
        selected,
        busy,
        error,
    } = state;
    view! {
        {move || pending.get().map(|prompt| {
            let n = prompt.tabs.len();
            let tabs = prompt.tabs.clone();
            view! {
                <div class="overlay" data-testid="browser-tab-cleanup">
                    <div class="modal confirm-modal browser-tab-cleanup-modal"
                        role="dialog" aria-modal="true"
                        aria-labelledby="browser-tab-cleanup-title">
                        <h2 id="browser-tab-cleanup-title">
                            {move || t(locale.get(), "browser.cleanup.title")}
                        </h2>
                        <div class="hint">
                            {move || tf(
                                locale.get(),
                                "browser.cleanup.body",
                                &[("n", &n.to_string())],
                            )}
                        </div>
                        <div class="browser-tab-cleanup-list" data-testid="browser-tab-cleanup-list">
                            {tabs.iter().cloned().map(|tab| {
                                let key = (tab.session.clone(), tab.tab_id);
                                let key_for_check = key.clone();
                                view! {
                                    <label class="browser-tab-cleanup-row"
                                        data-testid=format!("browser-tab-cleanup-row-{}", tab.tab_id)>
                                        <input type="checkbox"
                                            data-testid=format!("browser-tab-cleanup-check-{}", tab.tab_id)
                                            prop:checked=move || selected.get().contains(&key_for_check)
                                            on:change=move |ev| {
                                                let checked = event_target_checked(&ev);
                                                selected.update(|set| {
                                                    if checked {
                                                        set.insert(key.clone());
                                                    } else {
                                                        set.remove(&key);
                                                    }
                                                });
                                            } />
                                        <span class="browser-tab-cleanup-body">
                                            <strong>{if tab.title.trim().is_empty() {
                                                tab.url.clone()
                                            } else {
                                                tab.title.clone()
                                            }}</strong>
                                            <span class="browser-tab-cleanup-url">{tab.url.clone()}</span>
                                        </span>
                                    </label>
                                }
                            }).collect_view()}
                        </div>
                        {move || error.get().map(|text| view! {
                            <div class="settings-status fail">{text}</div>
                        })}
                        <div class="row">
                            <button type="button"
                                data-testid="browser-tab-cleanup-keep"
                                disabled=move || busy.get()
                                on:click=move |_| on_keep.call(())>
                                {move || t(locale.get(), "browser.cleanup.keep")}
                            </button>
                            <button type="button" class="primary"
                                data-testid="browser-tab-cleanup-close"
                                disabled=move || busy.get()
                                on:click=move |_| {
                                    let chosen = pending
                                        .get_untracked()
                                        .map(|prompt| {
                                            prompt
                                                .tabs
                                                .into_iter()
                                                .filter(|tab| selected.get_untracked().contains(&(tab.session.clone(), tab.tab_id)))
                                                .collect::<Vec<_>>()
                                        })
                                        .unwrap_or_default();
                                    on_close.call(chosen);
                                }>
                                {move || t(locale.get(), "browser.cleanup.close")}
                            </button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

pub(crate) fn present_browser_needs_human(
    pending: RwSignal<Option<BrowserNeedsHumanPrompt>>,
    error: RwSignal<Option<String>>,
    prompt: BrowserNeedsHumanPrompt,
) {
    error.set(None);
    if prompt.tabs.is_empty() {
        pending.set(None);
        return;
    }
    pending.set(Some(prompt));
}

#[derive(Clone, Copy)]
pub(crate) struct BrowserNeedsHumanOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) pending: RwSignal<Option<BrowserNeedsHumanPrompt>>,
    pub(crate) busy: RwSignal<bool>,
    pub(crate) error: RwSignal<Option<String>>,
}

#[component]
pub(crate) fn BrowserNeedsHumanOverlay(
    state: BrowserNeedsHumanOverlayState,
    on_later: Callback<()>,
    on_done: Callback<Vec<BrowserNeedsHumanTab>>,
    on_show: Callback<BrowserNeedsHumanTab>,
) -> impl IntoView {
    let BrowserNeedsHumanOverlayState {
        locale,
        pending,
        busy,
        error,
    } = state;
    view! {
        {move || pending.get().map(|prompt| {
            let tabs = prompt.tabs.clone();
            view! {
                <div class="overlay" data-testid="browser-needs-human">
                    <div class="modal confirm-modal browser-tab-cleanup-modal"
                        role="dialog" aria-modal="true"
                        aria-labelledby="browser-needs-human-title">
                        <h2 id="browser-needs-human-title">
                            {move || t(locale.get(), "browser.needs_human.title")}
                        </h2>
                        <div class="hint">
                            {move || t(locale.get(), "browser.needs_human.body")}
                        </div>
                        <div class="hint">
                            {move || t(locale.get(), "browser.needs_human.hint")}
                        </div>
                        <div class="browser-tab-cleanup-list" data-testid="browser-needs-human-list">
                            {tabs.iter().cloned().map(|tab| {
                                let show_tab = tab.clone();
                                view! {
                                    <button type="button" class="browser-tab-cleanup-row browser-needs-human-row"
                                        data-testid=format!("browser-needs-human-row-{}", tab.tab_id)
                                        disabled=move || busy.get()
                                        on:click=move |_| on_show.call(show_tab.clone())>
                                        <span class="browser-tab-cleanup-body">
                                            <strong>{if tab.title.trim().is_empty() {
                                                tab.url.clone()
                                            } else {
                                                tab.title.clone()
                                            }}</strong>
                                            <span class="browser-tab-cleanup-url">{tab.url.clone()}</span>
                                            <span class="browser-tab-cleanup-url">
                                                {move || t(locale.get(), "browser.needs_human.show")}
                                            </span>
                                        </span>
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                        {move || error.get().map(|text| view! {
                            <div class="settings-status fail" data-testid="browser-needs-human-error">{text}</div>
                        })}
                        <div class="row">
                            <button type="button"
                                data-testid="browser-needs-human-later"
                                disabled=move || busy.get()
                                on:click=move |_| on_later.call(())>
                                {move || t(locale.get(), "browser.needs_human.later")}
                            </button>
                            <button type="button" class="primary"
                                data-testid="browser-needs-human-done"
                                disabled=move || busy.get()
                                on:click=move |_| {
                                    let tabs = pending
                                        .get_untracked()
                                        .map(|prompt| prompt.tabs)
                                        .unwrap_or_default();
                                    on_done.call(tabs);
                                }>
                                {move || t(locale.get(), "browser.needs_human.done")}
                            </button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct UpdateCheckOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) update_check_modal: RwSignal<Option<UpdateCheckModal>>,
    pub(crate) update_check_enabled: RwSignal<bool>,
    pub(crate) update_banner: RwSignal<Option<AvailableUpdate>>,
}

#[component]
pub(crate) fn UpdateCheckOverlay(state: UpdateCheckOverlayState) -> impl IntoView {
    let UpdateCheckOverlayState {
        locale,
        update_check_modal,
        update_check_enabled,
        update_banner,
    } = state;
    view! {
        {move || update_check_modal.get().map(|modal| match modal {
            UpdateCheckModal::Checking => view! {
                <div class="overlay">
                    <div class="modal confirm-modal update-check-modal" data-testid="update-check-modal">
                        <h2>{move || t(locale.get(), "update_modal.checking_title")}</h2>
                        <div class="hint">{move || t(locale.get(), "update_modal.checking_body")}</div>
                    </div>
                </div>
            }
            .into_view(),
            UpdateCheckModal::Available {
                version,
                notes,
                release_url,
                install_supported,
                downloading,
            } => {
                let body = tf(locale.get(), "update_modal.available_body", &[("version", &version)]);
                let notes_html = (!notes.trim().is_empty()).then(|| md_to_html(&notes));
                let release_for_open = release_url.clone();
                let version_for_download = version.clone();
                let release_for_download = release_url.clone();
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal update-check-modal" data-testid="update-check-modal">
                            <h2>{move || t(locale.get(), "update_modal.available_title")}</h2>
                            <div class="hint">{body}</div>
                            {notes_html.map(|html| view! {
                                <div class="update-notes md markdown" inner_html=html></div>
                            })}
                            <div class="row">
                                <button
                                    type="button"
                                    class="update-modal-dismiss"
                                    data-testid="update-check-dismiss"
                                    on:click=move |_| {
                                        update_check_enabled.set(false);
                                        update_banner.set(None);
                                        update_check_modal.set(None);
                                        spawn_local(async {
                                            let arg = to_value(&serde_json::json!({ "enabled": false })).unwrap_or(JsValue::NULL);
                                            let _ = invoke("set_update_check_enabled", arg).await;
                                        });
                                    }
                                >
                                    {move || t(locale.get(), "update_modal.never")}
                                </button>
                                <button
                                    type="button"
                                    on:click=move |_| update_check_modal.set(None)
                                >
                                    {move || t(locale.get(), "update_modal.later")}
                                </button>
                                <button
                                    type="button"
                                    class:primary=move || !install_supported
                                    data-testid="update-check-open-releases"
                                    on:click=move |_| {
                                        open_external_url(release_for_open.clone());
                                        update_check_modal.set(None);
                                    }
                                >
                                    {move || t(locale.get(), "update_modal.open_releases")}
                                </button>
                                {install_supported.then(|| view! {
                                    <button
                                        type="button"
                                        class="primary"
                                        data-testid="update-check-download"
                                        prop:disabled=downloading
                                        on:click=move |_| {
                                            let version = version_for_download.clone();
                                            let release_url = release_for_download.clone();
                                            let downloaded_bytes = create_rw_signal(0_u64);
                                            let total_bytes = create_rw_signal(None::<u64>);
                                            update_check_modal.set(Some(UpdateCheckModal::Downloading {
                                                version: version.clone(),
                                                downloaded_bytes,
                                                total_bytes,
                                            }));
                                            spawn_local(async move {
                                                let callback = Closure::<dyn FnMut(JsValue)>::wrap(Box::new(
                                                    move |value: JsValue| {
                                                        let Ok(event) = serde_wasm_bindgen::from_value::<UpdateDownloadEvent>(value) else {
                                                            return;
                                                        };
                                                        match event {
                                                            UpdateDownloadEvent::Started { content_length } => {
                                                                total_bytes.set(content_length);
                                                            }
                                                            UpdateDownloadEvent::Progress { chunk_length } => {
                                                                downloaded_bytes.update(|bytes| {
                                                                    *bytes = bytes.saturating_add(chunk_length);
                                                                });
                                                            }
                                                            UpdateDownloadEvent::Verified => {}
                                                        }
                                                    },
                                                ));
                                                let result = download_app_update(
                                                    callback.as_ref().unchecked_ref(),
                                                ).await;
                                                drop(callback);
                                                match result {
                                                    Ok(_) => update_check_modal.set(Some(
                                                        UpdateCheckModal::ReadyToInstall {
                                                            version,
                                                            release_url,
                                                        },
                                                    )),
                                                    Err(error) => update_check_modal.set(Some(
                                                        UpdateCheckModal::Failed {
                                                            message: localize_backend(
                                                                locale.get_untracked(),
                                                                &js_error_text(error),
                                                            ),
                                                            release_url: Some(release_url),
                                                        },
                                                    )),
                                                }
                                            });
                                        }
                                    >
                                        {move || if downloading {
                                            t(locale.get(), "transfer.downloading")
                                        } else {
                                            t(locale.get(), "update_modal.download")
                                        }}
                                    </button>
                                })}
                            </div>
                        </div>
                    </div>
                }
                .into_view()
            }
            UpdateCheckModal::Downloading {
                version,
                downloaded_bytes,
                total_bytes,
            } => {
                let title = tf(
                    locale.get(),
                    "update_modal.downloading_title",
                    &[("version", &version)],
                );
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal update-check-modal" data-testid="update-check-modal">
                            <h2>{title}</h2>
                            <div class="hint">{move || t(locale.get(), "update_modal.downloading_body")}</div>
                            <div class="update-download-progress" role="status" aria-live="polite">
                                <progress
                                    max=move || total_bytes.get().unwrap_or(1).to_string()
                                    value=move || total_bytes.get().map(|_| downloaded_bytes.get().to_string())
                                ></progress>
                                <span>{move || if let Some(total) = total_bytes.get() {
                                    format!("{} / {}", format_bytes(downloaded_bytes.get()), format_bytes(total))
                                } else {
                                    format_bytes(downloaded_bytes.get())
                                }}</span>
                            </div>
                        </div>
                    </div>
                }
                .into_view()
            }
            UpdateCheckModal::ReadyToInstall { version, release_url } => {
                let body = tf(
                    locale.get(),
                    "update_modal.ready_body",
                    &[("version", &version)],
                );
                let release_for_open = release_url.clone();
                let version_for_install = version.clone();
                let release_for_install = release_url.clone();
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal update-check-modal" data-testid="update-check-modal">
                            <h2>{move || t(locale.get(), "update_modal.ready_title")}</h2>
                            <div class="hint">{body}</div>
                            <div class="row">
                                <button
                                    type="button"
                                    on:click=move |_| update_check_modal.set(None)
                                >
                                    {move || t(locale.get(), "update_modal.later")}
                                </button>
                                <button
                                    type="button"
                                    data-testid="update-check-open-releases"
                                    on:click=move |_| {
                                        open_external_url(release_for_open.clone());
                                        update_check_modal.set(None);
                                    }
                                >
                                    {move || t(locale.get(), "update_modal.open_releases")}
                                </button>
                                <button
                                    type="button"
                                    class="primary"
                                    data-testid="update-check-install"
                                    on:click=move |_| {
                                        let version = version_for_install.clone();
                                        let release_url = release_for_install.clone();
                                        update_check_modal.set(Some(UpdateCheckModal::Installing {
                                            version: version.clone(),
                                        }));
                                        spawn_local(async move {
                                            if let Err(error) = invoke_checked(
                                                "install_update",
                                                JsValue::UNDEFINED,
                                            ).await {
                                                update_check_modal.set(Some(UpdateCheckModal::Failed {
                                                    message: localize_backend(
                                                        locale.get_untracked(),
                                                        &js_error_text(error),
                                                    ),
                                                    release_url: Some(release_url),
                                                }));
                                            }
                                        });
                                    }
                                >
                                    {move || t(locale.get(), "update_modal.install")}
                                </button>
                            </div>
                        </div>
                    </div>
                }
                .into_view()
            }
            UpdateCheckModal::Installing { version } => {
                let title = tf(
                    locale.get(),
                    "update_modal.installing_title",
                    &[("version", &version)],
                );
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal update-check-modal" data-testid="update-check-modal">
                            <h2>{title}</h2>
                            <div class="hint">{move || t(locale.get(), "update_modal.installing_body")}</div>
                        </div>
                    </div>
                }
                .into_view()
            }
            UpdateCheckModal::UpToDate { version } => {
                let body = tf(locale.get(), "update_modal.up_to_date_body", &[("version", &version)]);
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal update-check-modal" data-testid="update-check-modal">
                            <h2>{move || t(locale.get(), "update_modal.up_to_date_title")}</h2>
                            <div class="hint">{body}</div>
                            <div class="row">
                                <button
                                    type="button"
                                    class="primary"
                                    on:click=move |_| update_check_modal.set(None)
                                >
                                    {move || t(locale.get(), "update_modal.ok")}
                                </button>
                            </div>
                        </div>
                    </div>
                }
                .into_view()
            }
            UpdateCheckModal::Failed { message, release_url } => {
                let has_release = release_url.is_some();
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal update-check-modal" data-testid="update-check-modal">
                            <h2>{move || t(locale.get(), "update_modal.failed_title")}</h2>
                            <div class="hint" role="alert">{message}</div>
                            <div class="row">
                                <button
                                    type="button"
                                    class:primary=move || !has_release
                                    on:click=move |_| update_check_modal.set(None)
                                >
                                    {move || t(locale.get(), "update_modal.ok")}
                                </button>
                                {release_url.map(|url| view! {
                                    <button
                                        type="button"
                                        class="primary"
                                        data-testid="update-check-open-releases"
                                        on:click=move |_| {
                                            open_external_url(url.clone());
                                            update_check_modal.set(None);
                                        }
                                    >
                                        {move || t(locale.get(), "update_modal.open_releases")}
                                    </button>
                                })}
                            </div>
                        </div>
                    </div>
                }
                .into_view()
            }
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SshConnectivityOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) ssh_connectivity_modal: RwSignal<Option<SshConnectivityModal>>,
    pub(crate) ssh_connectivity_busy: RwSignal<bool>,
    pub(crate) execution_contexts: RwSignal<Vec<ExecutionContext>>,
    pub(crate) remote_file_cwd: RwSignal<String>,
    pub(crate) remote_file_entries: RwSignal<Vec<DirEntry>>,
    pub(crate) remote_file_loading: RwSignal<bool>,
    pub(crate) remote_file_error: RwSignal<Option<String>>,
    pub(crate) file_source: RwSignal<String>,
}

#[component]
pub(crate) fn SshConnectivityOverlay(
    state: SshConnectivityOverlayState,
    apply_session_compute_resource: Callback<(String, bool)>,
    edit_ssh_host: Callback<String>,
    open_settings: Callback<Option<String>>,
) -> impl IntoView {
    let SshConnectivityOverlayState {
        locale,
        ssh_connectivity_modal,
        ssh_connectivity_busy,
        execution_contexts,
        remote_file_cwd,
        remote_file_entries,
        remote_file_loading,
        remote_file_error,
        file_source,
    } = state;
    view! {
        {move || ssh_connectivity_modal.get().map(|modal| {
            let host = modal.label.clone();
            let raw_detail = modal.detail.clone();
            let context_id = modal.context_id.clone();
            let enable_after = modal.enable_after_probe;
            let failed = modal.phase == SshCheckPhase::Failed;
            let fail_kind = classify_ssh_failure(&raw_detail);
            let loc = locale.get();
            let detail = localize_backend(loc, &raw_detail);
            let title = if failed {
                match fail_kind {
                    SshFailKind::ProbeOutput => t(loc, "ssh_check.probe_output_title"),
                    SshFailKind::PasswordAuth => t(loc, "ssh_check.password_title"),
                    SshFailKind::KeyAuth => t(loc, "ssh_check.key_title"),
                    _ => t(loc, "ssh_check.fail_title"),
                }
            } else {
                t(loc, "ssh_check.title")
            };
            let body = if failed {
                let key = match fail_kind {
                    SshFailKind::ProbeOutput => "ssh_check.probe_output_body",
                    SshFailKind::PasswordAuth => "ssh_check.password_body",
                    SshFailKind::KeyAuth => "ssh_check.key_body",
                    _ => "ssh_check.fail_body",
                };
                tf(loc, key, &[("host", &host)])
            } else {
                tf(loc, "ssh_check.body", &[("host", &host)])
            };
            let detail_line = tf(loc, "ssh_check.detail", &[("detail", &detail)]);
            let cause_keys = ssh_fail_cause_keys(fail_kind);
            let host_for_probe = host.clone();
            let run_probe = Rc::new({
                let context_id = context_id.clone();
                move || {
                    let context_id = context_id.clone();
                    let host_for_probe = host_for_probe.clone();
                    ssh_connectivity_busy.set(true);
                    spawn_local(async move {
                        let arg =
                            to_value(&serde_json::json!({ "contextId": context_id.clone() }))
                                .unwrap();
                        match invoke_checked("probe_execution_context", arg).await {
                            Ok(value) => {
                                show_probe_stopped_toast(&value, locale);
                                refresh_execution_contexts(execution_contexts);
                                let Ok(updated) =
                                    serde_wasm_bindgen::from_value::<ExecutionContext>(value)
                                else {
                                    ssh_connectivity_busy.set(false);
                                    return;
                                };
                                if ssh_context_known_good(&updated) {
                                    if enable_after {
                                        apply_session_compute_resource
                                            .call((context_id.clone(), true));
                                        show_toast(&t(
                                            locale.get_untracked(),
                                            "ssh_check.enabled",
                                        ));
                                    } else {
                                        show_toast(&t(
                                            locale.get_untracked(),
                                            "ssh_check.probed_ok",
                                        ));
                                    }
                                    if file_source.get_untracked() == context_id {
                                        refresh_remote_dir(
                                            context_id.clone(),
                                            remote_file_cwd,
                                            remote_file_entries,
                                            remote_file_loading,
                                            remote_file_error,
                                            file_source,
                                        );
                                    }
                                    ssh_connectivity_modal.set(None);
                                } else {
                                    // Stop probing loop: switch to diagnosis + fix.
                                    let detail = ssh_connectivity_gap(&updated)
                                        .unwrap_or_else(|| "probe failed".into());
                                    let label = if updated.label.trim().is_empty() {
                                        updated.id.clone()
                                    } else {
                                        updated.label.clone()
                                    };
                                    ssh_connectivity_modal.set(Some(SshConnectivityModal::failed(
                                        context_id.clone(),
                                        label,
                                        detail,
                                        enable_after,
                                    )));
                                    show_warning_toast(&t(
                                        locale.get_untracked(),
                                        "ssh_check.still_failed",
                                    ));
                                }
                            }
                            Err(error) => {
                                let message = localize_backend(
                                    locale.get_untracked(),
                                    &js_error_text(error),
                                );
                                show_toast(&message);
                                ssh_connectivity_modal.set(Some(SshConnectivityModal::failed(
                                    context_id.clone(),
                                    host_for_probe,
                                    message,
                                    enable_after,
                                )));
                            }
                        }
                        ssh_connectivity_busy.set(false);
                    });
                }
            });
            let open_edit_host = Rc::new({
                let context_id = context_id.clone();
                move || {
                    let alias = context_id
                        .strip_prefix("ssh:")
                        .unwrap_or(context_id.as_str())
                        .to_string();
                    edit_ssh_host.call(alias);
                }
            });
            view! {
                <div class="overlay" data-testid="ssh-connectivity-modal">
                    <div class="modal confirm-modal update-check-modal ssh-check-modal"
                        class:ssh-check-failed=failed role="dialog" aria-modal="true">
                        <h2>{title}</h2>
                        <div class="hint ssh-check-scroll">
                            <p>{body}</p>
                            <p class="ssh-check-error">{detail_line}</p>
                            {failed.then(|| view! {
                                <div class="ssh-check-causes" data-testid="ssh-check-causes">
                                    <div class="ssh-check-causes-title">
                                        {t(loc, "ssh_check.causes_title")}
                                    </div>
                                    <ul>
                                        {cause_keys.iter().map(|key| view! {
                                            <li>{t(loc, key)}</li>
                                        }).collect_view()}
                                    </ul>
                                </div>
                            })}
                            {(!failed).then(|| view! {
                                <p>{t(loc, "ssh_check.hint")}</p>
                            })}
                        </div>
                        <div class="row ssh-check-actions">
                            <button
                                type="button"
                                prop:disabled=move || ssh_connectivity_busy.get()
                                on:click=move |_| {
                                    ssh_connectivity_modal.set(None);
                                    ssh_connectivity_busy.set(false);
                                }
                            >
                                {t(loc, "ssh_check.cancel")}
                            </button>
                            {if failed {
                                let edit = open_edit_host.clone();
                                let reprobe = run_probe.clone();
                                view! {
                                    <button
                                        type="button"
                                        data-testid="ssh-connectivity-settings"
                                        prop:disabled=move || ssh_connectivity_busy.get()
                                        on:click=move |_| {
                                            ssh_connectivity_modal.set(None);
                                            open_settings.call(Some("environments".into()));
                                        }
                                    >
                                        {t(loc, "ssh_check.jump")}
                                    </button>
                                    <button
                                        type="button"
                                        class="primary"
                                        data-testid="ssh-connectivity-fix-host"
                                        prop:disabled=move || ssh_connectivity_busy.get()
                                        on:click=move |_| edit()
                                    >
                                        {t(loc, "ssh_check.fix_host")}
                                    </button>
                                    <button
                                        type="button"
                                        data-testid="ssh-connectivity-reprobe"
                                        prop:disabled=move || ssh_connectivity_busy.get()
                                        on:click=move |_| reprobe()
                                    >
                                        {move || if ssh_connectivity_busy.get() {
                                            t(locale.get(), "ssh_check.probing")
                                        } else {
                                            t(locale.get(), "ssh_check.reprobe_after_fix")
                                        }}
                                    </button>
                                }.into_view()
                            } else {
                                let probe = run_probe.clone();
                                view! {
                                    <button
                                        type="button"
                                        class="primary"
                                        data-testid="ssh-connectivity-probe"
                                        prop:disabled=move || ssh_connectivity_busy.get()
                                        on:click=move |_| probe()
                                    >
                                        {move || if ssh_connectivity_busy.get() {
                                            t(locale.get(), "ssh_check.probing")
                                        } else {
                                            t(locale.get(), "ssh_check.probe")
                                        }}
                                    </button>
                                }.into_view()
                            }}
                        </div>
                    </div>
                </div>
            }
            .into_view()
        })}
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ContextRecoveryOverlayState {
    pub(crate) locale: RwSignal<Locale>,
    pub(crate) context_recovery_dialog: RwSignal<Option<String>>,
    pub(crate) context_recovery_busy: RwSignal<bool>,
    pub(crate) context_recovery_error: RwSignal<Option<String>>,
}

#[component]
pub(crate) fn ContextRecoveryOverlay(
    state: ContextRecoveryOverlayState,
    on_compact: Callback<String>,
    on_new_session: Callback<String>,
) -> impl IntoView {
    let ContextRecoveryOverlayState {
        locale,
        context_recovery_dialog,
        context_recovery_busy,
        context_recovery_error,
    } = state;
    view! {
        {move || context_recovery_dialog.get().map(|frame_id| {
            let compact_id = frame_id.clone();
            let new_session_id = frame_id;
            view! {
                <div class="overlay context-recovery-overlay">
                    <div
                        class="modal context-recovery-modal"
                        role="dialog"
                        aria-modal="true"
                        aria-labelledby="context-recovery-title"
                        data-testid="context-recovery-modal"
                    >
                        <h2 id="context-recovery-title">
                            {move || t(locale.get(), "context_recovery.title")}
                        </h2>
                        <p class="context-recovery-body">
                            {move || t(locale.get(), "context_recovery.body")}
                        </p>
                        <div class="context-recovery-options">
                            <button
                                type="button"
                                class="context-recovery-option recommended"
                                data-testid="context-recovery-compact"
                                disabled=move || context_recovery_busy.get()
                                on:click=move |_| on_compact.call(compact_id.clone())
                            >
                                <span class="context-recovery-option-title">
                                    {move || t(locale.get(), "context_recovery.compact")}
                                </span>
                                <span class="context-recovery-option-hint">
                                    {move || t(locale.get(), "context_recovery.compact_hint")}
                                </span>
                            </button>
                            <button
                                type="button"
                                class="context-recovery-option"
                                data-testid="context-recovery-new-session"
                                disabled=move || context_recovery_busy.get()
                                on:click=move |_| on_new_session.call(new_session_id.clone())
                            >
                                <span class="context-recovery-option-title">
                                    {move || t(locale.get(), "context_recovery.new_session")}
                                </span>
                                <span class="context-recovery-option-hint">
                                    {move || t(locale.get(), "context_recovery.new_session_hint")}
                                </span>
                            </button>
                            <button
                                type="button"
                                class="context-recovery-option"
                                data-testid="context-recovery-pause"
                                disabled=move || context_recovery_busy.get()
                                on:click=move |_| {
                                    context_recovery_dialog.set(None);
                                    context_recovery_error.set(None);
                                }
                            >
                                <span class="context-recovery-option-title">
                                    {move || t(locale.get(), "context_recovery.pause")}
                                </span>
                                <span class="context-recovery-option-hint">
                                    {move || t(locale.get(), "context_recovery.pause_hint")}
                                </span>
                            </button>
                        </div>
                        {move || context_recovery_error.get().map(|error| view! {
                            <div class="context-recovery-error" role="alert">{error}</div>
                        })}
                    </div>
                </div>
            }
        })}
    }
}
