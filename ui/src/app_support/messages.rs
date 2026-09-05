use super::*;
use leptos::leptos_dom::helpers::TimeoutHandle;
use std::{cell::Cell, rc::Rc};

const STREAMING_MARKDOWN_TAIL_THRESHOLD_BYTES: usize = 8_000;

pub(crate) fn streaming_markdown_commit_interval_ms(
    text_len: usize,
    recent_parse_cost_ms: Option<f64>,
) -> u64 {
    if let Some(cost_ms) = recent_parse_cost_ms.filter(|cost| cost.is_finite()) {
        // Keep Markdown parsing below roughly one sixth of the main-thread
        // budget. The live plain-text tail still advances on every delta flush.
        return (cost_ms.max(0.0) * 6.0).ceil().clamp(50.0, 1_200.0) as u64;
    }

    // Cold-start fallback before the first measured parse. Length is only a
    // guard here: structure and device speed decide the steady-state cadence.
    if text_len >= 128_000 {
        600
    } else if text_len >= 32_000 {
        300
    } else if text_len >= STREAMING_MARKDOWN_TAIL_THRESHOLD_BYTES {
        150
    } else {
        50
    }
}

fn assistant_text_at(items: RwSignal<Vec<ChatItem>>, source_item: usize) -> String {
    items.with_untracked(|rows| match rows.get(source_item) {
        Some(ChatItem::Assistant { text, .. }) => text.clone(),
        _ => String::new(),
    })
}

/// Keep completed Markdown readable while an append-only assistant stream grows.
/// Parsing is throttled by answer size; once the answer is large, the unparsed
/// suffix remains visible as a cheap whitespace-preserving text tail.
#[component]
pub(crate) fn StreamingAssistantMessage(
    items: RwSignal<Vec<ChatItem>>,
    source_item: usize,
    on_artifact: Callback<usize>,
    on_file: Callback<ModalArtifact>,
) -> impl IntoView {
    let locale = use_locale();
    let rendered_text = create_rw_signal(assistant_text_at(items, source_item));
    let commit_handle = Rc::new(Cell::new(None::<TimeoutHandle>));
    let active = Rc::new(Cell::new(true));
    let recent_parse_cost_ms = Rc::new(Cell::new(None::<f64>));
    let project = use_context::<ReadSignal<Option<ProjectInfo>>>();

    create_render_effect({
        let commit_handle = Rc::clone(&commit_handle);
        let active = Rc::clone(&active);
        let recent_parse_cost_ms = Rc::clone(&recent_parse_cost_ms);
        move |_| {
            let (changed, text_len) = items.with(|rows| match rows.get(source_item) {
                Some(ChatItem::Assistant { text, .. }) => (
                    rendered_text.with_untracked(|rendered| rendered != text),
                    text.len(),
                ),
                _ => (false, 0),
            });
            if !changed || commit_handle.get().is_some() {
                return;
            }

            let delay = std::time::Duration::from_millis(streaming_markdown_commit_interval_ms(
                text_len,
                recent_parse_cost_ms.get(),
            ));
            let callback_handle = Rc::clone(&commit_handle);
            let callback_active = Rc::clone(&active);
            match leptos::set_timeout_with_handle(
                move || {
                    callback_handle.set(None);
                    if !callback_active.get() {
                        return;
                    }
                    let latest = assistant_text_at(items, source_item);
                    if rendered_text.get_untracked() != latest {
                        rendered_text.set(latest);
                    }
                },
                delay,
            ) {
                Ok(handle) => commit_handle.set(Some(handle)),
                Err(_) => rendered_text.set(assistant_text_at(items, source_item)),
            }
        }
    });

    on_cleanup({
        let commit_handle = Rc::clone(&commit_handle);
        move || {
            active.set(false);
            if let Some(handle) = commit_handle.take() {
                handle.clear();
            }
        }
    });

    let html = create_memo({
        let recent_parse_cost_ms = Rc::clone(&recent_parse_cost_ms);
        move |_| {
            let started_at = js_sys::Date::now();
            let project_root =
                project.and_then(|project| project.get().map(|project| project.root));
            let html = enrich_md_html(
                md_to_html(&rendered_text.get()),
                &[],
                &[],
                locale.get(),
                project_root.as_deref(),
            );
            let elapsed = (js_sys::Date::now() - started_at).max(0.0);
            let smoothed = recent_parse_cost_ms
                .get()
                .map_or(elapsed, |previous| previous * 0.7 + elapsed * 0.3);
            recent_parse_cost_ms.set(Some(smoothed));
            html
        }
    });
    let pending_text = create_memo(move |_| {
        let rendered = rendered_text.get();
        items.with(|rows| match rows.get(source_item) {
            Some(ChatItem::Assistant { text, .. })
                if text.len() >= STREAMING_MARKDOWN_TAIL_THRESHOLD_BYTES =>
            {
                text.strip_prefix(rendered.as_str())
                    .unwrap_or_default()
                    .to_string()
            }
            _ => String::new(),
        })
    });
    let hid = unique_dom_id("stream-md");
    // `inner_html` replaces the whole parsed prefix on every commit. Running
    // highlight.js and KaTeX here would therefore rescan and mutate an
    // increasingly large, short-lived DOM tree each time. The settled
    // `AssistantMessage` performs that post-processing once when the stream
    // completes; during streaming the Markdown structure and cheap text tail
    // remain live without paying the syntax-decoration cost repeatedly.

    view! {
        <div class="assistant-wrap">
            <div
                class="body streaming streaming-markdown"
                data-rendered-bytes=move || rendered_text.with(|text| text.len().to_string())
                data-pending-bytes=move || pending_text.with(|text| text.len().to_string())
            >
                <div
                    class="streaming-markdown-prefix md"
                    id=hid
                    inner_html=move || html.get()
                    on:click=move |ev: web_sys::MouseEvent| {
                        handle_md_click(&ev, &[], &[], &on_artifact, &on_file)
                    }
                ></div>
                {move || {
                    let tail = pending_text.get();
                    (!tail.is_empty()).then(|| view! {
                        <div class="streaming-markdown-tail">{tail}</div>
                    })
                }}
            </div>
        </div>
    }
}

#[component]
pub(crate) fn SessionStatusBadge(
    status: SessionStatusKind,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let key = status.i18n_key();
    let class = status.css();
    view! {
        <span class=format!("sess-status sess-status-{class}")>
            {move || t(locale.get(), key)}
        </span>
    }
}

