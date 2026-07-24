use crate::app_support::{compose_icon, copy_text, RpCodeView};
use crate::bindings::{invoke, invoke_checked, reveal_saved_mark};
use crate::dto::{LibraryItem, LibraryItemDetail, LibraryItemVersion};
use crate::i18n::{t, Locale};
use crate::text::event_target_value;
use leptos::*;
use serde_wasm_bindgen::to_value;
use wasm_bindgen::JsValue;

/// Right-pane list of this session's saved text excerpts ("划线").
/// Like `NotebookView`, this takes plain data: component bodies are untracked,
/// so the caller reads the library/session signals and re-renders us.
#[component]
pub(super) fn HighlightsPane(
    locale: Locale,
    excerpts: Vec<LibraryItem>,
    on_library_changed: Callback<()>,
) -> impl IntoView {
    if excerpts.is_empty() {
        return view! {
            <div class="rp-empty highlights-empty">
                <span class="rp-empty-icon"></span>
                <div class="rp-empty-title">{t(locale, "right.no_highlights.title")}</div>
                <p>{t(locale, "right.no_highlights.body")}</p>
            </div>
        }
        .into_view();
    }
    view! {
        <div class="highlights-list">
            {excerpts.into_iter().map(|item| {
                let reveal = item.code.clone();
                let copy = item.code.clone();
                let delete_id = item.id.clone();
                view! {
                    <article class="highlight-card">
                        <button type="button" class="highlight-text"
                            title=t(locale, "highlight.reveal")
                            on:click=move |_| reveal_saved_mark(&reveal)>
                            {item.code.clone()}
                        </button>
                        <div class="highlight-actions">
                            <button type="button" class="icon-btn"
                                title=t(locale, "ctx.copy")
                                aria-label=t(locale, "ctx.copy")
                                on:click=move |_| copy_text(copy.clone())>{compose_icon("copy")}</button>
                            <button type="button" class="icon-btn starred"
                                title=t(locale, "library.remove")
                                aria-label=t(locale, "library.remove")
                                on:click=move |_| {
                                    let id = delete_id.clone();
                                    spawn_local(async move {
                                        let args = to_value(&serde_json::json!({ "id": id })).unwrap();
                                        if invoke_checked("delete_library_item", args).await.is_ok() {
                                            on_library_changed.call(());
                                        }
                                    });
                                }>{compose_icon("star-filled")}</button>
                        </div>
                    </article>
                }
            }).collect_view()}
        </div>
    }
    .into_view()
}

