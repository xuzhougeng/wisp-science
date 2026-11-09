use crate::app_support::compose_icon;
use crate::bindings::invoke_checked;
use crate::dto::{PublicationSourceChoice, PublicationSourcePage};
use crate::i18n::{t, tf, Locale};
use crate::publication::PublicationEvidenceSource;
use crate::text::event_target_value;
use leptos::*;
use serde_wasm_bindgen::to_value;
use wasm_bindgen::JsValue;

/// DOM selection offsets count UTF-16 code units; persisted anchors count UTF-8
/// bytes. Reject offsets inside a surrogate pair rather than moving the range.
fn utf16_byte_offset(text: &str, offset: u32) -> Option<usize> {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += ch.len_utf16() as u32;
    }
    (units == offset).then_some(text.len())
}

/// Card teaser for a transcript source: drops Markdown emphasis, code and
/// heading markers and collapses whitespace. The preview keeps the exact text.
fn source_snippet(text: &str) -> String {
    let cleaned: String = text.chars().filter(|ch| !matches!(ch, '*' | '`')).collect();
    let mut snippet = String::new();
    for word in cleaned
        .split_whitespace()
        .filter(|word| !word.chars().all(|ch| ch == '#'))
    {
        if !snippet.is_empty() {
            snippet.push(' ');
        }
        snippet.push_str(word);
        if snippet.chars().count() >= 160 {
            break;
        }
    }
    snippet.chars().take(160).collect()
}

/// Short label beside a source title: file version, speaker, or Run status.
fn source_detail(locale: Locale, source: &PublicationSourceChoice) -> String {
    match (source.kind.as_str(), source.detail.as_str()) {
        ("artifact_version", version) => format!("v{version}"),
        ("message_span", role @ ("user" | "assistant")) => {
            t(locale, &format!("publication.role.{role}"))
        }
        (_, detail) => detail.to_string(),
    }
}

/// 1-based range of the sources shown on the current page, if any.
fn source_range(offset: u32, shown: usize) -> Option<(u32, u32)> {
    (shown > 0).then_some((offset + 1, offset + shown as u32))
}

