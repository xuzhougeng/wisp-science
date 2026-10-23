//! Review a selected range, choose a destination, or distill a reusable lesson.

use crate::app_support::{
    compose_icon, excerpt_quote, experience_quote, js_error_text, quote_source_label, show_toast,
    ComposerQuote,
};
use crate::bindings::invoke_checked;
use crate::dto::{SessionCursor, SessionExcerpt, SessionExperience, SessionInfo, SessionPage};
use crate::i18n::{localize_backend, t, Locale};
use crate::text::event_target_value;
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

#[component]
pub(crate) fn SessionReuseOverlay(
    excerpt: SessionExcerpt,
    locale: RwSignal<Locale>,
    excluded: HashSet<String>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    on_quote: Callback<(ComposerQuote, Option<String>)>,
    on_close: Callback<()>,
) -> impl IntoView {
    let source = store_value(excerpt);
    let excluded = store_value(excluded);
    let experience_mode = create_rw_signal(false);
    let draft = create_rw_signal(String::new());
    let instruction = create_rw_signal(String::new());
    let generating = create_rw_signal(false);
    let shortened = create_rw_signal(false);
    let scope = create_rw_signal("project".to_string());
    let saved = create_rw_signal(false);
    let query = create_rw_signal(String::new());
    let destination = create_rw_signal(String::new());
    let targets = create_rw_signal(Vec::<SessionInfo>::new());
    let cursor = create_rw_signal(None::<SessionCursor>);
    let searching = create_rw_signal(false);
    let search_error = create_rw_signal(None::<String>);
    let search_generation = create_rw_signal(0u64);
    let alive = Rc::new(Cell::new(true));
    let cleanup_alive = alive.clone();
    on_cleanup(move || cleanup_alive.set(false));

    let search_alive = alive.clone();
    let search = Callback::new(move |more: bool| {
        let ticket = search_generation.get_untracked().wrapping_add(1);
        search_generation.set(ticket);
        searching.set(true);
        search_error.set(None);
        if !more {
            targets.set(vec![]);
            cursor.set(None);
            destination.set(String::new());
        }
        let query = query.get_untracked();
        let next = if more { cursor.get_untracked() } else { None };
        let alive = search_alive.clone();
        spawn_local(async move {
            let args = to_value(&serde_json::json!({"query": query, "cursor": next})).unwrap();
            let result = invoke_checked("list_sessions_page", args)
                .await
                .map_err(js_error_text)
                .and_then(|value| {
                    serde_wasm_bindgen::from_value::<SessionPage>(value).map_err(|e| e.to_string())
                });
            if !alive.get() || search_generation.get_untracked() != ticket {
                return;
            }
            match result {
                Ok(page) => {
                    let source_id = source.with_value(|source| source.session_id.clone());
                    let rows = page.items.into_iter().filter(|row| {
                        row.id != source_id
                            && row.dispatched_from.is_none()
                            && !matches!(row.branch_state.as_deref(), Some("merged" | "orphaned"))
                            && !excluded.with_value(|ids| ids.contains(&row.id))
                            && (query.trim().is_empty()
                                || row.title.to_lowercase().contains(&query.to_lowercase()))
                    });
                    targets.update(|all| {
                        for row in rows {
                            if !all.iter().any(|existing| existing.id == row.id) {
                                all.push(row);
                            }
                        }
                    });
                    cursor.set(page.next_cursor);
                }
                Err(message) => {
                    search_error.set(Some(localize_backend(locale.get_untracked(), &message)))
                }
            }
            searching.set(false);
        });
    });
    create_effect(move |_| {
        let _ = query.get();
        search.call(false);
    });

    let generate_alive = alive.clone();
    let generate = Callback::new(move |_: ()| {
        if generating.get_untracked() || busy.get_untracked() {
            return;
        }
        generating.set(true);
        error.set(None);
        let excerpt = source.get_value();
        let focus = instruction.get_untracked();
        let alive = generate_alive.clone();
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "id": excerpt.session_id,
                "startUserIndex": excerpt.start_user_index,
                "endUserIndex": excerpt.end_user_index,
                "instruction": focus,
            }))
            .unwrap();
            let result = invoke_checked("generate_session_experience", args)
                .await
                .map_err(js_error_text)
                .and_then(|value| {
                    serde_wasm_bindgen::from_value::<SessionExperience>(value)
                        .map_err(|e| e.to_string())
                });
            if !alive.get() {
                return;
            }
            match result {
                Ok(experience) => {
                    draft.set(experience.content);
                    shortened.set(experience.truncated);
                    saved.set(false);
                }
                Err(message) => error.set(Some(localize_backend(locale.get_untracked(), &message))),
            }
            generating.set(false);
        });
    });
    let save_alive = alive.clone();
    let save = Callback::new(move |_: ()| {
        if busy.get_untracked()
            || generating.get_untracked()
            || draft.get_untracked().trim().is_empty()
        {
            return;
        }
        busy.set(true);
        error.set(None);
        let excerpt = source.get_value();
        let quote = experience_quote(&excerpt, &draft.get_untracked(), locale.get_untracked());
        let label = quote_source_label(quote.source.as_deref().unwrap_or_default());
        let content = format!(
            "{}\n\n{}: {} (session {})",
            quote.text,
            t(locale.get_untracked(), "excerpt.provenance"),
            label,
            excerpt.session_id
        );
        let scope = scope.get_untracked();
        let alive = save_alive.clone();
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "sessionId": excerpt.session_id,
                "turnIndex": excerpt.start_user_index,
                "scope": scope,
                "content": content,
                "replaceId": null,
            }))
            .unwrap();
            let result = invoke_checked("confirm_turn_memory", args).await;
            if !alive.get() {
                return;
            }
            match result {
                Ok(_) => {
                    saved.set(true);
                    show_toast(&t(
                        locale.get_untracked(),
                        if scope == "global" {
                            "memory.proposal.saved_global"
                        } else {
                            "memory.proposal.saved"
                        },
                    ));
                }
                Err(message) => error.set(Some(localize_backend(
                    locale.get_untracked(),
                    &js_error_text(message),
                ))),
            }
            busy.set(false);
        });
    });
    let quote = move |_| {
        if busy.get_untracked() || generating.get_untracked() {
            return;
        }
        let excerpt = source.get_value();
        let quote = if experience_mode.get_untracked() {
            if draft.get_untracked().trim().is_empty() {
                return;
            }
            experience_quote(&excerpt, &draft.get_untracked(), locale.get_untracked())
        } else {
            excerpt_quote(&excerpt, locale.get_untracked())
        };
        let target = destination.get_untracked();
        if !target.is_empty()
            && !targets.with_untracked(|rows| rows.iter().any(|row| row.id == target))
        {
            return;
        }
        on_quote.call((quote, (!target.is_empty()).then_some(target)));
    };
    let source_label = source.with_value(|excerpt| {
        let quote = excerpt_quote(excerpt, locale.get_untracked());
        quote_source_label(quote.source.as_deref().unwrap_or_default()).to_string()
    });

    view! {
        <div class="overlay session-reuse-overlay" data-testid="session-reuse-overlay">
            <div class="modal session-reuse-modal" role="dialog" aria-modal="true" aria-labelledby="session-reuse-title">
                <div class="ps-head">
                    <div><h2 id="session-reuse-title">{move || t(locale.get(), "excerpt.reuse_title")}</h2><p class="hint">{source_label}</p></div>
                    <button type="button" class="ps-close" aria-label=move || t(locale.get(), "excerpt.cancel")
                        disabled=move || busy.get() on:click=move |_| on_close.call(())>{compose_icon("close")}</button>
                </div>
                <div class="session-reuse-modes">
                    <button type="button" aria-pressed=move || (!experience_mode.get()).to_string() disabled=move || busy.get()
                        on:click=move |_| experience_mode.set(false)>{compose_icon("copy")}{move || t(locale.get(), "excerpt.original")}</button>
                    <button type="button" data-testid="excerpt-experience" aria-pressed=move || experience_mode.get().to_string() disabled=move || busy.get()
                        on:click=move |_| experience_mode.set(true)>{compose_icon("sparkles")}{move || t(locale.get(), "excerpt.experience")}</button>
                </div>
                <Show when=move || experience_mode.get() fallback=move || view! {
                    <pre class="session-reuse-preview">{source.with_value(|excerpt| excerpt.text.clone())}</pre>
                    {source.with_value(|excerpt| excerpt.truncated).then(|| view! { <p class="hint">{move || t(locale.get(), "excerpt.truncated")}</p> })}
                }>
                    <p class="hint">{move || t(locale.get(), "excerpt.experience_hint")}</p>
                    <label class="session-reuse-field">{move || t(locale.get(), "excerpt.focus")}
                        <input data-testid="excerpt-focus" maxlength="1000" prop:value=move || instruction.get() disabled=move || busy.get() || generating.get()
                            on:input=move |ev| instruction.set(event_target_value(&ev))/>
                    </label>
                    <button type="button" data-testid="excerpt-generate" disabled=move || busy.get() || generating.get() on:click=move |_| generate.call(())>
                        {move || t(locale.get(), if generating.get() { "excerpt.generating" } else if draft.get().is_empty() { "excerpt.generate" } else { "excerpt.regenerate" })}
                    </button>
                    <label class="session-reuse-field">{move || t(locale.get(), "excerpt.draft")}
                        <textarea data-testid="excerpt-draft" rows="9" prop:value=move || draft.get() disabled=move || busy.get() || generating.get()
                            on:input=move |ev| { draft.set(event_target_value(&ev)); saved.set(false); }></textarea>
                    </label>
                    {move || shortened.get().then(|| view! { <p class="hint">{move || t(locale.get(), "excerpt.evidence_shortened")}</p> })}
                    <div class="session-reuse-save">
                        <label class="session-reuse-field">{move || t(locale.get(), "memory.proposal.scope")}
                            <select data-testid="excerpt-memory-scope" prop:value=move || scope.get() disabled=move || busy.get()
                                on:change=move |ev| { scope.set(event_target_value(&ev)); saved.set(false); }>
                                <option value="project" prop:selected=move || scope.get() == "project">{move || t(locale.get(), "memory.proposal.scope_project")}</option>
                                <option value="global" prop:selected=move || scope.get() == "global">{move || t(locale.get(), "memory.proposal.scope_global")}</option>
                            </select>
                        </label>
                        <button type="button" data-testid="excerpt-save-memory" disabled=move || busy.get() || generating.get() || saved.get() || draft.get().trim().is_empty()
                            on:click=move |_| save.call(())>{move || t(locale.get(), if saved.get() { "excerpt.saved" } else { "excerpt.save_memory" })}</button>
                    </div>
                </Show>
                <label class="session-reuse-field">{move || t(locale.get(), "excerpt.search")}
                    <input data-testid="excerpt-target-search" prop:value=move || query.get() disabled=move || busy.get()
                        on:input=move |ev| query.set(event_target_value(&ev))/>
                </label>
                <label class="session-reuse-field">{move || t(locale.get(), "excerpt.destination")}
                    <select data-testid="excerpt-destination" prop:value=move || destination.get() disabled=move || busy.get()
                        on:change=move |ev| destination.set(event_target_value(&ev))>
                        <option value="" prop:selected=move || destination.get().is_empty()>{move || t(locale.get(), "excerpt.new_chat")}</option>
                        <For each=move || targets.get() key=|row| row.id.clone() let:row>
                            <option value=row.id.clone() prop:selected=move || destination.get() == row.id>{if row.title.trim().is_empty() { t(locale.get(), "excerpt.untitled") } else { row.title }}</option>
                        </For>
                    </select>
                </label>
                {move || cursor.get().is_some().then(|| view! {
                    <button type="button" disabled=move || searching.get() || busy.get() on:click=move |_| search.call(true)>{move || t(locale.get(), "excerpt.more")}</button>
                })}
                {move || search_error.get().map(|message| view! { <p role="alert">{message}</p><button type="button" on:click=move |_| search.call(false)>{move || t(locale.get(), "transcript.retry")}</button> })}
                <p class="hint">{move || t(locale.get(), "excerpt.staging_hint")}</p>
                {move || error.get().map(|message| view! { <div class="settings-status fail" role="alert">{message}</div> })}
                <div class="row">
                    <button type="button" disabled=move || busy.get() on:click=move |_| on_close.call(())>{move || t(locale.get(), "excerpt.cancel")}</button>
                    <button type="button" class="primary" data-testid="excerpt-attach" disabled=move || busy.get() || generating.get() || (experience_mode.get() && draft.get().trim().is_empty())
                        on:click=quote>{move || t(locale.get(), "excerpt.attach")}</button>
                </div>
            </div>
        </div>
    }
}