#[component]
pub(super) fn LibraryScreen(
    locale: ReadSignal<Locale>,
    items: ReadSignal<Vec<LibraryItem>>,
    on_close: Callback<()>,
    on_open_source: Callback<(String, String)>,
    on_changed: Callback<()>,
) -> impl IntoView {
    let query = create_rw_signal(String::new());
    let filter = create_rw_signal("all");
    let selected = create_rw_signal(None::<LibraryItemDetail>);
    let loading = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);

    let open_item = Callback::new(move |id: String| {
        loading.set(true);
        error.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "id": id })).unwrap();
            match invoke_checked("get_library_item", args).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<LibraryItemDetail>(value) {
                    Ok(detail) => selected.set(Some(detail)),
                    Err(_) => error.set(Some(
                        t(locale.get_untracked(), "library.read_failed").into(),
                    )),
                },
                Err(_) => error.set(Some(
                    t(locale.get_untracked(), "library.read_failed").into(),
                )),
            }
            loading.set(false);
        });
    });

    let delete_item = Callback::new(move |id: String| {
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "id": id })).unwrap();
            if invoke_checked("delete_library_item", args).await.is_ok() {
                selected.set(None);
                on_changed.call(());
            }
        });
    });

    view! {
        <section class="library-screen" data-testid="library-screen">
            <header class="library-head">
                <div>
                    <h1>{move || t(locale.get(), "library.title")}</h1>
                    <p>{move || t(locale.get(), "library.subtitle")}</p>
                </div>
                <button type="button" class="icon-btn library-close"
                    title=move || t(locale.get(), "library.close")
                    aria-label=move || t(locale.get(), "library.close")
                    on:click=move |_| on_close.call(())>{compose_icon("close")}</button>
            </header>
            <div class="library-toolbar">
                <label class="library-search">
                    <span class="gi search" aria-hidden="true"></span>
                    <input type="search"
                        aria-label=move || t(locale.get(), "library.search")
                        placeholder=move || t(locale.get(), "library.search")
                        prop:value=move || query.get()
                        on:input=move |event| query.set(event_target_value(&event)) />
                </label>
                <div class="library-filters" role="group" aria-label=move || t(locale.get(), "library.filter")>
                    {[ ("all", "library.all"), ("figure", "library.figures"), ("code", "library.code"), ("text", "library.texts") ]
                        .into_iter()
                        .map(|(value, key)| view! {
                            <button type="button" class:active=move || filter.get() == value
                                on:click=move |_| filter.set(value)>{move || t(locale.get(), key)}</button>
                        }).collect_view()}
                </div>
            </div>
            {move || error.get().map(|message| view! { <div class="library-error" role="alert">{message}</div> })}
            <div class="library-list">
                {move || {
                    let needle = query.get().trim().to_lowercase();
                    let selected_filter = filter.get();
                    let visible = items.get().into_iter().filter(|item| {
                        (selected_filter == "all" || item.kind == selected_filter)
                            && (needle.is_empty()
                                || item.title.to_lowercase().contains(&needle)
                                || item.code.to_lowercase().contains(&needle)
                                || item.source_project_name.to_lowercase().contains(&needle)
                                || item.source_session_title.to_lowercase().contains(&needle))
                    }).collect::<Vec<_>>();
                    if visible.is_empty() {
                        return view! {
                            <div class="library-empty">
                                {compose_icon("star")}
                                <h2>{t(locale.get(), "library.empty.title")}</h2>
                                <p>{t(locale.get(), "library.empty.body")}</p>
                            </div>
                        }.into_view();
                    }
                    visible.into_iter().map(|item| {
                        let id = item.id.clone();
                        let source_project = item.source_project_id.clone();
                        let source_session = item.source_session_id.clone();
                        let is_figure = item.kind == "figure";
                        let excerpt = item.code.lines().take(4).collect::<Vec<_>>().join("\n");
                        view! {
                            <article class="library-card" data-library-kind=item.kind.clone()>
                                <button type="button" class="library-card-main"
                                    on:click=move |_| open_item.call(id.clone())>
                                    <span class="library-card-icon">
                                        {compose_icon(if is_figure { "image" } else { "doc" })}
                                    </span>
                                    <span class="library-card-body">
                                        <span class="library-card-title">{item.title.clone()}</span>
                                        {(!excerpt.is_empty()).then(|| view! { <pre>{excerpt.clone()}</pre> })}
                                        <span class="library-card-meta">
                                            {format!("{} / {}", item.source_project_name, item.source_session_title)}
                                        </span>
                                    </span>
                                    <span class="library-card-kind">
                                        {if is_figure {
                                            t(locale.get_untracked(), "library.figure")
                                        } else if item.kind == "text" {
                                            t(locale.get_untracked(), "library.text")
                                        } else {
                                            item.language.as_deref().unwrap_or("code").to_string()
                                        }}
                                    </span>
                                </button>
                                <button type="button" class="library-source"
                                    title=move || t(locale.get(), "library.open_source")
                                    on:click=move |_| {
                                        on_close.call(());
                                        on_open_source.call((source_project.clone(), source_session.clone()));
                                    }>
                                    {move || t(locale.get(), "library.open_source")}
                                </button>
                            </article>
                        }
                    }).collect_view().into_view()
                }}
            </div>
            {move || loading.get().then(|| view! { <div class="library-loading">{t(locale.get(), "loading")}</div> })}
            {move || selected.get().map(|detail| {
                let item = detail.item;
                let delete_id = item.id.clone();
                let project_id = item.source_project_id.clone();
                let session_id = item.source_session_id.clone();
                let code_copy = item.code.clone();
                let image_src = detail.base64.map(|base64| format!(
                    "data:{};base64,{base64}",
                    item.content_type.as_deref().unwrap_or("application/octet-stream")
                ));
                let is_figure = item.kind == "figure";
                view! {
                    <div class="overlay library-detail-overlay" on:click=move |_| selected.set(None)>
                        <div class="modal library-detail" on:click=|event| event.stop_propagation()>
                            <header>
                                <div>
                                    <h2>{item.title.clone()}</h2>
                                    <span>{format!("{} / {}", item.source_project_name, item.source_session_title)}</span>
                                </div>
                                <div class="library-detail-actions">
                                    <button type="button" class="icon-btn starred"
                                        title=move || t(locale.get(), "library.remove")
                                        aria-label=move || t(locale.get(), "library.remove")
                                        on:click=move |_| delete_item.call(delete_id.clone())>
                                        {compose_icon("star-filled")}
                                    </button>
                                    <button type="button" class="icon-btn"
                                        title=move || t(locale.get(), "library.close")
                                        aria-label=move || t(locale.get(), "library.close")
                                        on:click=move |_| selected.set(None)>{compose_icon("close")}</button>
                                </div>
                            </header>
                            {if is_figure {
                                image_src.map(|src| view! {
                                    <div class="library-figure"><img src=src alt=item.title.clone() /></div>
                                }).unwrap_or_else(|| view! {
                                    <div class="library-error">{t(locale.get_untracked(), "library.read_failed")}</div>
                                }).into_view()
                            } else if item.kind == "text" {
                                view! { <div class="library-text">{item.code.clone()}</div> }.into_view()
                            } else {
                                view! { <CodeVersionPanel locale=locale item=item.clone() /> }.into_view()
                            }}
                            {(is_figure && !item.code.is_empty()).then(|| view! {
                                <section class="library-generating-code">
                                    <div class="library-code-head">
                                        <h3>{move || t(locale.get(), "library.generating_code")}</h3>
                                        <button type="button" class="icon-btn"
                                            title=move || t(locale.get(), "tool.copy_code")
                                            aria-label=move || t(locale.get(), "tool.copy_code")
                                            on:click=move |_| copy_text(code_copy.clone())>{compose_icon("copy")}</button>
                                    </div>
                                    <RpCodeView lang=item.language.clone().unwrap_or_default() body=item.code.clone() />
                                </section>
                            })}
                            <footer>
                                <button type="button" class="btn-ghost" on:click=move |_| {
                                    selected.set(None);
                                    on_close.call(());
                                    on_open_source.call((project_id.clone(), session_id.clone()));
                                }>{move || t(locale.get(), "library.open_source")}</button>
                            </footer>
                        </div>
                    </div>
                }
            })}
        </section>
    }
}