#[component]
pub(super) fn PublicationSourcePicker(
    locale: ReadSignal<Locale>,
    on_select: Callback<PublicationEvidenceSource>,
    on_advanced: Callback<()>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    let kind = create_rw_signal("files".to_string());
    let query = create_rw_signal(String::new());
    let offset = create_rw_signal(0u32);
    let page = create_rw_signal(PublicationSourcePage::default());
    let selected = create_rw_signal::<Option<PublicationSourceChoice>>(None);
    let range = create_rw_signal::<Option<(usize, usize)>>(None);
    let loading = create_rw_signal(false);
    let error = create_rw_signal::<Option<String>>(None);
    let generation = create_rw_signal(0u64);
    let preview = create_node_ref::<html::Textarea>();
    let load = Callback::new(move |_: ()| {
        generation.update(|v| *v += 1);
        let ticket = generation.get_untracked();
        loading.set(true);
        error.set(None);
        selected.set(None);
        range.set(None);
        let args = serde_json::json!({"kind": kind.get_untracked(), "query": query.get_untracked(), "offset": offset.get_untracked()});
        spawn_local(async move {
            let result = invoke_checked(
                "list_publication_sources",
                to_value(&args).unwrap_or(JsValue::UNDEFINED),
            )
            .await;
            if generation.try_get_untracked() != Some(ticket) {
                return;
            }
            match result {
                Ok(value) => match serde_wasm_bindgen::from_value(value) {
                    Ok(value) => page.set(value),
                    Err(e) => error.set(Some(e.to_string())),
                },
                Err(e) => error.set(Some(e.as_string().unwrap_or_else(|| format!("{e:?}")))),
            }
            loading.set(false);
        });
    });
    load.call(());
    let update_range = Callback::new(move |_: ()| {
        let Some(element) = preview.get() else {
            return;
        };
        let value = element.value();
        let offsets = element
            .selection_start()
            .ok()
            .flatten()
            .zip(element.selection_end().ok().flatten());
        range.set(offsets.and_then(|(start, end)| {
            let start = utf16_byte_offset(&value, start)?;
            let end = utf16_byte_offset(&value, end)?;
            (start < end).then_some((start, end))
        }));
    });
    view! {
        <section class="publication-source-picker" data-testid="publication-source-picker">
            <header class="publication-source-head">
                <div>
                    <h3>{move || t(locale.get(), "publication.choose_source")}</h3>
                    <p class="publication-help">{move || t(locale.get(), "publication.choose_source_hint")}</p>
                </div>
                <button type="button" class="secondary" data-testid="publication-source-cancel"
                    on:click=move |_| on_cancel.call(())>
                    {move || t(locale.get(), "publication.cancel")}
                </button>
            </header>
            <div class="publication-source-controls">
                <div class="publication-source-tabs">
                    {["files", "runs", "messages"].into_iter().map(|value| view! {
                        <button type="button" class:active=move || kind.get() == value
                            aria-pressed=move || (kind.get() == value).to_string()
                            on:click=move |_| { kind.set(value.into()); offset.set(0); load.call(()); }>
                            {move || t(locale.get(), &format!("publication.sources.{value}"))}
                        </button>
                    }).collect_view()}
                </div>
                <form class="publication-source-search" on:submit=move |event| { event.prevent_default(); offset.set(0); load.call(()); }>
                    <input aria-label=move || t(locale.get(), "publication.source_search")
                        placeholder=move || t(locale.get(), "publication.source_search")
                        prop:value=move || query.get() on:input=move |event| query.set(event_target_value(&event)) />
                    <button type="submit" class="secondary" disabled=move || loading.get()>{compose_icon("search")}{move || t(locale.get(), "publication.search")}</button>
                </form>
            </div>
            {move || error.get().map(|error| view! { <div class="publication-error" role="alert">{error}</div> })}
            <div class="publication-source-grid">
                <div class="publication-source-column">
                    <div class="publication-source-list" aria-busy=move || loading.get().to_string()>
                        {move || if loading.get() { view! { <p class="publication-help">{t(locale.get(), "publication.sources_loading")}</p> }.into_view() } else {
                            let choices = page.get().sources;
                            if choices.is_empty() { view! { <p class="publication-help">{t(locale.get(), "publication.sources_empty")}</p> }.into_view() } else {
                                let locale = locale.get();
                                choices.into_iter().map(|source| {
                                    let id = source.id.clone();
                                    let pressed_id = source.id.clone();
                                    let snippet = source.text.as_deref().map(source_snippet);
                                    let title = source.title.clone();
                                    let detail = source_detail(locale, &source);
                                    view! {
                                        <button type="button" class="publication-source-choice"
                                            class:active=move || selected.get().is_some_and(|source| source.id == id)
                                            aria-pressed=move || selected.get().is_some_and(|source| source.id == pressed_id).to_string()
                                            on:click=move |_| { selected.set(Some(source.clone())); range.set(None); }>
                                            <span class="publication-source-choice-head">
                                                <strong>{title}</strong>
                                                <span class="publication-source-detail">{detail}</span>
                                            </span>
                                            {snippet.map(|snippet| view! { <p>{snippet}</p> })}
                                        </button>
                                    }
                                }).collect_view()
                            }
                        }}
                    </div>
                    // Outside the scrolling list, so paging never needs a scroll to the last card.
                    <nav class="publication-source-pagination" data-testid="publication-source-pagination"
                        hidden=move || offset.get() == 0 && !page.get().has_more>
                        <button type="button" class="secondary" data-testid="publication-source-previous"
                            disabled=move || loading.get() || offset.get() == 0
                            on:click=move |_| { offset.update(|v| *v = v.saturating_sub(50)); load.call(()); }>
                            {compose_icon("chevron-left")}{move || t(locale.get(), "publication.previous")}
                        </button>
                        <span class="publication-source-range" aria-live="polite">
                            {move || if loading.get() { None } else {
                                source_range(offset.get(), page.with(|page| page.sources.len())).map(|(from, to)| tf(
                                    locale.get(),
                                    "publication.source_range",
                                    &[("from", &from.to_string()), ("to", &to.to_string())],
                                ))
                            }}
                        </span>
                        <button type="button" class="secondary" data-testid="publication-source-next"
                            disabled=move || loading.get() || !page.get().has_more
                            on:click=move |_| { offset.update(|v| *v += 50); load.call(()); }>
                            {move || t(locale.get(), "publication.next")}{compose_icon("chevron-right")}
                        </button>
                    </nav>
                </div>
                <div class="publication-source-preview">
                    {move || selected.get().map(|source| {
                        let title = source.title.clone();
                        view! {
                            <h3>{title}</h3>
                            {source.text.clone().map(|text| view! {
                                <p class="publication-help">{t(locale.get(), "publication.select_excerpt")}</p>
                                <textarea node_ref=preview readonly aria-label=t(locale.get(), "publication.message_preview")
                                    prop:value=text on:mouseup=move |_| update_range.call(())
                                    on:keyup=move |_| update_range.call(()) on:select=move |_| update_range.call(())></textarea>
                            })}
                            <p class="publication-help">{t(locale.get(), "publication.exact_hint")}</p>
                            <button type="button" class="primary" data-testid="publication-source-continue"
                                on:click=move |_| {
                                    let kind = match source.kind.as_str() { "artifact_version" => "artifact_version", "run" => "run", _ => "message_span" };
                                    let id = if let Some(text) = &source.text {
                                        let (start,end) = range.get_untracked().unwrap_or((0,text.len()));
                                        let mut locator = serde_json::json!({"byte_end":end,"byte_start":start,"frame_id":source.frame_id,"message_seq":source.message_seq});
                                        if let Some(hash) = &source.text_sha256 { locator["message_content_sha256"] = hash.clone().into(); }
                                        // Keep canonical key order even if serde_json's
                                        // preserve_order feature is enabled by another dependency.
                                        let fields: std::collections::BTreeMap<_, _> = locator.as_object().expect("message locator").iter().collect();
                                        serde_json::to_string(&fields).expect("message locator is serializable")
                                    } else { source.id.clone() };
                                    on_select.call(PublicationEvidenceSource {kind,id,label:source.title.clone()});
                                }>{move || t(locale.get(), "publication.explain_use")}</button>
                        }.into_view()
                    }).unwrap_or_else(|| view! { <p class="publication-help publication-source-placeholder">{t(locale.get(), "publication.choose_source_first")}</p> }.into_view())}
                </div>
            </div>
            <details class="publication-advanced"><summary>{move || t(locale.get(), "publication.advanced")}</summary>
                <button type="button" class="secondary" data-testid="add-precise-publication-evidence"
                    on:click=move |_| on_advanced.call(())>{move || t(locale.get(), "publication.add_precise")}</button>
            </details>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::{source_range, source_snippet, utf16_byte_offset};
    #[test]
    fn transcript_selection_preserves_utf8_boundaries() {
        let text = "A水稻🌱结果";
        assert_eq!(utf16_byte_offset(text, 0), Some(0));
        assert_eq!(utf16_byte_offset(text, 3), Some(7));
        assert_eq!(utf16_byte_offset(text, 4), None);
        assert_eq!(utf16_byte_offset(text, 5), Some(11));
        assert_eq!(utf16_byte_offset(text, 7), Some(text.len()));
        assert_eq!(utf16_byte_offset(text, 8), None);
    }

    #[test]
    fn source_snippet_drops_markdown_markers_and_bounds_length() {
        assert_eq!(
            source_snippet("## 确认了。**`celltype_go/`** 就是\n\n这张图 `run_in_context`"),
            "确认了。celltype_go/ 就是 这张图 run_in_context"
        );
        assert_eq!(source_snippet("  \n "), "");
        assert_eq!(source_snippet(&"水稻 ".repeat(200)).chars().count(), 160);
    }

    #[test]
    fn source_range_is_one_based_and_empty_without_sources() {
        assert_eq!(source_range(0, 50), Some((1, 50)));
        assert_eq!(source_range(50, 7), Some((51, 57)));
        assert_eq!(source_range(50, 0), None);
    }
}
