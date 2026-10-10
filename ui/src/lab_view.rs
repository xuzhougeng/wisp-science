//! Settings → Remote Access → Wisp Lab (docs/wisp-lab.md): found or join the
//! research group hosted by a relay, review members as its leader, and
//! publish or download the shared knowledge base.

use crate::app_support::{copy_text, js_error_text};
use crate::bindings::invoke_checked;
use crate::dto::{LabStatus, ProjectSummary};
use crate::i18n::{localize_backend, t, Locale};
use crate::text::event_target_input;
use leptos::*;
use serde_wasm_bindgen::to_value;
use wasm_bindgen::JsValue;

/// A relay refusal arrives as `lab:<code>`; everything else is a message.
fn lab_error(locale: Locale, text: &str) -> String {
    match text.strip_prefix("lab:") {
        Some(code) => {
            let key = format!("channels.lab.error.{code}");
            let message = t(locale, &key);
            if message == key {
                code.to_string()
            } else {
                message
            }
        }
        None => localize_backend(locale, text),
    }
}

#[component]
pub(super) fn LabPane(
    locale: RwSignal<Locale>,
    on_open_project: Callback<String>,
    /// Joining a lab can fill in the project sync relay. The app reloads
    /// those settings so **Sync now** appears without a restart.
    on_sync_settings_changed: Callback<()>,
) -> impl IntoView {
    let status = create_rw_signal(None::<LabStatus>);
    let msg = create_rw_signal(None::<(bool, String)>);
    let busy = create_rw_signal(false);
    let relay_url = create_rw_signal(String::new());
    let relay_token = create_rw_signal(String::new());
    let member_name = create_rw_signal(String::new());
    let lab_name = create_rw_signal(String::new());
    let invite = create_rw_signal(String::new());
    let new_token = create_rw_signal(String::new());
    let invite_code = create_rw_signal(None::<String>);
    let projects = create_rw_signal(Vec::<ProjectSummary>::new());
    let kb_project = create_rw_signal(String::new());
    // The destructive action waiting for its second click.
    let confirming = create_rw_signal(None::<String>);

    // A plain closure over signals, not a `Callback`: requests outlive this
    // pane when the user navigates away, and `try_set` on a disposed signal
    // is a no-op where calling a disposed `Callback` would panic.
    let refresh = move || {
        spawn_local(async move {
            if let Ok(value) = invoke_checked("lab_status", JsValue::UNDEFINED).await {
                if let Ok(next) = serde_wasm_bindgen::from_value::<LabStatus>(value) {
                    let _ = status.try_set(Some(next));
                }
            }
        });
    };
    refresh();
    // The page is rebuilt when the membership state changes, not on every
    // reload of the member list.
    let state = create_memo(move |_| status.get().map(|current| current.state));
    spawn_local(async move {
        if let Ok(value) = invoke_checked("list_projects", JsValue::UNDEFINED).await {
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(value) {
                let _ = projects.try_set(list);
            }
        }
    });

    // Runs one command, reports its outcome and reloads the status.
    let run = move |command: &'static str, args: JsValue, done: Option<&'static str>| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        confirming.set(None);
        spawn_local(async move {
            match invoke_checked(command, args).await {
                Ok(_) => {
                    let _ = msg.try_set(done.map(|key| (true, t(locale.get_untracked(), key))));
                    if command == "lab_join" {
                        // Owned by the app, so it outlives this pane.
                        on_sync_settings_changed.call(());
                    }
                }
                Err(error) => {
                    let text = lab_error(locale.get_untracked(), &js_error_text(error));
                    let _ = msg.try_set(Some((false, text)));
                }
            }
            let _ = busy.try_set(false);
            refresh();
        });
    };

    let join = move |_: web_sys::MouseEvent| {
        let args = to_value(&serde_json::json!({
            "relayUrl": relay_url.get_untracked().trim(),
            "relayToken": relay_token.get_untracked(),
            "name": member_name.get_untracked().trim(),
            "labName": lab_name.get_untracked().trim(),
            "invite": invite.get_untracked().trim(),
        }))
        .unwrap();
        relay_token.set(String::new());
        run("lab_join", args, None);
    };
    let create_invite = move |_: web_sys::MouseEvent| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            match invoke_checked("lab_create_invite", JsValue::UNDEFINED).await {
                Ok(value) => {
                    let _ = invite_code.try_set(value.as_string());
                    let _ = msg.try_set(None);
                }
                Err(error) => {
                    let text = lab_error(locale.get_untracked(), &js_error_text(error));
                    let _ = msg.try_set(Some((false, text)));
                }
            }
            let _ = busy.try_set(false);
        });
    };
    let member_action = move |command: &'static str, member_id: String, done: &'static str| {
        let args = to_value(&serde_json::json!({ "memberId": member_id })).unwrap();
        run(command, args, Some(done));
    };
    let leave = move |forget: bool| {
        let args = to_value(&serde_json::json!({ "forget": forget })).unwrap();
        invite_code.set(None);
        run("lab_leave", args, Some("channels.lab.left"));
    };
    let save_token = move |_: web_sys::MouseEvent| {
        let args =
            to_value(&serde_json::json!({ "relayToken": new_token.get_untracked() })).unwrap();
        new_token.set(String::new());
        run("lab_set_relay_token", args, Some("channels.saved"));
    };
    let publish_kb = move |_: web_sys::MouseEvent| {
        let project_id = kb_project.get_untracked();
        if project_id.is_empty() {
            return;
        }
        let args = to_value(&serde_json::json!({ "projectId": project_id })).unwrap();
        run(
            "lab_publish_knowledge_base",
            args,
            Some("channels.lab.kb_published"),
        );
    };
    let set_inbox = move |project_id: String| {
        let args = to_value(&serde_json::json!({ "projectId": project_id })).unwrap();
        run("lab_set_inbox_project", args, Some("channels.lab.inbox_saved"));
    };
    let join_kb = move |_: web_sys::MouseEvent| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            match invoke_checked("lab_join_knowledge_base", JsValue::UNDEFINED).await {
                Ok(value) => {
                    // `None` means the folder picker was cancelled.
                    if let Ok(Some(project)) =
                        serde_wasm_bindgen::from_value::<Option<ProjectSummary>>(value)
                    {
                        // Opening the project closes Settings and this pane
                        // with it, so nothing here is touched afterwards.
                        let _ = busy.try_set(false);
                        on_open_project.call(project.id);
                        return;
                    }
                }
                Err(error) => {
                    let text = lab_error(locale.get_untracked(), &js_error_text(error));
                    let _ = msg.try_set(Some((false, text)));
                }
            }
            let _ = busy.try_set(false);
            refresh();
        });
    };

    // First click arms the button, second click runs `action`.
    let confirm_button = move |id: String,
                               label: &'static str,
                               testid: String,
                               action: Callback<()>| {
        let armed = {
            let id = id.clone();
            move || confirming.get().as_deref() == Some(id.as_str())
        };
        let label_armed = armed.clone();
        view! {
            <button type="button" class="danger" data-testid=testid
                prop:disabled=move || busy.get()
                on:click=move |_| {
                    if armed() {
                        action.call(());
                    } else {
                        confirming.set(Some(id.clone()));
                    }
                }>
                {move || if label_armed() {
                    t(locale.get(), "channels.lab.confirm")
                } else {
                    t(locale.get(), label)
                }}
            </button>
        }
    };

    let token_row = move || {
        view! {
            <div class="settings-form-grid">
                <label class="span-2">
                    <span>{move || t(locale.get(), "channels.lab.new_token")}</span>
                    <input type="password" data-testid="lab-new-token"
                        prop:value=move || new_token.get()
                        placeholder=move || t(locale.get(), "channels.lab.new_token_placeholder")
                        on:input=move |ev| new_token.set(event_target_input(&ev).value()) />
                </label>
                <div class="span-2 row">
                    <button type="button" data-testid="lab-save-token"
                        prop:disabled=move || busy.get() || new_token.get().trim().is_empty()
                        on:click=save_token>
                        {move || t(locale.get(), "channels.lab.save_token")}
                    </button>
                </div>
            </div>
        }
    };

    let join_form = move || {
        view! {
            <div class="settings-form-grid">
                <label class="span-2">
                    <span>{move || t(locale.get(), "channels.lab.invite")}</span>
                    <input type="text" data-testid="lab-invite"
                        prop:value=move || invite.get()
                        placeholder=move || t(locale.get(), "channels.lab.invite_placeholder")
                        on:input=move |ev| invite.set(event_target_input(&ev).value()) />
                </label>
                <label class="span-2">
                    <span>{move || t(locale.get(), "channels.remote.relay_url")}</span>
                    <input type="url" data-testid="lab-relay-url"
                        prop:value=move || relay_url.get()
                        placeholder=move || if invite.get().trim().is_empty() {
                            "https://relay.example.com".to_string()
                        } else {
                            t(locale.get(), "channels.lab.relay_url_from_invite")
                        }
                        on:input=move |ev| relay_url.set(event_target_input(&ev).value()) />
                </label>
                <label class="span-2">
                    <span>{move || t(locale.get(), "channels.remote.token")}</span>
                    <input type="password" data-testid="lab-relay-token"
                        prop:value=move || relay_token.get()
                        placeholder=move || if status.get().unwrap_or_default().has_token {
                            t(locale.get(), "settings.key_stored")
                        } else {
                            t(locale.get(), "channels.remote.token_placeholder")
                        }
                        on:input=move |ev| relay_token.set(event_target_input(&ev).value()) />
                </label>
                <label>
                    <span>{move || t(locale.get(), "channels.lab.your_name")}</span>
                    <input type="text" data-testid="lab-member-name" maxlength="64"
                        prop:value=move || member_name.get()
                        on:input=move |ev| member_name.set(event_target_input(&ev).value()) />
                </label>
                {move || invite.get().trim().is_empty().then(|| view! {
                    <label>
                        <span>{move || t(locale.get(), "channels.lab.lab_name")}</span>
                        <input type="text" data-testid="lab-name" maxlength="64"
                            prop:value=move || lab_name.get()
                            on:input=move |ev| lab_name.set(event_target_input(&ev).value()) />
                    </label>
                })}
            </div>
            <p class="settings-note">{move || t(locale.get(), "channels.lab.join_note")}</p>
            <div class="row settings-footer">
                <span class="settings-footer-note">{move || t(locale.get(), "channels.secret_note")}</span>
                <button type="button" class="primary" data-testid="lab-join"
                    prop:disabled=move || busy.get() || member_name.get().trim().is_empty()
                    on:click=join>
                    {move || if invite.get().trim().is_empty() {
                        t(locale.get(), "channels.lab.found")
                    } else {
                        t(locale.get(), "channels.lab.join")
                    }}
                </button>
            </div>
        }
    };

    let members_view = move || {
        let current = status.get().unwrap_or_default();
        let leader = current.leader;
        let me = current.member_id.clone();
        current
            .members
            .into_iter()
            .map(|member| {
                let mine = member.id == me;
                let approve_id = member.id.clone();
                let remove_id = member.id.clone();
                let remove = Callback::new(move |_: ()| {
                    member_action("lab_remove_member", remove_id.clone(), "channels.lab.removed")
                });
                view! {
                    <div class="settings-list-row" data-testid="lab-member-row">
                        <div class="settings-list-main">
                            <span class="settings-list-title">
                                {member.name.clone()}
                                {member.leader.then(|| view! {
                                    " " <span class="badge">{move || t(locale.get(), "channels.lab.role_leader")}</span>
                                })}
                                {mine.then(|| view! {
                                    " " <span class="badge">{move || t(locale.get(), "channels.lab.you")}</span>
                                })}
                                {member.pending.then(|| view! {
                                    " " <span class="badge channel-state-connecting">{move || t(locale.get(), "channels.lab.pending")}</span>
                                })}
                            </span>
                        </div>
                        {(leader && !mine).then(|| view! {
                            <div class="settings-list-actions">
                                {member.pending.then(|| view! {
                                    <button type="button" data-testid="lab-approve"
                                        prop:disabled=move || busy.get()
                                        on:click=move |_| member_action(
                                            "lab_approve_member",
                                            approve_id.clone(),
                                            "channels.lab.approved",
                                        )>
                                        {move || t(locale.get(), "channels.lab.approve")}
                                    </button>
                                })}
                                {confirm_button(
                                    format!("remove:{}", member.id),
                                    if member.pending { "channels.lab.reject" } else { "channels.lab.remove" },
                                    "lab-remove".to_string(),
                                    remove,
                                )}
                            </div>
                        })}
                    </div>
                }
            })
            .collect_view()
    };

    let knowledge_base_view = move || {
        let current = status.get().unwrap_or_default();
        let knowledge_base = current.knowledge_base.clone();
        let summary = match &knowledge_base {
            Some(kb) if kb.local => t(locale.get(), "channels.lab.kb_local")
                .replace("{name}", &kb.project_name),
            Some(_) => t(locale.get(), "channels.lab.kb_remote"),
            None => t(locale.get(), "channels.lab.kb_none"),
        };
        let downloadable = knowledge_base.as_ref().is_some_and(|kb| !kb.local);
        view! {
            <h4>{move || t(locale.get(), "channels.lab.kb_title")}</h4>
            <p class="settings-field-hint" data-testid="lab-kb-summary">{summary}</p>
            {downloadable.then(|| view! {
                <div class="row">
                    <button type="button" data-testid="lab-kb-download"
                        prop:disabled=move || busy.get()
                        on:click=join_kb>
                        {move || t(locale.get(), "channels.lab.kb_download")}
                    </button>
                </div>
            })}
            {current.leader.then(|| view! {
                <div class="settings-form-grid">
                    <label class="span-2">
                        <span>{move || t(locale.get(), "channels.lab.kb_project")}</span>
                        <select data-testid="lab-kb-project"
                            on:change=move |ev| kb_project.set(event_target_value(&ev))>
                            <option value="">{move || t(locale.get(), "channels.lab.kb_choose")}</option>
                            {move || projects.get().into_iter().map(|project| {
                                let id = project.id.clone();
                                view! {
                                    <option value=project.id
                                        selected=move || kb_project.get() == id>
                                        {project.name}
                                    </option>
                                }
                            }).collect_view()}
                        </select>
                    </label>
                    <div class="span-2 row">
                        <button type="button" data-testid="lab-kb-publish"
                            prop:disabled=move || busy.get() || kb_project.get().is_empty()
                            on:click=publish_kb>
                            {move || t(locale.get(), "channels.lab.kb_publish")}
                        </button>
                    </div>
                </div>
                <p class="settings-note">{move || t(locale.get(), "channels.lab.kb_note")}</p>
            })}
        }
    };

    let mail_view = move || {
        let current = status.get().unwrap_or_default().inbox_project_id;
        let none_selected = current.is_empty();
        view! {
            <h4>{move || t(locale.get(), "channels.lab.mail_title")}</h4>
            <div class="settings-form-grid">
                <label class="span-2">
                    <span>{move || t(locale.get(), "channels.lab.inbox_project")}</span>
                    <select data-testid="lab-inbox-project"
                        prop:disabled=move || busy.get()
                        on:change=move |ev| set_inbox(event_target_value(&ev))>
                        <option value="" selected=none_selected>
                            {move || t(locale.get(), "channels.lab.inbox_none")}
                        </option>
                        {projects.get().into_iter().map(|project| {
                            let selected = project.id == current;
                            view! {
                                <option value=project.id selected=selected>{project.name}</option>
                            }
                        }).collect_view()}
                    </select>
                </label>
            </div>
            <p class="settings-note">{move || t(locale.get(), "channels.lab.mail_note")}</p>
        }
    };

    let invite_view = move || {
        view! {
            <div class="device-token-row">
                <div>
                    <strong>{move || t(locale.get(), "channels.lab.invite_title")}</strong>
                    <p class="settings-field-hint">{move || t(locale.get(), "channels.lab.invite_hint")}</p>
                    {move || invite_code.get().map(|code| view! {
                        <p><code class="lab-invite-code" data-testid="lab-invite-code">{code}</code></p>
                    })}
                </div>
                <div class="row">
                    {move || invite_code.get().map(|code| view! {
                        <button type="button" data-testid="lab-invite-copy"
                            on:click=move |_| {
                                copy_text(code.clone());
                                msg.set(Some((true, t(locale.get_untracked(), "channels.remote.copied"))));
                            }>
                            {move || t(locale.get(), "channels.lab.invite_copy")}
                        </button>
                    })}
                    <button type="button" data-testid="lab-invite-create"
                        prop:disabled=move || busy.get()
                        on:click=create_invite>
                        {move || t(locale.get(), "channels.lab.invite_create")}
                    </button>
                </div>
            </div>
        }
    };

    let state_badge = move || {
        let current = status.get().unwrap_or_default();
        let (tone, key) = match current.state.as_str() {
            "active" if current.leader => ("running", "channels.lab.role_leader"),
            "active" => ("running", "channels.lab.role_member"),
            "pending" => ("connecting", "channels.lab.pending"),
            "removed" | "error" => ("error", "channels.state.error"),
            _ => ("stopped", "channels.lab.not_joined"),
        };
        view! {
            <span class=format!("badge channel-state-{tone}") data-testid="lab-state">
                {t(locale.get(), key)}
            </span>
        }
    };

    view! {
        <div class="settings-pane settings-pane-subpage" data-testid="lab-channel-card">
            {move || msg.get().map(|(ok, text)| view! {
                <div class="settings-status" class:ok=move || ok class:fail=move || !ok
                    data-testid="lab-message">{text}</div>
            })}
            <div class="channel-bind-row">
                <div>
                    <strong data-testid="lab-heading">{move || {
                        let current = status.get().unwrap_or_default();
                        if current.lab_name.is_empty() {
                            t(locale.get(), "channels.lab.heading")
                        } else {
                            current.lab_name
                        }
                    }}</strong>
                    <p>{move || {
                        let current = status.get().unwrap_or_default();
                        if current.relay_url.is_empty() {
                            t(locale.get(), "channels.lab.hint")
                        } else {
                            current.relay_url
                        }
                    }}</p>
                </div>
                {state_badge}
            </div>
            {move || match state.get() {
                None => view! { <div></div> }.into_view(),
                Some(state) if state == "active" => view! {
                    <h4>{move || t(locale.get(), "channels.lab.members")}</h4>
                    <div class="settings-list" data-testid="lab-members">{members_view}</div>
                    {move || status.get().unwrap_or_default().leader.then(invite_view)}
                    {knowledge_base_view}
                    {mail_view}
                    {token_row}
                    {move || (!status.get().unwrap_or_default().leader).then(|| view! {
                        <div class="row settings-footer">
                            <span class="settings-footer-note">{move || t(locale.get(), "channels.lab.leave_note")}</span>
                            {confirm_button(
                                "leave".to_string(),
                                "channels.lab.leave",
                                "lab-leave".to_string(),
                                Callback::new(move |_: ()| leave(false)),
                            )}
                        </div>
                    })}
                }.into_view(),
                Some(state) if state == "pending" => view! {
                    <p class="settings-note" data-testid="lab-pending-note">
                        {move || t(locale.get(), "channels.lab.pending_note")}
                    </p>
                    <div class="row settings-footer">
                        <button type="button" data-testid="lab-refresh"
                            on:click=move |_| refresh()>
                            {move || t(locale.get(), "channels.lab.refresh")}
                        </button>
                        {confirm_button(
                            "leave".to_string(),
                            "channels.lab.withdraw",
                            "lab-leave".to_string(),
                            Callback::new(move |_: ()| leave(false)),
                        )}
                    </div>
                }.into_view(),
                Some(state) if state == "removed" || state == "error" => view! {
                    <p class="settings-field-error" data-testid="lab-error">{move || {
                        let current = status.get().unwrap_or_default();
                        if current.state == "removed" {
                            t(locale.get(), "channels.lab.removed_note")
                        } else {
                            lab_error(locale.get(), &current.detail)
                        }
                    }}</p>
                    {token_row}
                    <div class="row settings-footer">
                        <button type="button" data-testid="lab-refresh"
                            on:click=move |_| refresh()>
                            {move || t(locale.get(), "channels.lab.refresh")}
                        </button>
                        {confirm_button(
                            "forget".to_string(),
                            "channels.lab.forget",
                            "lab-forget".to_string(),
                            Callback::new(move |_: ()| leave(true)),
                        )}
                    </div>
                }.into_view(),
                Some(_) => join_form.into_view(),
            }}
        </div>
    }
}
