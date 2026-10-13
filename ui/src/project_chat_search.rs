use crate::{
    bindings::invoke_checked,
    compose_icon,
    dto::*,
    event_target_value, format_message_datetime,
    i18n::{t, tf, Locale},
    ime_composing, js_error_text, scroll_picker_item,
};
use leptos::{leptos_dom::helpers::TimeoutHandle, *};
use serde_wasm_bindgen::to_value;
use std::{cell::Cell, rc::Rc, time::Duration};
use wasm_bindgen::JsCast;

/// Some existing modal branches occur later in the app's Escape stack. Keep
/// them above this search when their actual CSS layer covers it (the shelved
/// collection is the exception: search is explicitly lifted above it).
pub(crate) fn covered_by_dialog() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Some(document) = window.document() else {
        return false;
    };
    let Some(surface) = document
        .query_selector(".project-chat-search-overlay")
        .ok()
        .flatten()
    else {
        return false;
    };
    let layer = |element: &web_sys::Element| {
        window
            .get_computed_style(element)
            .ok()
            .flatten()
            .and_then(|style| style.get_property_value("z-index").ok())
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or_default()
    };
    let search_layer = layer(&surface);
    let Ok(dialogs) = document.query_selector_all(".overlay, .session-timer-backdrop") else {
        return false;
    };
    (0..dialogs.length()).any(|index| {
        dialogs
            .item(index)
            .and_then(|node| node.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|element| {
                let bounds = element.get_bounding_client_rect();
                bounds.width() > 0.0 && bounds.height() > 0.0 && layer(&element) > search_layer
            })
    })
}

