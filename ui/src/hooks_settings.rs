//! Settings → Hooks: the two built-in hooks (automatic review, tool-failure
//! analysis) and the user's command hooks.

use crate::app_support::{compose_icon, js_error_text};
use crate::bindings::invoke_checked;
use crate::dto::{AutoFailureAnalysisSettings, CommandHook, HookEvent};
use crate::i18n::{t, use_locale};
use crate::text::{dom_value, event_target_checked, event_target_value};
use leptos::*;
use serde_wasm_bindgen::{from_value, to_value};

fn event_hint_key(event: HookEvent) -> &'static str {
    match event {
        HookEvent::UserPromptSubmit => "hooks.event_hint.user_prompt_submit",
        HookEvent::PreToolUse => "hooks.event_hint.pre_tool_use",
        HookEvent::PostToolUse => "hooks.event_hint.post_tool_use",
        HookEvent::PostToolUseFailure => "hooks.event_hint.post_tool_use_failure",
        HookEvent::Stop => "hooks.event_hint.stop",
    }
}

fn new_hook() -> CommandHook {
    CommandHook {
        event: HookEvent::PreToolUse,
        matcher: String::new(),
        command: String::new(),
        enabled: true,
    }
}

#[component]
pub(crate) fn HooksSettingsView(
    auto_failure_analysis: RwSignal<AutoFailureAnalysisSettings>,
    save_auto_failure_analysis: Callback<AutoFailureAnalysisSettings>,
    reviewer_label: Signal<String>,
    open_reviewer: Callback<()>,
) -> impl IntoView {
    let locale = use_locale();
    let auto_review_default = create_rw_signal(false);
    let hooks = create_rw_signal(Vec::<CommandHook>::new());
    // (index being edited, None for a new hook; draft)
    let editing = create_rw_signal(None::<(Option<usize>, CommandHook)>);
    let error = create_rw_signal(None::<String>);
    spawn_local(async move {
        // No session id: the default new conversations inherit.
        let default = invoke_checked("get_auto_review_enabled", to_value(&serde_json::json!({})).unwrap()).await;
        let saved = invoke_checked("get_command_hooks", wasm_bindgen::JsValue::UNDEFINED).await;
        if hooks.try_get_untracked().is_none() {
            return;
        }
        auto_review_default.set(default.ok().and_then(|value| value.as_bool()).unwrap_or(false));
        match saved {
            Ok(value) => hooks.set(from_value(value).unwrap_or_default()),
            Err(e) => error.set(Some(js_error_text(e))),
        }
    });
    let save_hooks = Callback::new(move |next: Vec<CommandHook>| {
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "hooks": next })).unwrap();
            match invoke_checked("set_command_hooks", args).await {
                Ok(value) => {
                    hooks.set(from_value(value).unwrap_or_default());
                    editing.set(None);
                    error.set(None);
                }
                Err(e) => error.set(Some(js_error_text(e))),
            }
        });
    });
    let set_auto_review_default = move |enabled: bool| {
        auto_review_default.set(enabled);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "enabled": enabled })).unwrap();
            let saved = invoke_checked("set_auto_review_enabled", args).await;
            auto_review_default.set(saved.ok().and_then(|value| value.as_bool()).unwrap_or(!enabled));
        });
    };
    let open_editor = move |index: Option<usize>, hook: CommandHook| {
        error.set(None);
        editing.set(Some((index, hook)));
    };
    let form_key = create_memo(move |_| {
        editing.with(|editing| editing.as_ref().map(|(index, draft)| (*index, draft.event)))
    });
    let update_draft = move |change: &dyn Fn(&mut CommandHook)| {
        editing.update(|editing| {
            if let Some((_, draft)) = editing {
                change(draft);
            }
        })
    };

    view! {
        <div class="settings-pane hooks-pane" data-testid="hooks-pane">
            <p class="settings-note">{move || t(locale.get(), "hooks.intro")}</p>
            <div class="conn-group-label">{move || t(locale.get(), "hooks.builtin")}</div>
            <div class="session-fields hooks-builtin">
                <div class="appearance-config-row" data-testid="hook-auto-review">
                    <div>
                        <strong>{move || t(locale.get(), "composer.auto_review")}<span class="badge">"Stop"</span></strong>
                        <span>{move || t(locale.get(), "hooks.auto_review.desc")}</span>
                        <button type="button" class="hooks-link" data-testid="hook-reviewer-model"
                            on:click=move |_| open_reviewer.call(())>
                            {move || format!("{}: {}", t(locale.get(), "composer.reviewer_model"), reviewer_label.get())}
                            {compose_icon("chevron-right")}
                        </button>
                    </div>
                    <label class="toggle">
                        <input type="checkbox" data-testid="hook-auto-review-toggle"
                            aria-label=move || t(locale.get(), "composer.auto_review")
                            prop:checked=move || auto_review_default.get()
                            on:change=move |ev| set_auto_review_default(event_target_checked(&ev)) />
                        <span class="toggle-track" aria-hidden="true"></span>
                    </label>
                </div>
                <div class="appearance-config-row" data-testid="hook-failure-analysis">
                    <div>
                        <strong>{move || t(locale.get(), "composer.auto_failure_analysis")}<span class="badge">"Stop"</span></strong>
                        <span>{move || t(locale.get(), "hooks.failure_analysis.desc")}</span>
                    </div>
                    <label class="toggle">
                        <input type="checkbox" data-testid="hook-failure-analysis-toggle"
                            aria-label=move || t(locale.get(), "composer.auto_failure_analysis")
                            prop:checked=move || auto_failure_analysis.get().enabled
                            on:change=move |ev| {
                                let mut next = auto_failure_analysis.get_untracked();
                                next.enabled = event_target_checked(&ev);
                                save_auto_failure_analysis.call(next);
                            } />
                        <span class="toggle-track" aria-hidden="true"></span>
                    </label>
                </div>
                {move || auto_failure_analysis.get().enabled.then(|| view! {
                    <label class="session-number-field"><span>{move || t(locale.get(), "composer.failure_rate_threshold")}</span>
                        <input type="number" min="1" max="100" step="1" data-testid="hook-failure-rate"
                            prop:value=move || auto_failure_analysis.get().failure_rate_threshold.to_string()
                            on:change=move |ev| {
                                let Ok(value) = dom_value(&ev).parse::<u8>() else { return; };
                                let mut next = auto_failure_analysis.get_untracked();
                                next.failure_rate_threshold = value;
                                save_auto_failure_analysis.call(next);
                            } />
                    </label>
                    <label class="session-number-field"><span>{move || t(locale.get(), "composer.minimum_failures")}</span>
                        <input type="number" min="1" max="100" step="1" data-testid="hook-minimum-failures"
                            prop:value=move || auto_failure_analysis.get().minimum_failures.to_string()
                            on:change=move |ev| {
                                let Ok(value) = dom_value(&ev).parse::<u16>() else { return; };
                                let mut next = auto_failure_analysis.get_untracked();
                                next.minimum_failures = value;
                                save_auto_failure_analysis.call(next);
                            } />
                    </label>
                })}
            </div>
            <div class="settings-toolbar settings-toolbar-end hooks-toolbar">
                <span class="conn-group-label">{move || t(locale.get(), "hooks.custom")}</span>
                <button type="button" class="settings-add-btn" data-testid="hook-new"
                    disabled=move || form_key.get().is_some_and(|(index, _)| index.is_none())
                    on:click=move |_| open_editor(None, new_hook())>
                    {compose_icon("plus")}
                    <span>{move || t(locale.get(), "hooks.add")}</span>
                </button>
            </div>
            {move || error.get().map(|message| view! {
                <div class="settings-status fail" role="alert">{message}</div>
            })}
            // Keyed on (row, event) so typing does not rebuild the form; the
            // event decides whether the matcher field shows.
            {move || form_key.get().map(|(_, event)| {
                let draft = editing.get_untracked().map(|(_, draft)| draft).unwrap_or_else(|| new_hook());
                view! {
                    <div class="conn-form hooks-form" data-testid="hook-form">
                        <div class="settings-form-grid">
                            <label>{move || t(locale.get(), "hooks.event")}
                                <select data-testid="hook-event"
                                    on:change=move |ev| {
                                        let value = dom_value(&ev);
                                        if let Some(event) = HookEvent::ALL.into_iter().find(|event| event.as_str() == value) {
                                            update_draft(&|draft| draft.event = event);
                                        }
                                    }>
                                    {HookEvent::ALL.into_iter().map(|option| view! {
                                        <option value=option.as_str() prop:selected=option == event>{option.as_str()}</option>
                                    }).collect_view()}
                                </select>
                            </label>
                            {event.matches_tools().then(|| view! {
                                <label>{move || t(locale.get(), "hooks.matcher")}
                                    <input data-testid="hook-matcher" placeholder="shell|write|edit"
                                        prop:value=draft.matcher.clone()
                                        on:input=move |ev| {
                                            let value = event_target_value(&ev);
                                            update_draft(&|draft| draft.matcher = value.clone());
                                        } />
                                    <span class="settings-field-hint">{move || t(locale.get(), "hooks.matcher_hint")}</span>
                                </label>
                            })}
                            <span class="hint span-2">{move || t(locale.get(), event_hint_key(event))}</span>
                            <label class="span-2">{move || t(locale.get(), "hooks.command")}
                                <textarea rows="3" class="hooks-command-input" data-testid="hook-command"
                                    spellcheck="false"
                                    placeholder="python3 .wisp/hooks/check.py"
                                    prop:value=draft.command.clone()
                                    on:input=move |ev| {
                                        let value = event_target_value(&ev);
                                        update_draft(&|draft| draft.command = value.clone());
                                    }></textarea>
                                <span class="settings-field-hint">{move || t(locale.get(), "hooks.command_hint")}</span>
                            </label>
                        </div>
                        <div class="row settings-footer">
                            <button type="button" on:click=move |_| { editing.set(None); error.set(None); }>
                                {move || t(locale.get(), "settings.cancel")}
                            </button>
                            <button type="button" class="primary" data-testid="hook-save"
                                disabled=move || editing.get().is_none_or(|(_, draft)| draft.command.trim().is_empty())
                                on:click=move |_| {
                                    let Some((index, draft)) = editing.get_untracked() else { return; };
                                    let mut next = hooks.get_untracked();
                                    match index {
                                        Some(index) if index < next.len() => next[index] = draft,
                                        _ => next.push(draft),
                                    }
                                    save_hooks.call(next);
                                }>
                                {move || t(locale.get(), "settings.save")}
                            </button>
                        </div>
                    </div>
                }
            })}
            {move || hooks.get().is_empty().then(|| view! {
                <div class="settings-status">{move || t(locale.get(), "hooks.empty")}</div>
            })}
            <div class="settings-list hooks-list">
                {move || hooks.get().into_iter().enumerate().map(|(index, hook)| {
                    let edit = hook.clone();
                    let enabled = hook.enabled;
                    view! {
                        <div class="settings-list-row settings-list-row-link" data-testid="hook-row"
                            on:click=move |_| open_editor(Some(index), edit.clone())>
                            <div class="settings-list-main">
                                <span class="settings-list-title">
                                    {hook.event.as_str()}
                                    {(!hook.matcher.is_empty()).then(|| view! { <span class="badge">{hook.matcher.clone()}</span> })}
                                </span>
                                <code class="settings-list-sub hooks-command">{hook.command.clone()}</code>
                            </div>
                            <div class="settings-list-actions" on:click=|ev| ev.stop_propagation()>
                                <label class="toggle">
                                    <input type="checkbox" data-testid="hook-enabled"
                                        aria-label=move || t(locale.get(), "hooks.enabled")
                                        prop:checked=enabled
                                        on:change=move |ev| {
                                            let mut next = hooks.get_untracked();
                                            if let Some(hook) = next.get_mut(index) {
                                                hook.enabled = event_target_checked(&ev);
                                            }
                                            save_hooks.call(next);
                                        } />
                                    <span class="toggle-track" aria-hidden="true"></span>
                                </label>
                                <button class="settings-list-remove" type="button" data-testid="hook-remove"
                                    title=move || t(locale.get(), "hooks.remove")
                                    aria-label=move || t(locale.get(), "hooks.remove")
                                    on:click=move |_| {
                                        let mut next = hooks.get_untracked();
                                        if index < next.len() {
                                            next.remove(index);
                                        }
                                        save_hooks.call(next);
                                    }>{compose_icon("trash")}</button>
                            </div>
                        </div>
                    }
                }).collect_view()}
            </div>
            <p class="settings-note">{move || t(locale.get(), "hooks.acp_note")}</p>
        </div>
    }
}
