use crate::app_support::{compose_icon, copy_text, RpCodeView};
use crate::bindings::invoke_checked;
use crate::dto::{ChatItem, LibraryItem};
use crate::i18n::{t, Locale};
use crate::text::fenced_blocks;
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum NotebookOrigin {
    Assistant,
    Repl,
    Shell,
}

#[derive(Clone, PartialEq)]
pub(super) struct NotebookProto {
    language: String,
    source: String,
    output: String,
    ok: Option<bool>,
    origin: NotebookOrigin,
}

#[derive(Clone, PartialEq)]
pub(super) struct NotebookCell {
    index: usize,
    language: String,
    source: String,
    output: String,
    ok: Option<bool>,
    origin: NotebookOrigin,
}

pub(super) type NotebookCache = HashMap<(usize, u64), Rc<Vec<NotebookProto>>>;

fn item_notebook_protos(item: &ChatItem) -> Vec<NotebookProto> {
    match item {
        ChatItem::Assistant { text, .. } => fenced_blocks(text)
            .into_iter()
            .filter(|(language, _)| !matches!(language.as_str(), "csv" | "tsv" | "fasta" | "fa"))
            .map(|(language, source)| NotebookProto {
                language: if language.is_empty() {
                    "text".into()
                } else {
                    language
                },
                source,
                output: String::new(),
                ok: None,
                origin: NotebookOrigin::Assistant,
            })
            .collect(),
        ChatItem::Tool {
            name,
            input,
            output,
            ok,
            ..
        } if matches!(name.as_str(), "python" | "r" | "shell") && !input.trim().is_empty() => {
            let source = if matches!(name.as_str(), "python" | "r") {
                input
                    .strip_prefix('[')
                    .and_then(|value| value.split_once("] "))
                    .map(|(_, code)| code)
                    .unwrap_or(input)
                    .to_string()
            } else {
                input.clone()
            };
            vec![NotebookProto {
                language: match name.as_str() {
                    "python" => "python".into(),
                    "r" => "r".into(),
                    _ => "bash".into(),
                },
                source,
                output: output.clone(),
                ok: *ok,
                origin: if matches!(name.as_str(), "python" | "r") {
                    NotebookOrigin::Repl
                } else {
                    NotebookOrigin::Shell
                },
            }]
        }
        _ => Vec::new(),
    }
}

/// Build a notebook projection from transcript code without introducing a
/// second persistence model. Per-item extraction is cached for streaming turns.
pub(super) fn collect_notebook_cells(
    items: &[ChatItem],
    cache: &mut NotebookCache,
) -> Vec<NotebookCell> {
    let mut next = NotebookCache::with_capacity(items.len());
    let mut per_item = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let key = (index, item.fingerprint());
        let protos = cache
            .remove(&key)
            .unwrap_or_else(|| Rc::new(item_notebook_protos(item)));
        next.insert(key, protos.clone());
        per_item.push(protos);
    }
    *cache = next;

    // A code fence that is subsequently executed is one cell, not two. Keep
    // every actual execution so intentionally repeated REPL calls remain visible.
    let executed: HashSet<String> = per_item
        .iter()
        .flat_map(|protos| protos.iter())
        .filter(|proto| proto.origin != NotebookOrigin::Assistant)
        .map(|proto| proto.source.trim().to_string())
        .collect();

    per_item
        .into_iter()
        .flat_map(|protos| protos.iter().cloned().collect::<Vec<_>>())
        .filter(|proto| {
            proto.origin != NotebookOrigin::Assistant || !executed.contains(proto.source.trim())
        })
        .enumerate()
        .map(|(index, proto)| NotebookCell {
            index,
            language: proto.language,
            source: proto.source,
            output: proto.output,
            ok: proto.ok,
            origin: proto.origin,
        })
        .collect()
}