/// Escape is owned by the app stack. Each mounted instance belongs to exactly
/// one project; cleanup invalidates both the debounce and in-flight replies.
#[component]
pub(crate) fn ProjectChatSearch(
    project_id: String,
    project_name: String,
    locale: RwSignal<Locale>,
    on_close: Callback<()>,
    on_open: Callback<ProjectChatSearchHit>,
) -> impl IntoView {
    let query = create_rw_signal(String::new());
    let items = create_rw_signal(Vec::<ProjectChatSearchHit>::new());
    let cursor = create_rw_signal(None::<ProjectChatSearchCursor>);
    let loading = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);
    let active = create_rw_signal(0usize);
    let generation = Rc::new(Cell::new(0u64));
    let alive = Rc::new(Cell::new(true));
    let timer = Rc::new(Cell::new(None::<TimeoutHandle>));
    let cleanup_alive = alive.clone();
    let cleanup_timer = timer.clone();
    on_cleanup(move || {
        cleanup_alive.set(false);
        if let Some(timer) = cleanup_timer.take() {
            timer.clear();
        }
    });
    let fetch_generation = generation.clone();
    let fetch = Callback::new(move |next: Option<ProjectChatSearchCursor>| {
        let ticket = fetch_generation.get();
        let generation = fetch_generation.clone();
        let alive = alive.clone();
        let append = next.is_some();
        let args = to_value(&serde_json::json!({
            "projectId": project_id,
            "query": query.get_untracked(),
            "cursor": next,
        }))
        .unwrap();
        loading.set(true);
        error.set(None);
        spawn_local(async move {
            let result = invoke_checked("search_project_chats", args)
                .await
                .map_err(js_error_text)
                .and_then(|value| {
                    serde_wasm_bindgen::from_value::<ProjectChatSearchPage>(value)
                        .map_err(|e| e.to_string())
                });
            if !alive.get() || ticket != generation.get() {
                return;
            }
            match result {
                Ok(page) => {
                    if append {
                        items.update(|rows| rows.extend(page.items));
                    } else {
                        items.set(page.items);
                    }
                    cursor.set(page.next_cursor);
                }
                Err(message) => error.set(Some(message)),
            }
            loading.set(false);
        });
    });
    create_effect(move |_| {
        let q = query.get();
        generation.set(generation.get().wrapping_add(1));
        if let Some(timer) = timer.take() {
            timer.clear();
        }
        items.set(Vec::new());
        cursor.set(None);
        error.set(None);
        active.set(0);
        loading.set(!q.trim().is_empty());
        if !q.trim().is_empty() {
            timer.set(
                set_timeout_with_handle(move || fetch.call(None), Duration::from_millis(250)).ok(),
            );
        }
    });
    let input = create_node_ref::<html::Input>();
    create_effect(move |_| {
        if let Some(input) = input.get() {
            let _ = input.focus();
        }
    });
    view! {
        <div class="project-search-overlay project-chat-search-overlay" on:click=move |_| on_close.call(())>
            <section class="project-search-dialog project-chat-search-dialog" role="dialog" aria-modal="true"
                aria-label=move || t(locale.get(), "chat_search.title") on:click=|ev| ev.stop_propagation()>
                <header>
                    <div><h2>{move || t(locale.get(), "chat_search.title")}</h2><p>{project_name}</p></div>
                    <button type="button" class="icon-btn" aria-label=move || t(locale.get(), "chat_search.close")
                        on:click=move |_| on_close.call(())>{compose_icon("close")}</button>
                </header>
                <div class="project-search-input">
                    {compose_icon("search")}
                    <input node_ref=input type="search" maxlength="256" autocomplete="off" spellcheck="false"
                        aria-label=move || t(locale.get(), "chat_search.keyword")
                        placeholder=move || t(locale.get(), "chat_search.keyword")
                        prop:value=move || query.get() on:input=move |ev| query.set(event_target_value(&ev))
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            if ime_composing(&ev) { return; }
                            let n = items.with_untracked(Vec::len);
                            match ev.key().as_str() {
                                "ArrowDown" | "ArrowUp" if n > 0 => {
                                    ev.prevent_default();
                                    let next = (active.get_untracked() + if ev.key() == "ArrowDown" { 1 } else { n-1 }) % n;
                                    active.set(next);
                                    scroll_picker_item(".project-chat-search-result", next);
                                }
                                "Enter" if n > 0 => {
                                    ev.prevent_default();
                                    if let Some(hit) = items.with_untracked(|rows| rows.get(active.get_untracked()).cloned()) {
                                        on_open.call(hit);
                                    }
                                }
                                _ => {}
                            }
                        } />
                </div>
                <p class="project-chat-search-hint">{move || t(locale.get(), "chat_search.scope")}</p>
                <div class="project-chat-search-results" aria-busy=move || loading.get().to_string()>
                    {move || items.get().into_iter().enumerate().map(|(index, hit)| {
                        let selected = hit.clone();
                        let role = match hit.role.as_str() {
                            "user" => "chat_search.user", "tool" => "chat_search.tool", _ => "chat_search.assistant",
                        };
                        view! {
                            <button type="button" class="project-chat-search-result" class:active=move || active.get() == index
                                data-session-id=hit.session_id data-message-seq=hit.message_seq
                                on:mousemove=move |_| active.set(index) on:click=move |_| on_open.call(selected.clone())>
                                <span class="project-chat-search-meta"><strong>{hit.session_title}</strong>
                                    <span>{t(locale.get(), role)}</span><time>{format_message_datetime(hit.ts, locale.get())}</time></span>
                                <span class="project-chat-search-snippet">{hit.before}<mark>{hit.matched}</mark>{hit.after}</span>
                            </button>
                        }
                    }).collect_view()}
                </div>
                <div class="project-chat-search-status" role="status" aria-live="polite">
                    {move || if loading.get() {
                        t(locale.get(), "chat_search.loading").to_string()
                    } else if query.get().trim().is_empty() {
                        t(locale.get(), "chat_search.start").to_string()
                    } else if error.get().is_some() {
                        String::new()
                    } else if items.with(Vec::is_empty) {
                        t(locale.get(), "chat_search.empty").to_string()
                    } else {
                        tf(locale.get(), "chat_search.count", &[("n", &items.with(Vec::len).to_string())])
                    }}
                </div>
                {move || error.get().map(|message| view! {
                    <div class="project-chat-search-error" role="alert"><p>{message}</p>
                        <button type="button" class="btn-ghost" on:click=move |_| fetch.call(cursor.get_untracked())>
                            {move || t(locale.get(), "chat_search.retry")}</button></div>
                })}
                {move || (cursor.get().is_some() && error.get().is_none()).then(|| view! {
                    <button type="button" class="btn-ghost project-chat-search-more" disabled=move || loading.get()
                        on:click=move |_| fetch.call(cursor.get_untracked())>{move || t(locale.get(), "chat_search.more")}</button>
                })}
            </section>
        </div>
    }
}