/// Shared Lucide-style UI icons. Interactive controls must use these SVGs,
/// never font glyphs whose shape varies by platform and fallback font.
pub(crate) fn compose_icon(kind: &str) -> impl IntoView {
    let body = match kind {
        "attach" => view! { <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/> }.into_view(),
        "folder" => view! { <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/> }.into_view(),
        "plan" => view! { <path d="M8 6h13"/><path d="M8 12h13"/><path d="M8 18h13"/><path d="M3 6l1 1 2-2"/><path d="M3 12l1 1 2-2"/><path d="M3 18l1 1 2-2"/> }.into_view(),
        "chat" => view! { <path d="M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4z"/><path d="M8 10h8"/><path d="M8 14h5"/> }.into_view(),
        "branch" => view! { <path d="M6 3v6a4 4 0 0 0 4 4h8"/><path d="M18 7v12"/><path d="M14 15l4 4 4-4"/><circle cx="6" cy="3" r="2"/> }.into_view(),
        "flask" => view! { <path d="M10 2v7.3"/><path d="M14 9.3V2"/><path d="M8.5 2h7"/><path d="m10 9.3-6.5 10.8a1 1 0 0 0 .9 1.5h15.2a1 1 0 0 0 .9-1.5L14 9.3"/><path d="M6.5 16h11"/> }.into_view(),
        "dna" => view! { <path d="M4 3c5 0 11 18 16 18"/><path d="M20 3C15 3 9 21 4 21"/><path d="M7 6h10"/><path d="M5 10h14"/><path d="M5 14h14"/><path d="M7 18h10"/> }.into_view(),
        "arrow-left" => view! { <path d="M19 12H5"/><path d="m12 19-7-7 7-7"/> }.into_view(),
        "folder-plus" => view! { <path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/> }.into_view(),
        "book" => view! { <path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/> }.into_view(),
        "gear" => view! { <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/> }.into_view(),
        "bubble" => view! { <path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/> }.into_view(),
        "sparkles" => view! { <path d="m12 3-1.9 5.8a2 2 0 0 1-1.3 1.3L3 12l5.8 1.9a2 2 0 0 1 1.3 1.3L12 21l1.9-5.8a2 2 0 0 1 1.3-1.3L21 12l-5.8-1.9a2 2 0 0 1-1.3-1.3Z"/> }.into_view(),
        "undo" => view! { <path d="M9 14 4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 6 6v1"/> }.into_view(),
        "panel" => view! { <rect x="3" y="3" width="18" height="18" rx="2"/><path d="M15 3v18"/> }.into_view(),
        "dock" => view! { <rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 15h18"/> }.into_view(),
        "chevron-down" => view! { <path d="m6 9 6 6 6-6"/> }.into_view(),
        "chevron-left" => view! { <path d="m15 18-6-6 6-6"/> }.into_view(),
        "chevron-right" => view! { <path d="m9 18 6-6-6-6"/> }.into_view(),
        "expand" => view! { <path d="M15 3h6v6"/><path d="m21 3-7 7"/><path d="M9 21H3v-6"/><path d="m3 21 7-7"/> }.into_view(),
        "download" => view! { <path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/> }.into_view(),
        "upload" => view! { <path d="M12 21V9"/><path d="m7 14 5-5 5 5"/><path d="M5 3h14"/> }.into_view(),
        "share" => view! { <circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><path d="m8.59 13.51 6.83 3.98"/><path d="m15.41 6.51-6.83 3.98"/> }.into_view(),
        "sync" => view! { <path d="M20 7h-9"/><path d="m16 3 4 4-4 4"/><path d="M4 17h9"/><path d="m8 21-4-4 4-4"/> }.into_view(),
        "loader" => view! { <circle cx="12" cy="12" r="9" opacity="0.16"/><path d="M21 12a9 9 0 0 0-9-9"/><path d="M21 12a9 9 0 0 0-5.2-8.2" opacity="0.4"/> }.into_view(),
        "circle-alert" => view! { <circle cx="12" cy="12" r="10"/><path d="M12 8v4"/><path d="M12 16h.01"/> }.into_view(),
        "pin" => view! { <path d="M12 17v5"/><path d="M5 17h14"/><path d="m6 3 1 7-3 4h16l-3-4 1-7Z"/> }.into_view(),
        "link" => view! { <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/> }.into_view(),
        "bell" => view! { <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/> }.into_view(),
        "close" => view! { <path d="M18 6 6 18"/><path d="m6 6 12 12"/> }.into_view(),
        "more" => view! { <circle cx="12" cy="5" r="1" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="1" fill="currentColor" stroke="none"/><circle cx="12" cy="19" r="1" fill="currentColor" stroke="none"/> }.into_view(),
        "minus" => view! { <path d="M5 12h14"/> }.into_view(),
        "database" => view! { <ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5"/><path d="M3 12a9 3 0 0 0 18 0"/> }.into_view(),
        "trash" => view! { <path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M10 11v6"/><path d="M14 11v6"/> }.into_view(),
        "plus" => view! { <path d="M12 5v14"/><path d="M5 12h14"/> }.into_view(),
        "crop" => view! { <path d="M6 2v14a2 2 0 0 0 2 2h14"/><path d="M2 6h14a2 2 0 0 1 2 2v14"/> }.into_view(),
        "split" => view! { <rect x="3" y="4" width="18" height="16" rx="2"/><path d="M14 4v16"/> }.into_view(),
        "runtime-panel" => view! { <rect x="3" y="3" width="18" height="18" rx="2"/><path d="M14 3v18"/><path d="M3 15h11"/><circle cx="17.5" cy="7" r="1" fill="currentColor" stroke="none"/><circle cx="17.5" cy="11" r="1" fill="currentColor" stroke="none"/> }.into_view(),
        "play" => view! { <path d="M6 4.5v15l13-7.5Z"/> }.into_view(),
        "file-play" => view! { <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z"/><path d="M14 2v6h6"/><path d="M10 12.5v5l4.5-2.5Z"/> }.into_view(),
        "bolt" => view! { <path d="M13 2 3 14h8l-1 8 10-12h-8l1-8z"/> }.into_view(),
        "up" => view! { <path d="m18 15-6-6-6 6"/> }.into_view(),
        "copy" => view! { <rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/> }.into_view(),
        "clipboard" => view! { <rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/> }.into_view(),
        "star" => view! { <path d="m12 2.7 2.85 5.77 6.37.93-4.61 4.49 1.09 6.34L12 17.23l-5.7 3 1.09-6.34L2.78 9.4l6.37-.93Z"/> }.into_view(),
        "star-filled" => view! { <path d="m12 2.7 2.85 5.77 6.37.93-4.61 4.49 1.09 6.34L12 17.23l-5.7 3 1.09-6.34L2.78 9.4l6.37-.93Z" fill="currentColor"/> }.into_view(),
        "edit" => view! { <path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L8 18l-4 1 1-4Z"/> }.into_view(),
        "code" => view! { <path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/> }.into_view(),
        "doc" => view! { <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z"/><path d="M14 2v6h6"/> }.into_view(),
        "image" => view! { <rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="m21 15-5-5L5 21"/> }.into_view(),
        "video" => view! { <path d="m22 8-6 4 6 4V8Z"/><rect x="2" y="6" width="14" height="12" rx="2"/> }.into_view(),
        "review" => view! { <circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 1 0 18Z" fill="currentColor" stroke="none"/> }.into_view(),
        "memory" => view! { <path d="M12 2a7 7 0 0 0-7 7v2a4 4 0 0 0-2 3.46V18a2 2 0 0 0 2 2h3"/><path d="M12 2a7 7 0 0 1 7 7v2a4 4 0 0 1 2 3.46V18a2 2 0 0 1-2 2h-3"/><path d="M9 9h6"/><path d="M9 13h6"/><path d="M12 17v5"/> }.into_view(),
        "gauge" => view! { <path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/> }.into_view(),
        "controls" => view! { <path d="M4 21v-7"/><path d="M4 10V3"/><path d="M12 21v-9"/><path d="M12 8V3"/><path d="M20 21v-5"/><path d="M20 12V3"/><path d="M1 14h6"/><path d="M9 8h6"/><path d="M17 16h6"/> }.into_view(),
        "adjustments" => view! { <path d="M4 7h9"/><path d="M17 7h3"/><circle cx="15" cy="7" r="2"/><path d="M4 17h3"/><path d="M11 17h9"/><circle cx="9" cy="17" r="2"/> }.into_view(),
        "check" => view! { <path d="m20 6-11 11-5-5"/> }.into_view(),
        "skill" => view! { <path d="M19 17V5a2 2 0 0 0-2-2H4"/><path d="M8 21h12a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1H11a1 1 0 0 0-1 1v1a2 2 0 1 1-4 0V5a2 2 0 1 0-4 0v2a1 1 0 0 0 1 1h3"/> }.into_view(),
        "computer" => view! { <rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/> }.into_view(),
        "server" => view! { <rect x="3" y="4" width="18" height="7" rx="1"/><rect x="3" y="13" width="18" height="7" rx="1"/><circle cx="7" cy="7.5" r="0.5" fill="currentColor"/><circle cx="7" cy="16.5" r="0.5" fill="currentColor"/> }.into_view(),
        "monitor" => view! { <rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/> }.into_view(),
        "user" => view! { <path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/> }.into_view(),
        "wrench" => view! { <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/> }.into_view(),
        "clock" => view! { <circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/> }.into_view(),
        "sort" => view! { <path d="m21 16-4 4-4-4"/><path d="M17 20V4"/><path d="m3 8 4-4 4 4"/><path d="M7 4v16"/> }.into_view(),
        "search" => view! { <circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/> }.into_view(),
        "eye" => view! { <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/> }.into_view(),
        "eye-off" => view! { <path d="m3 3 18 18"/><path d="M10.6 10.6a2 2 0 0 0 2.8 2.8"/><path d="M9.9 4.2A10.8 10.8 0 0 1 12 4c5 0 9 5 9 8a12.4 12.4 0 0 1-2 3.7"/><path d="M6.6 6.6C4.4 8 3 10.2 3 12c0 3 4 8 9 8a10.4 10.4 0 0 0 4.2-.9"/> }.into_view(),
        "terminal" => view! { <path d="m4 17 6-5-6-5"/><path d="M12 19h8"/> }.into_view(),
        "grid" => view! { <rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/> }.into_view(),
        "list" => view! { <path d="M8 6h13"/><path d="M8 12h13"/><path d="M8 18h13"/><path d="M3 6h.01"/><path d="M3 12h.01"/><path d="M3 18h.01"/> }.into_view(),
        "timeline" => view! { <path d="M22 12h-4l-3 9L9 3l-3 9H2"/> }.into_view(),
        "compact" => view! { <path d="M4 14h6v6"/><path d="M20 10h-6V4"/><path d="m14 10 7-7"/><path d="m3 21 7-7"/> }.into_view(),
        "fork" => view! { <circle cx="12" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><circle cx="18" cy="6" r="3"/><path d="M18 9v2c0 .6-.5 1-1 1H7c-.6 0-1-.4-1-1V9"/><path d="M12 12v3"/> }.into_view(),
        "save" => view! { <path d="M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7"/><path d="M7 3v4a1 1 0 0 0 1 1h7"/> }.into_view(),
        "shield" => view! { <path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/> }.into_view(),
        _ => view! { <path d="M9 18l6-6-6-6"/> }.into_view(), // chevron
    };
    let size = if matches!(
        kind,
        "chevron" | "chevron-down" | "chevron-left" | "chevron-right" | "arrow-left"
    ) {
        "16"
    } else {
        "18"
    };
    view! {
        <svg width=size height=size viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>
    }
}

/// A `generate_image` call owns a stable media slot in the transcript. The
/// ToolCall event paints the placeholder immediately; ToolResult remounts the
/// same card and loads the PNG into that slot.
#[component]
pub(crate) fn ImageGenerationCard(
    path: String,
    ok: Option<bool>,
    output: String,
    on_file: Callback<ModalArtifact>,
) -> impl IntoView {
    let locale = use_locale();
    let source = create_rw_signal(None::<String>);
    let preview_failed = create_rw_signal(false);
    if ok == Some(true) {
        let load_path = path.clone();
        spawn_local(async move {
            // Full-resolution blob object URL from the shared cache — a data
            // URL here meant ~1.33x the file size as a string in the DOM.
            match crate::bindings::media_url(&load_path).await.as_string() {
                Some(url) => {
                    let _ = source.try_set(Some(url));
                }
                None => {
                    let _ = preview_failed.try_set(true);
                }
            }
        });
    }

    let status = match ok {
        None => "running",
        Some(true) => "completed",
        Some(false) => "failed",
    };
    let title_key = match ok {
        None => "chat.image_generating",
        Some(true) => "chat.image_generated",
        Some(false) => "chat.image_failed",
    };
    let display_path = path.clone();
    let open_path = path.clone();
    let filename = attachment_name(&path);
    let open_name = filename.clone();
    let failure_detail = output.trim().to_string();

    view! {
        <article
            class="image-generation-card"
            data-testid="image-generation-card"
            data-status=status
            data-path=path
        >
            <div class="image-generation-media">
                {move || match ok {
                    None => view! {
                        <div class="image-generation-state" role="status" aria-live="polite">
                            <span class="image-generation-spinner" aria-hidden="true"></span>
                            <span>{move || t(locale.get(), "chat.image_generating")}</span>
                        </div>
                    }.into_view(),
                    Some(false) => view! {
                        <div class="image-generation-state failed">
                            <span class="image-generation-failed-mark" aria-hidden="true">"!"</span>
                            <span>{move || t(locale.get(), "chat.image_failed")}</span>
                            {(!failure_detail.is_empty()).then(|| view! {
                                <small title=failure_detail.clone()>{failure_detail.clone()}</small>
                            })}
                        </div>
                    }.into_view(),
                    Some(true) => match source.get() {
                        Some(src) => {
                            let click_path = open_path.clone();
                            let click_name = open_name.clone();
                            let open = on_file;
                            view! {
                                <button
                                    type="button"
                                    class="image-generation-open"
                                    aria-label=move || t(locale.get(), "chat.image_generated")
                                    on:click=move |_| open.call((
                                        click_path.clone(),
                                        click_name.clone(),
                                        "image".into(),
                                    ))
                                >
                                    <img
                                        src=src
                                        alt=filename.clone()
                                        on:error=move |_| {
                                            source.set(None);
                                            preview_failed.set(true);
                                        }
                                    />
                                </button>
                            }.into_view()
                        }
                        None if preview_failed.get() => view! {
                            <div class="image-generation-state failed">
                                <span class="image-generation-failed-mark" aria-hidden="true">"!"</span>
                                <span>{move || t(locale.get(), "chat.image_preview_unavailable")}</span>
                            </div>
                        }.into_view(),
                        None => view! {
                            <div class="image-generation-state" role="status" aria-live="polite">
                                <span class="image-generation-spinner" aria-hidden="true"></span>
                                <span>{move || t(locale.get(), "chat.image_loading")}</span>
                            </div>
                        }.into_view(),
                    },
                }}
            </div>
            <footer class="image-generation-meta">
                <strong>{move || t(locale.get(), title_key)}</strong>
                <code>{display_path}</code>
            </footer>
        </article>
    }
}

/// A `generate_video` call owns a stable media slot in the transcript, same
/// pattern as the image card: ToolCall paints the placeholder, ToolResult
/// remounts and streams the MP4 into an inline player.
#[component]
pub(crate) fn VideoGenerationCard(path: String, ok: Option<bool>, output: String) -> impl IntoView {
    let locale = use_locale();
    let source = create_rw_signal(None::<String>);
    let preview_failed = create_rw_signal(false);
    if ok == Some(true) {
        let load_path = path.clone();
        spawn_local(async move {
            // Blob object URL streamed by the browser's media stack. A 64 MB
            // MP4 inlined as base64 was ~85 MB of string in the DOM — the
            // worst offender of the renderer OOM reports.
            match crate::bindings::media_url(&load_path).await.as_string() {
                Some(url) => {
                    let _ = source.try_set(Some(url));
                }
                None => {
                    let _ = preview_failed.try_set(true);
                }
            }
        });
    }

    let status = match ok {
        None => "running",
        Some(true) => "completed",
        Some(false) => "failed",
    };
    let title_key = match ok {
        None => "chat.video_generating",
        Some(true) => "chat.video_generated",
        Some(false) => "chat.video_failed",
    };
    let display_path = path.clone();
    let filename = attachment_name(&path);
    let failure_detail = output.trim().to_string();

    view! {
        <article
            class="video-generation-card"
            data-testid="video-generation-card"
            data-status=status
            data-path=path
        >
            <div class="video-generation-media">
                {move || match ok {
                    None => view! {
                        <div class="video-generation-state" role="status" aria-live="polite">
                            <span class="video-generation-spinner" aria-hidden="true"></span>
                            <span>{move || t(locale.get(), "chat.video_generating")}</span>
                        </div>
                    }.into_view(),
                    Some(false) => view! {
                        <div class="video-generation-state failed">
                            <span class="video-generation-failed-mark" aria-hidden="true">"!"</span>
                            <span>{move || t(locale.get(), "chat.video_failed")}</span>
                            {(!failure_detail.is_empty()).then(|| view! {
                                <small title=failure_detail.clone()>{failure_detail.clone()}</small>
                            })}
                        </div>
                    }.into_view(),
                    Some(true) => match source.get() {
                        Some(src) => view! {
                            <video
                                class="video-generation-player"
                                controls
                                preload="metadata"
                                src=src
                                aria-label=filename.clone()
                                on:error=move |_| {
                                    source.set(None);
                                    preview_failed.set(true);
                                }
                            ></video>
                        }.into_view(),
                        None if preview_failed.get() => view! {
                            <div class="video-generation-state failed">
                                <span class="video-generation-failed-mark" aria-hidden="true">"!"</span>
                                <span>{move || t(locale.get(), "chat.video_preview_unavailable")}</span>
                            </div>
                        }.into_view(),
                        None => view! {
                            <div class="video-generation-state" role="status" aria-live="polite">
                                <span class="video-generation-spinner" aria-hidden="true"></span>
                                <span>{move || t(locale.get(), "chat.video_loading")}</span>
                            </div>
                        }.into_view(),
                    },
                }}
            </div>
            <footer class="video-generation-meta">
                <strong>{move || t(locale.get(), title_key)}</strong>
                <code>{display_path}</code>
            </footer>
        </article>
    }
}

/// Small, lazy image preview shared by composer cards and sent messages. The
/// source is a cached, downscaled blob object URL (see `media_thumbnail_url`)
/// and gracefully falls back to an image icon when a native path cannot be
/// read from the active project.
#[component]
pub(crate) fn AttachmentThumbnail(path: String, alt: String) -> impl IntoView {
    let source = create_rw_signal(None::<String>);
    let path_for_effect = path;
    create_effect(move |_| {
        let path = path_for_effect.clone();
        spawn_local(async move {
            let url = crate::bindings::media_thumbnail_url(&path)
                .await
                .as_string();
            let _ = source.try_set(url);
        });
    });
    view! {
        <span class="attachment-thumbnail">
            {move || source.get().map_or_else(
                || view! { <span class="attachment-thumbnail-placeholder">{compose_icon("image")}</span> }.into_view(),
                |src| view! { <img src=src alt=alt.clone() /> }.into_view(),
            )}
        </span>
    }
}

/// Tile face for an in-thread artifact card: a real thumbnail for image
/// artifacts, the kind badge for everything else. The badge is the base layer
/// rather than an alternative branch, so an image whose bytes never arrive (or
/// fail to decode) falls back to exactly the badge card these tiles replaced.
#[component]
fn ArtifactThumb(path: Option<String>, kind: &'static str) -> impl IntoView {
    // A per-card unique id, so the async thumbnail fill below can address this
    // exact mount. The generated-card list rebuilds whenever the shared
    // artifact signal changes; a rebuild disposes the previous owner, and a
    // signal write from the disposed task is silently dropped by design. DOM
    // direct fill keeps the async result visible for as long as this node is
    // in the document, independent of the reactive owner that started it.
    let dom_id = unique_dom_id("art-thumb");
    if let Some(path) = path.filter(|_| kind == "image") {
        // Thumbnail-sized card: the shared downscaled blob URL also accepts
        // the artifact:/version:/ssh:// spellings `load_file_content` does.
        let dom_id_for_load = dom_id.clone();
        spawn_local(async move {
            let url = crate::bindings::media_thumbnail_url(&path)
                .await
                .as_string();
            let Some(url) = url else { return };
            let Some(el) = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.get_element_by_id(&dom_id_for_load))
            else {
                return;
            };
            let Some(img) = append_thumb_image(&el) else {
                return;
            };
            // A failed load (e.g. its blob URL was evicted meanwhile) removes
            // the img so the kind badge stays the fallback.
            let img_for_error = img.clone();
            let handler = wasm_bindgen::closure::Closure::once(move || {
                if let Some(parent) = img_for_error.parent_element() {
                    let _ = parent.remove_child(&img_for_error).ok();
                }
            });
            let _ = img.add_event_listener_with_callback("error", handler.as_ref().unchecked_ref());
            let _ = img.set_attribute("src", &url);
        });
    }
    view! {
        <span class="message-artifact-thumb" id=dom_id.clone()>
            <span class=format!("rp-badge {kind}")>{kind}</span>
        </span>
    }
}

/// The `<img>` for a filled thumbnail is created lazily inside the thumb span,
/// after the badge, so an image whose bytes never arrive (or fail to decode)
/// falls back to exactly the badge card these tiles replaced.
fn append_thumb_image(parent: &web_sys::Element) -> Option<web_sys::Element> {
    let img = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.create_element("img").ok())?;
    parent.append_child(&img).ok()?;
    Some(img)
}

/// Queue (#433): an operation on a parked follow-up, raised from its bubble and
/// handled by the parent (which owns the transcript signals + invoke).
#[derive(Clone)]
pub(crate) enum QueueOp {
    /// Drop it from the queue.
    Cancel(u64),
    /// Fold it into the running turn now (native sessions only).
    CutIn(u64),
    /// Unqueue it and restore its text to the composer.
    Edit(u64),
    /// Swap one place earlier / later in the FIFO order (clamped at the ends).
    MoveUp(u64),
    MoveDown(u64),
}

/// Queue (#433): one parked follow-up in the composer card. `id == 0` is a
/// transient cut-in (no controls). `can_cut_in` is false for ACP sessions.
/// Reorder controls stay hidden when the queue has only one item.
#[component]
pub(crate) fn QueuedMessage(
    id: u64,
    text: String,
    user_index: usize,
    can_cut_in: bool,
    can_reorder: bool,
    on_queue: Callback<QueueOp>,
) -> impl IntoView {
    let locale = use_locale();
    let show_controls = id != 0;
    let preview = text.clone();
    view! {
        <div class="msg user queued" data-user-index=user_index.to_string()>
            <div class="queued-card">
                <div class="body" title=preview>{text}</div>
                {show_controls.then(move || view! {
                    <div class="queue-actions">
                        {can_cut_in.then(|| view! {
                            <button type="button" class="queue-cut-in"
                                on:click=move |_| on_queue.call(QueueOp::CutIn(id))>
                                {compose_icon("bolt")}
                                <span>{move || t(locale.get(), "queue.cut_in")}</span>
                            </button>
                        })}
                        {can_reorder.then(|| view! {
                            <button type="button" class="msg-icon-btn"
                                title=move || t(locale.get(), "queue.move_up")
                                aria-label=move || t(locale.get(), "queue.move_up")
                                on:click=move |_| on_queue.call(QueueOp::MoveUp(id))>
                                {compose_icon("up")}
                            </button>
                            <button type="button" class="msg-icon-btn"
                                title=move || t(locale.get(), "queue.move_down")
                                aria-label=move || t(locale.get(), "queue.move_down")
                                on:click=move |_| on_queue.call(QueueOp::MoveDown(id))>
                                {compose_icon("chevron-down")}
                            </button>
                        })}
                        <button type="button" class="msg-icon-btn"
                            title=move || t(locale.get(), "queue.edit")
                            aria-label=move || t(locale.get(), "queue.edit")
                            on:click=move |_| on_queue.call(QueueOp::Edit(id))>
                            {compose_icon("edit")}
                        </button>
                        <button type="button" class="msg-icon-btn queue-remove"
                            title=move || t(locale.get(), "queue.remove")
                            aria-label=move || t(locale.get(), "queue.remove")
                            on:click=move |_| on_queue.call(QueueOp::Cancel(id))>
                            {compose_icon("trash")}
                        </button>
                    </div>
                })}
            </div>
        </div>
    }
}

/// Compact card above the composer: parked follow-ups sit with the input
/// instead of as dashed transcript bubbles.
#[component]
pub(crate) fn ComposerQueue(
    items: RwSignal<Vec<ChatItem>>,
    user_offset: Signal<usize>,
    can_cut_in: Signal<bool>,
    on_queue: Callback<QueueOp>,
) -> impl IntoView {
    let locale = use_locale();
    view! {
        {move || {
            let rows = queued_turn_rows(&items.get(), user_offset.get());
            let len = rows.len();
            let can_cut_in = can_cut_in.get();
            let can_reorder = len > 1;
            let loc = locale.get();
            (!rows.is_empty()).then(move || view! {
                <div class="composer-queue" data-testid="composer-queue"
                    role="region"
                    aria-label=t(loc, "queue.region")>
                    <div class="composer-queue-head">
                        <span class="composer-queue-dot" aria-hidden="true"></span>
                        <span>{tf(loc, "queue.header", &[("n", &len.to_string())])}</span>
                    </div>
                    <div class="composer-queue-list">
                        {rows.into_iter().map(|row| {
                            view! {
                                <QueuedMessage
                                    id=row.id
                                    text=row.text
                                    user_index=row.user_index
                                    can_cut_in=can_cut_in
                                    can_reorder=can_reorder
                                    on_queue=on_queue
                                />
                            }
                        }).collect_view()}
                    </div>
                </div>
            })
        }}
    }
}

#[component]
pub(crate) fn UserMessage(
    text: String,
    timestamp: Option<i64>,
    ui_index: usize,
    busy: ReadSignal<bool>,
    can_modify: bool,
    can_branch: Signal<bool>,
    on_copy: Callback<String>,
    on_edit: Callback<usize>,
    on_branch: Callback<usize>,
    on_file: Callback<ModalArtifact>,
) -> impl IntoView {
    let locale = use_locale();
    let presentation = user_message_presentation(&text);
    let body = presentation.body;
    let (images, files): (Vec<_>, Vec<_>) = presentation
        .attachments
        .into_iter()
        .partition(|path| file_kind(path) == Some("image"));
    let has_images = !images.is_empty();
    let has_files = !files.is_empty();
    let has_context = !presentation.artifacts.is_empty()
        || !presentation.sessions.is_empty()
        || !presentation.projects.is_empty()
        || !presentation.skills.is_empty()
        || !presentation.workflows.is_empty()
        || !presentation.contexts.is_empty()
        || !presentation.runtimes.is_empty();
    let has_body = !body.is_empty();
    // 长消息先折叠，"展开全部"再看全文
    let is_long = body.lines().count() > 12 || body.chars().count() > 600;
    let (expanded, set_expanded) = create_signal(false);
    let body_short = is_long.then(|| {
        let head = body.lines().take(12).collect::<Vec<_>>().join("\n");
        let head = match head.char_indices().nth(600) {
            Some((cut, _)) => head[..cut].to_string(),
            None => head,
        };
        format!("{}…", head.trim_end())
    });
    let image_cards = images
        .into_iter()
        .map(|path| {
            let name = attachment_name(&path);
            let name_for_click = name.clone();
            let path_for_click = path.clone();
            let on_file = on_file.clone();
            view! {
                <button type="button" class="user-attachment-image"
                    title=name.clone()
                    on:click=move |_| on_file.call((path_for_click.clone(), name_for_click.clone(), "image".into()))>
                    <AttachmentThumbnail path=path alt=name.clone() />
                    <span class="user-attachment-image-name">{name}</span>
                </button>
            }
        })
        .collect_view();
    let file_cards = files
        .into_iter()
        .map(|path| {
            let name = attachment_name(&path);
            let name_for_click = name.clone();
            let kind = file_kind(&path).unwrap_or("text").to_string();
            let path_for_click = path.clone();
            let kind_for_click = kind.clone();
            let on_file = on_file.clone();
            view! {
                <button type="button" class="user-attachment-file"
                    title=path.clone()
                    on:click=move |_| on_file.call((path_for_click.clone(), name_for_click.clone(), kind_for_click.clone()))>
                    <span class="user-attachment-file-icon">{compose_icon("doc")}</span>
                    <span class="user-attachment-file-copy">
                        <span class="user-attachment-file-name">{name}</span>
                        <span class="user-attachment-file-meta">{move || t(locale.get(), "attachment.file")}</span>
                    </span>
                    <span class="user-attachment-open">{compose_icon("chevron-right")}</span>
                </button>
            }
        })
        .collect_view();
    let context_cards = [
        ("artifact", "attachment.artifact", presentation.artifacts),
        ("session", "attachment.session", presentation.sessions),
        ("project", "attachment.project", presentation.projects),
        ("skill", "attachment.skill", presentation.skills),
        (
            "workflow",
            "attachment.workflow",
            presentation.workflows,
        ),
        ("context", "attachment.context", presentation.contexts),
        ("runtime", "attachment.runtime", presentation.runtimes),
    ]
    .into_iter()
    .flat_map(|(kind, label_key, items)| {
        items.into_iter().map(move |label| {
            view! {
                <span class=format!("user-context-card {kind}") data-reference-kind=kind>
                    <span class="user-context-icon">{compose_icon(if kind == "skill" { "skill" } else if kind == "workflow" { "branch" } else if kind == "session" { "chat" } else if kind == "project" { "folder" } else if kind == "context" { "server" } else if kind == "runtime" { "terminal" } else { "doc" })}</span>
                    <span class="user-context-copy">
                        <span class="user-context-label">{label}</span>
                        <span class="user-context-meta">{move || t(locale.get(), label_key)}</span>
                    </span>
                </span>
            }
        })
    })
    .collect_view();
    view! {
        <div class="user-bubble"
            data-branch-ui-index=can_branch.get_untracked().then(|| ui_index.to_string())>
            {has_images.then(|| view! { <div class="user-attachment-images">{image_cards}</div> })}
            {has_files.then(|| view! { <div class="user-attachment-files">{file_cards}</div> })}
            {has_context.then(|| view! { <div class="user-context-cards">{context_cards}</div> })}
            {has_body.then(|| view! {
                <div class="body">{move || match (&body_short, expanded.get()) {
                    (Some(short), false) => short.clone(),
                    _ => body.clone(),
                }}</div>
            })}
            {(has_body && is_long).then(|| view! {
                <button
                    type="button"
                    class="msg-btn body-toggle"
                    on:click=move |_| set_expanded.update(|v| *v = !*v)
                >{move || t(locale.get(), if expanded.get() { "msg.show_less" } else { "msg.show_all" })}</button>
            })}
            {timestamp.map(|timestamp| {
                let compact = format_message_time(timestamp);
                view! {
                    <time
                        class="message-time user-message-time"
                        data-timestamp=timestamp.to_string()
                        title=move || tf(
                            locale.get(),
                            "msg.sent_at",
                            &[("time", &format_message_datetime(timestamp, locale.get()))],
                        )
                    >
                        {compact}
                    </time>
                }
            })}
            <div class="msg-actions">
                <button
                    type="button"
                    class="msg-btn"
                    disabled=move || busy.get()
                    title=move || t(locale.get(), "msg.copy")
                    on:click=move |_| on_copy.call(text.clone())
                >{move || t(locale.get(), "msg.copy")}</button>
                {can_modify.then(|| view! {
                    <button
                        type="button"
                        class="msg-btn"
                        disabled=move || busy.get()
                        title=move || t(locale.get(), "msg.edit")
                        on:click=move |_| on_edit.call(ui_index)
                    >{move || t(locale.get(), "msg.edit")}</button>
                })}
                {move || can_branch.get().then(|| view! {
                    <button
                        type="button"
                        class="msg-btn"
                        title=move || t(locale.get(), "msg.branch")
                        on:click=move |_| on_branch.call(ui_index)
                    >{move || t(locale.get(), "msg.branch")}</button>
                })}
            </div>
        </div>
    }
}

#[component]
pub(crate) fn AssistantMessage(
    text: String,
    model: Option<String>,
    timestamp: Option<i64>,
    resources: Vec<MessageResource>,
    artifacts: Memo<Vec<Artifact>>,
    source_item: usize,
    on_artifact: Callback<usize>,
    on_file: Callback<ModalArtifact>,
    on_copy: Callback<String>,
    on_memory: Callback<()>,
    on_review: Callback<()>,
    on_branch: Callback<usize>,
    can_branch: Signal<bool>,
    show_actions: Signal<bool>,
    can_undo: Signal<bool>,
    on_undo: Callback<usize>,
    show_explore: Signal<bool>,
    can_explore: Signal<bool>,
    explore_turn_index: usize,
    on_explore: Callback<usize>,
) -> impl IntoView {
    let locale = use_locale();
    let resources_for_html = resources.clone();
    let text_for_html = text.clone();
    let project = use_context::<ReadSignal<Option<ProjectInfo>>>();
    let html = create_memo(move |_| {
        let project_root = project.and_then(|project| project.get().map(|project| project.root));
        // Subscribe to the shared artifact list at row scope: an artifact
        // change recomputes only this memo, and String equality keeps the DOM
        // (plus the highlight/resource effects below) untouched for rows whose
        // enriched HTML did not actually change. This replaces the global
        // fingerprint that used to remount every assistant row on any artifact
        // event — the remount storm behind the dead-window reports.
        artifacts.with(|arts| {
            enrich_md_html(
                md_to_html(&text_for_html),
                arts,
                &resources_for_html,
                locale.get(),
                project_root.as_deref(),
            )
        })
    });
    let hid = unique_dom_id("md");
    let hid_for_effect = hid.clone();
    create_effect(move |_| {
        let _ = html.get();
        schedule_highlight(hid_for_effect.clone());
    });
    let hid_for_resources = hid.clone();
    let resources_for_effect = resources.clone();
    create_effect(move |_| {
        let _ = html.get();
        let dom_id = hid_for_resources.clone();
        let resources = resources_for_effect.clone();
        spawn_local(async move {
            for resource in resources
                .into_iter()
                .filter(|resource| resource.status == "ready" && resource.kind == "image")
            {
                let Some(version_id) = resource.artifact_version_id else {
                    continue;
                };
                // Cached blob object URL keyed by the immutable version path —
                // repeated mounts of the same image reuse one blob instead of
                // re-fetching and re-inlining its base64.
                let path = format!("artifact-version:{version_id}");
                let Some(url) = crate::bindings::media_url(&path).await.as_string() else {
                    continue;
                };
                let selector = format!(r#"#{dom_id} [data-resource-id="{}"]"#, resource.id);
                if let Some(element) = web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|document| document.query_selector(&selector).ok().flatten())
                {
                    let _ = element.set_attribute("src", &url);
                    let _ = element.set_attribute("class", "resource-inline-image");
                }
            }
        });
    });
    let on_artifact = on_artifact.clone();
    let on_artifact_for_cards = on_artifact.clone();
    let on_file = on_file.clone();
    let resources_for_click = resources.clone();
    let generated = create_memo(move |_| {
        artifacts.with(|arts| {
            arts.iter()
                .enumerate()
                .filter(|(_, artifact)| artifact.source_item == source_item)
                .map(|(index, artifact)| {
                    let path = match &artifact.data {
                        PreviewData::File { path, .. } => Some(path.clone()),
                        _ => None,
                    };
                    (
                        index,
                        artifact.name.clone(),
                        artifact.kind,
                        artifact.superseded,
                        path,
                    )
                })
                .collect::<Vec<_>>()
        })
    });
    let generated_count = move || generated.with(Vec::len);
    // Anything past this is folded behind "+N more". Kept in step with the
    // `nth-child(n+9)` rule in chat.css that does the hiding.
    let generated_overflow = move || generated_count().saturating_sub(8);
    let generated_expanded = create_rw_signal(false);
    let generated_collapsed = move || generated_overflow() > 0 && !generated_expanded.get();
    let generated_cards = move || {
        generated
            .get()
            .into_iter()
            .map(|(index, name, kind, superseded, path)| {
                let on_artifact = on_artifact_for_cards.clone();
                view! {
                    <button type="button" class="message-artifact-card" class:superseded=superseded
                        disabled=superseded
                        data-artifact-name=name.clone()
                        title=name.clone()
                        on:click=move |_| on_artifact.call(index)>
                        <ArtifactThumb path=path kind=kind />
                        <span class="message-artifact-name">{name}</span>
                        {superseded.then(|| view! { <span class="message-artifact-status">{move || t(locale.get(), "artifact.updated")}</span> })}
                    </button>
                }
            })
            .collect_view()
    };
    let text_for_disabled = text.clone();
    let text_for_click_copy = text;
    view! {
        <div class="role">
            <span class="role-brand">{move || t(locale.get(), "chat.assistant")}</span>
            {move || model.clone().filter(|m| !m.is_empty()).map(|m| view! {
                <span class="role-model">{m}</span>
            })}
            {timestamp.map(|timestamp| {
                let compact = format_message_time(timestamp);
                view! {
                    <time
                        class="message-time assistant-message-time"
                        data-timestamp=timestamp.to_string()
                        title=move || tf(
                            locale.get(),
                            "msg.replied_at",
                            &[("time", &format_message_datetime(timestamp, locale.get()))],
                        )
                    >
                        {compact}
                    </time>
                }
            })}
        </div>
        <div class="assistant-wrap">
            <div class="body md" id=hid.clone()
                inner_html=move || html.get()
                on:click=move |ev: web_sys::MouseEvent| {
                    let arts = artifacts.get_untracked();
                    handle_md_click(
                        &ev,
                        &arts,
                        &resources_for_click,
                        &on_artifact,
                        &on_file,
                    )
                }></div>
            {move || (generated_count() > 0).then(|| view! {
                <div class="message-artifacts">
                    <div class="message-artifacts-label">{move || format!("Generated · {}", generated_count())}</div>
                    <div class="message-artifact-cards"
                        class:collapsed=generated_collapsed>
                        {generated_cards}
                        {move || generated_collapsed().then(|| view! {
                            <button type="button" class="message-artifact-more"
                                on:click=move |_| generated_expanded.set(true)>
                                {move || tf(locale.get(), "artifact.more_count", &[("n", &generated_overflow().to_string())])}
                            </button>
                        })}
                    </div>
                </div>
            })}
            {move || {
                let text_for_disabled = text_for_disabled.clone();
                let text_for_click_copy = text_for_click_copy.clone();
                show_actions.get().then(move || view! { <div class="msg-actions">
                <button
                    type="button"
                    class="msg-icon-btn msg-memory-btn"
                    data-testid="remember-turn"
                    title=move || t(locale.get(), "msg.memory")
                    aria-label=move || t(locale.get(), "msg.memory")
                    on:click=move |_| on_memory.call(())
                >
                    {compose_icon("memory")}
                </button>
                <button
                    type="button"
                    class="msg-icon-btn msg-review-btn"
                    title=move || t(locale.get(), "msg.review")
                    aria-label=move || t(locale.get(), "msg.review")
                    on:click=move |_| on_review.call(())
                >
                    {compose_icon("review")}
                </button>
                {move || show_explore.get().then(|| view! { <button
                    type="button"
                    class="msg-icon-btn msg-explore-btn"
                    data-testid="start-exploration"
                    title=move || t(locale.get(), "exploration.start")
                    aria-label=move || t(locale.get(), "exploration.start")
                    disabled=move || !can_explore.get()
                    on:click=move |_| on_explore.call(explore_turn_index)
                >
                    {compose_icon("flask")}
                </button> })}
                {move || can_branch.get().then(|| view! { <button
                    type="button"
                    class="msg-icon-btn msg-branch-btn"
                    title=move || t(locale.get(), "msg.branch")
                    aria-label=move || t(locale.get(), "msg.branch")
                    on:click=move |_| on_branch.call(source_item)
                >
                    {compose_icon("branch")}
                </button> })}
                <button
                    type="button"
                    class="msg-icon-btn"
                    title=move || t(locale.get(), "ctx.copy_message")
                    aria-label=move || t(locale.get(), "ctx.copy_message")
                    disabled=move || text_for_disabled.trim().is_empty()
                    on:click=move |_| on_copy.call(text_for_click_copy.clone())
                >
                    {compose_icon("copy")}
                </button>
                {move || can_undo.get().then(|| view! {
                    <button
                        type="button"
                        class="msg-icon-btn"
                        title=move || t(locale.get(), "msg.undo")
                        aria-label=move || t(locale.get(), "msg.undo")
                        on:click=move |_| on_undo.call(source_item)
                    >
                        {compose_icon("undo")}
                    </button>
                })}
            </div> })}}
        </div>
    }
}

#[component]
pub(crate) fn ToolBlock(
    name: String,
    ok: Option<bool>,
    input: String,
    output: String,
) -> impl IntoView {
    let locale = use_locale();
    let expanded = create_rw_signal(ok != Some(true));
    let lang = tool_lang(&name).to_string();
    let hid = unique_dom_id("tool");
    let hid_for_effect = hid.clone();
    let has_input = !input.is_empty();
    let has_output = !output.is_empty();
    let (badge_key, title) = tool_card_label(&name, &input);
    let input = store_value(input);
    let output = store_value(output);
    let lang = store_value(lang);
    create_effect(move |_| {
        if expanded.get() {
            schedule_highlight(hid_for_effect.clone());
        }
    });
    let input_label_key = if matches!(name.as_str(), "python" | "r") {
        "tool.copy_code"
    } else {
        "tool.copy_input"
    };

    view! {
        <details class="tool" class:ext=badge_key.is_some() open=move || expanded.get()>
            <summary class="head" aria-expanded=move || expanded.get().to_string()
                on:click=move |event| {
                    event.prevent_default();
                    expanded.update(|open| *open = !*open);
                }>
                {badge_key.map(|key| view! {
                    <span class="tool-badge">{move || t(locale.get(), key)}</span>
                })}
                <span>{title}</span>
                {match ok {
                    Some(true) => view!{ <span class="ok">"✓"</span> }.into_view(),
                    Some(false) => view!{ <span class="fail">"✗"</span> }.into_view(),
                    None => view!{ <span class="run"><span class="run-dot"></span>{move || t(locale.get(), "tool.running")}</span> }.into_view(),
                }}
            </summary>
            {move || expanded.get().then(|| {
                let input = input.get_value();
                let output = output.get_value();
                let input_for_copy = input.clone();
                let output_for_copy = output.clone();
                let language = lang.get_value();
                view! {
                    <div class="tool-panel" id=hid.clone()>
                        <div class="tool-actions">
                            {has_input.then(|| view! {
                                <button type="button" class="tool-btn"
                                    on:click=move |_| copy_text(input_for_copy.clone())>
                                    {move || t(locale.get(), input_label_key)}
                                </button>
                            })}
                            {has_output.then(|| view! {
                                <button type="button" class="tool-btn"
                                    on:click=move |_| copy_text(output_for_copy.clone())>
                                    {move || t(locale.get(), "tool.copy_output")}
                                </button>
                            })}
                        </div>
                        {has_input.then(|| view! {
                            <pre class="tool-input md-code"><code class=format!("language-{language}")>{input}</code></pre>
                        })}
                        {has_output.then(|| view! {
                            <pre class="tool-output md-code"><code class="language-plaintext">{output}</code></pre>
                        })}
                    </div>
                }
            })}
        </details>
    }
}

/// Parse a rendered plan checklist line
/// (`[x] text` / `[~] text` / `[ ] text` / `[-] text`)
/// into (status_class, text). Mirrors `update_plan`'s render in wisp-tools.
pub(crate) fn plan_step_line(line: &str) -> Option<(&'static str, &str)> {
    for (prefix, cls) in [
        ("[x] ", "done"),
        ("[~] ", "running"),
        ("[ ] ", "pending"),
        ("[-] ", "cancelled"),
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some((cls, rest));
        }
    }
    None
}

fn plan_status_class(status: &str) -> &'static str {
    match status {
        "completed" | "done" => "done",
        "in_progress" | "running" => "running",
        "cancelled" => "cancelled",
        _ => "pending",
    }
}

/// New approvals carry structured steps so fenced code, blank lines, and
/// task lists cannot be mistaken for top-level checklist rows. Old persisted
/// approvals still parse the legacy marker format, including continuations.
pub(crate) fn parse_plan_steps(preview: &str) -> Vec<(&'static str, String)> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(preview) {
        if value.get("v").and_then(serde_json::Value::as_u64) == Some(1) {
            if let Some(steps) = value.get("steps").and_then(serde_json::Value::as_array) {
                return steps
                    .iter()
                    .filter_map(|step| {
                        let content = step.get("content")?.as_str()?.trim();
                        (!content.is_empty()).then(|| {
                            (
                                plan_status_class(
                                    step.get("status")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or("pending"),
                                ),
                                content.to_string(),
                            )
                        })
                    })
                    .collect();
            }
        }
    }

    let mut steps: Vec<(&'static str, String)> = vec![];
    for line in preview.lines() {
        if let Some((class, text)) = plan_step_line(line) {
            steps.push((class, text.to_string()));
        } else if let Some((_, text)) = steps.last_mut() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(line);
        }
    }
    steps
}

pub(crate) fn approval_allow_label_key(scope: &str) -> &'static str {
    match scope {
        "session" => "approval.allow_session",
        "project" => "approval.allow_project",
        "global" => "approval.allow_global",
        _ => "approval.allow_once",
    }
}

#[component]
pub(crate) fn ApprovalCard(
    tool: String,
    preview: String,
    message: String,
    session_id: String,
    on_decide: Callback<(String, bool, Option<String>, String)>,
    on_artifact: Callback<usize>,
    on_file: Callback<ModalArtifact>,
) -> impl IntoView {
    let locale = use_locale();
    let is_plan = tool == "update_plan";
    let is_resource_conflict = tool == "resource_conflict";
    let is_image_resize = tool == "image_resize";
    let show_feedback = create_rw_signal(false);
    let feedback = create_rw_signal(String::new());
    let approval_scope = create_rw_signal(String::from("once"));
    let feedback_ready = move || !feedback.get().trim().is_empty();
    if is_plan {
        window_capture_escape(move || {
            if !show_feedback.get_untracked() {
                return false;
            }
            feedback.set(String::new());
            show_feedback.set(false);
            true
        });
    }
    create_effect(move |_| {
        if show_feedback.get() {
            focus_element_soon("plan-feedback-input");
        }
    });
    let lang = tool_lang(&tool).to_string();
    // New approvals carry JSON; old persisted cards fall back to checklist text.
    let plan_steps = if is_plan {
        parse_plan_steps(&preview)
    } else {
        vec![]
    };
    let project_root = use_context::<ReadSignal<Option<ProjectInfo>>>()
        .and_then(|project| project.get().map(|project| project.root));
    let tool_for_title = tool.clone();
    let title = move || {
        let loc = locale.get();
        match tool_for_title.as_str() {
            _ if is_plan => t(loc, "approval.review_plan"),
            "resource_conflict" => t(loc, "approval.resource_conflict_title"),
            "image_resize" => t(loc, "approval.image_resize_title"),
            "python" => t(loc, "approval.run_python"),
            "r" => t(loc, "approval.run_r"),
            "shell" => t(loc, "approval.run_shell"),
            _ => tf(loc, "approval.run_tool", &[("tool", &tool_for_title)]),
        }
    };
    let sid_allow = session_id.clone();
    let sid_deny = session_id.clone();
    let sid_feedback = create_rw_signal(session_id);
    view! {
        <div class="approval-wrap">
            <div class="approval-wait-line">{move || t(locale.get(), "approval.waiting_line")}</div>
            <div class="approval-card" class:plan=is_plan>
                <div class="approval-head">
                    <span class="approval-title">{title}</span>
                    <span class="approval-status">
                        <span class="approval-dot"></span>
                        {move || t(locale.get(), "approval.waiting")}
                    </span>
                </div>
                {if is_plan {
                    view! {
                        <div class="plan-steps">
                            {plan_steps.into_iter().map(|(cls, text)| {
                                let html = enrich_md_html(
                                    md_to_html(&text),
                                    &[],
                                    &[],
                                    locale.get(),
                                    project_root.as_deref(),
                                );
                                let step_artifact = on_artifact.clone();
                                let step_file = on_file.clone();
                                view! {
                                    <div class=format!("plan-step {cls}")>
                                        <span class="plan-step-mark"></span>
                                        <div class="plan-step-text md" inner_html=html
                                            on:click=move |ev: web_sys::MouseEvent| {
                                                handle_md_click(&ev, &[], &[], &step_artifact, &step_file)
                                            }></div>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    }.into_view()
                } else {
                    let show_tag = !tool.is_empty() && !is_resource_conflict;
                    let tag = tool.clone();
                    let show_code = !preview.is_empty() && !is_resource_conflict;
                    let p = preview.clone();
                    let lang = lang.clone();
                    view! {
                        {(is_resource_conflict || is_image_resize).then(|| view! {
                            <p class="approval-conflict-message">{if is_image_resize && !p.is_empty() { p.clone() } else if is_image_resize { message.clone() } else { p.clone() }}</p>
                        })}
                        {show_tag.then(|| view! {
                            <div class="approval-tags"><span class="approval-tag">{tag}</span></div>
                        })}
                        {show_code.then(|| view! {
                            <details class="approval-code" open=true>
                                <summary>{move || t(locale.get(), "approval.code")}</summary>
                                <pre><code class=format!("language-{lang}")>{p}</code></pre>
                            </details>
                        })}
                    }.into_view()
                }}
                <p class="approval-hint">{move || t(locale.get(), if is_plan {
                    "approval.plan_hint"
                } else if is_resource_conflict {
                    "approval.resource_conflict_hint"
                } else if is_image_resize {
                    "approval.image_resize_hint"
                } else {
                    "approval.hint"
                })}</p>
                <div class="approval-actions">
                    {(!is_plan && !is_resource_conflict && !is_image_resize).then(|| view! {
                        <label class="approval-scope">
                            <span>{move || t(locale.get(), "approval.scope")}</span>
                            <select
                                aria-label=move || t(locale.get(), "approval.scope")
                                prop:value=move || approval_scope.get()
                                on:change=move |ev| approval_scope.set(dom_value(&ev))>
                                <option value="once">{move || t(locale.get(), "approval.scope.once")}</option>
                                <option value="session">{move || t(locale.get(), "approval.scope.session")}</option>
                                <option value="project">{move || t(locale.get(), "approval.scope.project")}</option>
                                <option value="global">{move || t(locale.get(), "approval.scope.global")}</option>
                            </select>
                        </label>
                    })}
                    <button type="button" class="primary"
                        on:click=move |_| {
                            let scope = if is_plan { "once".into() } else { approval_scope.get() };
                            on_decide.call((sid_allow.clone(), true, None, scope));
                        }>
                        {move || {
                            if is_plan {
                                t(locale.get(), "approval.plan_approve").to_string()
                            } else if is_resource_conflict {
                                t(locale.get(), "approval.resource_conflict_wait").to_string()
                            } else if is_image_resize {
                                t(locale.get(), "approval.image_resize_continue").to_string()
                            } else {
                                t(locale.get(), approval_allow_label_key(&approval_scope.get())).to_string()
                            }
                        }}
                    </button>
                    <button type="button"
                        on:click=move |_| on_decide.call((sid_deny.clone(), false, None, "once".into()))>
                        {move || t(locale.get(), if is_plan {
                            "approval.plan_reject"
                        } else if is_resource_conflict {
                            "approval.resource_conflict_cancel"
                        } else if is_image_resize {
                            "approval.image_resize_cancel"
                        } else {
                            "confirm.deny"
                        })}
                    </button>
                    {is_plan.then(|| view! {
                        <button type="button" on:click=move |_| show_feedback.update(|open| *open = !*open)>
                            {move || t(locale.get(), "approval.plan_other")}
                        </button>
                    })}
                </div>
                {is_plan.then(move || {
                    view! {
                        <Show when=move || show_feedback.get()>
                            <div class="plan-feedback">
                                <textarea
                                    id="plan-feedback-input"
                                    class="plan-feedback-input"
                                    rows="3"
                                    prop:value=move || feedback.get()
                                    placeholder=move || t(locale.get(), "approval.plan_feedback_placeholder")
                                    on:input=move |ev| feedback.set(event_target_value(&ev))
                                ></textarea>
                                <div class="plan-feedback-actions">
                                    <button
                                        type="button"
                                        class="primary"
                                        disabled=move || !feedback_ready()
                                        on:click=move |_| {
                                            let text = feedback.get().trim().to_string();
                                            if !text.is_empty() {
                                                on_decide.call((sid_feedback.get_untracked(), false, Some(text), "once".into()));
                                            }
                                        }
                                    >
                                        {move || t(locale.get(), "approval.plan_feedback_submit")}
                                    </button>
                                    <button
                                        type="button"
                                        on:click=move |_| {
                                            feedback.set(String::new());
                                            show_feedback.set(false);
                                        }
                                    >
                                        {move || t(locale.get(), "approval.plan_feedback_cancel")}
                                    </button>
                                </div>
                            </div>
                        </Show>
                    }
                })}
            </div>
        </div>
    }
}

#[cfg(test)]
mod layout_block_tests {
    use super::apply_layout_block;

    const BLOCK: &str = "Write outputs to figures/, results/tables/.";

    #[test]
    fn toggling_is_idempotent_and_preserves_user_text() {
        // Checking twice must not duplicate the block, and unchecking must
        // leave the user's own notes untouched.
        let on = apply_layout_block("", BLOCK, true);
        assert_eq!(on, BLOCK);
        assert_eq!(apply_layout_block(&on, BLOCK, true), BLOCK);

        let mixed = apply_layout_block("Counts are in GEO.", BLOCK, true);
        assert_eq!(mixed, format!("Counts are in GEO.\n\n{BLOCK}"));
        assert_eq!(apply_layout_block(&mixed, BLOCK, true), mixed);
        assert_eq!(
            apply_layout_block(&mixed, BLOCK, false),
            "Counts are in GEO."
        );
        assert_eq!(apply_layout_block("", BLOCK, false), "");
    }
}

#[cfg(test)]
mod streaming_markdown_tests {
    use super::streaming_markdown_commit_interval_ms;

    #[test]
    fn parsing_budget_uses_measured_cost_after_a_length_guarded_cold_start() {
        assert_eq!(streaming_markdown_commit_interval_ms(7_999, None), 50);
        assert_eq!(streaming_markdown_commit_interval_ms(8_000, None), 150);
        assert_eq!(streaming_markdown_commit_interval_ms(32_000, None), 300);
        assert_eq!(streaming_markdown_commit_interval_ms(128_000, None), 600);

        // Once measured, actual work rather than answer length controls the
        // cadence: cheap large Markdown remains fluid; expensive small
        // Markdown yields more main-thread time.
        assert_eq!(
            streaming_markdown_commit_interval_ms(512_000, Some(4.0)),
            50
        );
        assert_eq!(
            streaming_markdown_commit_interval_ms(1_000, Some(40.0)),
            240
        );
        assert_eq!(
            streaming_markdown_commit_interval_ms(1_000, Some(400.0)),
            1_200
        );
    }
}

/// Add or remove the standard-layout convention block in the new-project Agent
/// Context field (#405). The block is plain editable text, not hidden state:
/// whatever ends up in the textarea is what gets written to `.wisp/WISP.md`.
/// Strip-then-append keeps repeated toggles idempotent.
pub(crate) fn apply_layout_block(ctx: &str, block: &str, on: bool) -> String {
    let rest = ctx.replace(block, "");
    let rest = rest.trim();
    match (on, rest.is_empty()) {
        (false, _) => rest.to_string(),
        (true, true) => block.to_string(),
        (true, false) => format!("{rest}\n\n{block}"),
    }
}