#[component]
pub(super) fn NotebookView(
    cells: Vec<NotebookCell>,
    locale: Locale,
    active_session: ReadSignal<Option<String>>,
    library_items: ReadSignal<Vec<LibraryItem>>,
    on_library_changed: Callback<()>,
) -> impl IntoView {
    if cells.is_empty() {
        return view! {
            <div class="rp-empty notebook-empty">
                <span class="rp-empty-icon"></span>
                <div class="rp-empty-title">{t(locale, "right.no_notebook.title")}</div>
                <p>{t(locale, "right.no_notebook.body")}</p>
            </div>
        }
        .into_view();
    }

    view! {
        <div class="notebook-cells">
            {cells.into_iter().map(|cell| {
                let copy = cell.source.clone();
                let language = cell.language.clone();
                let source = cell.source.clone();
                let star_language = cell.language.clone();
                let star_source = cell.source.clone();
                let starred = create_memo(move |_| {
                    active_session.get().is_some_and(|session| {
                        library_items.get().iter().any(|item| {
                            item.matches_code(&session, &star_language, &star_source)
                        })
                    })
                });
                let click_language = cell.language.clone();
                let click_source = cell.source.clone();
                let has_output = !cell.output.is_empty();
                let output = cell.output.clone();
                let output_open = cell.ok == Some(false);
                let runtime = match cell.origin {
                    NotebookOrigin::Assistant => t(locale, "notebook.assistant"),
                    NotebookOrigin::Repl => "repl".into(),
                    NotebookOrigin::Shell => "shell".into(),
                };
                let status = match cell.ok {
                    Some(true) => "ok",
                    Some(false) => "error",
                    None if cell.origin == NotebookOrigin::Assistant => "source",
                    None => "running",
                };
                view! {
                    <section class="notebook-cell" data-cell-index=cell.index>
                        <header class="notebook-cell-head">
                            <span class="notebook-index">{format!("[{}]", cell.index)}</span>
                            <span class="notebook-language">{language.clone()}</span>
                            <span class="spacer"></span>
                            <span class=format!("notebook-status {status}") title=status></span>
                            <span class="notebook-runtime">{format!("{runtime} · wisp-science")}</span>
                            <button type="button" class="notebook-star" class:starred=move || starred.get()
                                disabled=move || active_session.get().is_none()
                                title=move || t(locale, if starred.get() { "library.remove" } else { "library.add" })
                                aria-label=move || t(locale, if starred.get() { "library.remove" } else { "library.add" })
                                aria-pressed=move || starred.get().to_string()
                                on:click=move |_| {
                                    let Some(session_id) = active_session.get_untracked() else { return; };
                                    let existing = library_items.get_untracked().into_iter().find(|item| {
                                        item.matches_code(&session_id, &click_language, &click_source)
                                    });
                                    let language = click_language.clone();
                                    let code = click_source.clone();
                                    spawn_local(async move {
                                        let (command, args) = match existing {
                                            Some(item) => (
                                                "delete_library_item",
                                                serde_json::json!({ "id": item.id }),
                                            ),
                                            None => (
                                                "star_library_code",
                                                serde_json::json!({
                                                    "sessionId": session_id,
                                                    "language": language,
                                                    "code": code,
                                                }),
                                            ),
                                        };
                                        if invoke_checked(command, to_value(&args).unwrap()).await.is_ok() {
                                            on_library_changed.call(());
                                        }
                                    });
                                }>
                                {move || compose_icon(if starred.get() { "star-filled" } else { "star" })}
                            </button>
                            <button type="button" class="notebook-copy"
                                title=t(locale, "tool.copy_code")
                                aria-label=t(locale, "tool.copy_code")
                                on:click=move |_| copy_text(copy.clone())>{compose_icon("copy")}</button>
                        </header>
                        <div class="notebook-source">
                            <RpCodeView lang=language body=source />
                        </div>
                        {has_output.then(|| view! {
                            <details class="notebook-output" open=output_open>
                                <summary>{t(locale, "notebook.output")}</summary>
                                <pre>{output}</pre>
                            </details>
                        })}
                    </section>
                }
            }).collect_view()}
        </div>
    }
    .into_view()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notebook_prefers_executed_cells_and_ignores_data_fences() {
        let items = vec![
            ChatItem::Assistant {
                text: "```python\nprint(1)\n```\n```rust\nfn main() {}\n```\n```csv\na,b\n1,2\n```"
                    .into(),
                model: None,
            },
            ChatItem::Tool {
                name: "python".into(),
                ok: Some(true),
                input: "print(1)".into(),
                output: "1".into(),
                started_at_ms: None,
                duration_ms: None,
            },
        ];

        let cells = collect_notebook_cells(&items, &mut NotebookCache::new());
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].language, "rust");
        assert_eq!(cells[1].language, "python");
        assert_eq!(cells[1].output, "1");
    }

    #[test]
    fn notebook_projects_r_calls_and_strips_runtime_context_preview() {
        let items = vec![ChatItem::Tool {
            name: "r".into(),
            ok: Some(true),
            input: "[r @ ssh:omics] summary(dataset)".into(),
            output: "Length  Class".into(),
            started_at_ms: None,
            duration_ms: None,
        }];
        let cells = collect_notebook_cells(&items, &mut NotebookCache::new());
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].language, "r");
        assert_eq!(cells[0].source, "summary(dataset)");
    }
}