/// Version-aware code view for a library detail modal (#455). The item's
/// stored snapshot is the immutable version 1; saving an edit appends a new
/// version and never rewrites history. Shows the newest version by default
/// with read-only switching to older ones.
#[component]
fn CodeVersionPanel(locale: ReadSignal<Locale>, item: LibraryItem) -> impl IntoView {
    let seed = LibraryItemVersion {
        id: item.id.clone(),
        item_id: item.id.clone(),
        version_number: 1,
        parent_version_id: None,
        language: item.language.clone(),
        code: item.code.clone(),
        origin: "original".into(),
        created_at: item.created_at,
    };
    let versions = create_rw_signal(vec![seed]);
    let shown = create_rw_signal(0usize);
    let editing = create_rw_signal(false);
    let draft = create_rw_signal(String::new());
    let saving = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);

    let load = {
        let item_id = item.id.clone();
        Callback::new(move |_: ()| {
            let item_id = item_id.clone();
            spawn_local(async move {
                let args = to_value(&serde_json::json!({ "id": item_id })).unwrap();
                if let Ok(value) = invoke_checked("list_library_item_versions", args).await {
                    if let Ok(list) =
                        serde_wasm_bindgen::from_value::<Vec<LibraryItemVersion>>(value)
                    {
                        if !list.is_empty() {
                            shown.set(list.len() - 1);
                            versions.set(list);
                        }
                    }
                }
            });
        })
    };
    load.call(());

    let save = {
        let item_id = item.id.clone();
        Callback::new(move |_: ()| {
            let code = draft.get_untracked();
            if code.trim().is_empty() || saving.get_untracked() {
                return;
            }
            let item_id = item_id.clone();
            saving.set(true);
            spawn_local(async move {
                let args = to_value(&serde_json::json!({ "id": item_id, "code": code })).unwrap();
                match invoke_checked("update_library_code", args).await {
                    Ok(_) => {
                        editing.set(false);
                        error.set(None);
                        load.call(());
                    }
                    Err(_) => error.set(Some(
                        t(locale.get_untracked(), "library.edit_failed").into(),
                    )),
                }
                saving.set(false);
            });
        })
    };

    view! {
        <div class="library-code-panel">
            {move || {
                let list = versions.get();
                (list.len() > 1).then(|| view! {
                    <div class="library-versions library-filters" role="group"
                        aria-label=t(locale.get_untracked(), "library.versions")>
                        {list.into_iter().enumerate().map(|(index, version)| {
                            let label = if version.version_number == 1 {
                                format!("v1 · {}", t(locale.get_untracked(), "library.version_original"))
                            } else {
                                format!("v{}", version.version_number)
                            };
                            view! {
                                <button type="button" class:active=move || shown.get() == index
                                    on:click=move |_| { editing.set(false); shown.set(index); }>
                                    {label}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                })
            }}
            {move || error.get().map(|message| view! { <div class="library-error" role="alert">{message}</div> })}
            {move || {
                let list = versions.get();
                let current = list
                    .get(shown.get())
                    .or_else(|| list.last())
                    .cloned()
                    .expect("versions is seeded non-empty");
                let language = current.language.clone().unwrap_or_default();
                if editing.get() {
                    view! {
                        <textarea class="library-edit-area" prop:value=move || draft.get()
                            on:input=move |ev| draft.set(event_target_value(&ev))></textarea>
                        <div class="library-edit-actions">
                            <button type="button" class="btn-ghost"
                                on:click=move |_| editing.set(false)>
                                {t(locale.get_untracked(), "library.cancel")}
                            </button>
                            <button type="button" class="btn-primary" disabled=move || saving.get()
                                on:click=move |_| save.call(())>
                                {t(locale.get_untracked(), "library.save")}
                            </button>
                        </div>
                    }.into_view()
                } else {
                    let copy_code = current.code.clone();
                    let draft_seed = current.code.clone();
                    view! {
                        <div class="library-code-head">
                            <h3>{format!("v{}", current.version_number)}</h3>
                            <div class="library-detail-actions">
                                <button type="button" class="icon-btn"
                                    title=t(locale.get_untracked(), "tool.copy_code")
                                    aria-label=t(locale.get_untracked(), "tool.copy_code")
                                    on:click=move |_| copy_text(copy_code.clone())>{compose_icon("copy")}</button>
                                <button type="button" class="icon-btn"
                                    title=t(locale.get_untracked(), "library.edit")
                                    aria-label=t(locale.get_untracked(), "library.edit")
                                    on:click=move |_| { draft.set(draft_seed.clone()); editing.set(true); }>{compose_icon("edit")}</button>
                            </div>
                        </div>
                        <RpCodeView lang=language body=current.code.clone() />
                    }.into_view()
                }
            }}
        </div>
    }
}

pub(super) fn refresh_library(items: RwSignal<Vec<LibraryItem>>) {
    spawn_local(async move {
        let value = invoke("list_library_items", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<LibraryItem>>(value) {
            items.set(list);
        }
    });
}
