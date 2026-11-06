use super::*;

#[component]
pub(crate) fn DocxReviewPreview(dom_id: String, path: String) -> impl IntoView {
    let locale = use_locale();
    let snapshot = create_rw_signal(None::<DocxReviewSnapshot>);
    let busy = create_rw_signal(true);
    let stale = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);
    let reload = create_rw_signal(0u64);
    let alive = Rc::new(Cell::new(true));
    let generation = Rc::new(Cell::new(0u64));
    let cleanup = alive.clone();
    on_cleanup(move || cleanup.set(false));
    let read_only_reference = path.starts_with("artifact:")
        || path.starts_with("artifact-version:")
        || path.starts_with("remote:");
    let effect_path = path.clone();
    let effect_dom = dom_id.clone();
    let effect_alive = alive.clone();
    let effect_generation = generation.clone();
    create_effect(move |_| {
        reload.get();
        let loc = locale.get();
        let path = effect_path.clone();
        let dom_id = effect_dom.clone();
        let alive = effect_alive.clone();
        let generation = effect_generation.clone();
        let current = generation.get().wrapping_add(1);
        generation.set(current);
        busy.set(true);
        stale.set(false);
        error.set(None);
        snapshot.set(None);
        spawn_local(async move {
            let result = if read_only_reference {
                Ok(None)
            } else {
                invoke_checked(
                    "read_docx_review",
                    to_value(&serde_json::json!({"path":path})).unwrap(),
                )
                .await
                .map_err(js_error_text)
                .and_then(|v| {
                    serde_wasm_bindgen::from_value::<DocxReviewSnapshot>(v)
                        .map(Some)
                        .map_err(|e| e.to_string())
                })
            };
            if !alive.get() || generation.get() != current {
                return;
            }
            match result {
                Ok(value) => snapshot.set(value),
                Err(message) => error.set(Some(message)),
            }
            let payload = serde_json::json!({"path":path, "maxBytes":32*1024*1024,
                "loading":t(loc,"loading"), "error":t(loc,"preview.docx_error"), "renderChanges":true});
            let _ = mount_preview("docx", &dom_id, &payload.to_string()).await;
            if alive.get() && generation.get() == current {
                busy.set(false);
            }
        });
    });
    let act: Callback<(DocxReviewSelection, DocxReviewAction)> = Callback::new(
        move |(selection, action)| {
            if busy.get_untracked() || stale.get_untracked() {
                return;
            }
            let Some(current) = snapshot.get_untracked().filter(|s| s.writable) else {
                return;
            };
            busy.set(true);
            error.set(None);
            let path = path.clone();
            let alive = alive.clone();
            let generation = generation.clone();
            let request_generation = generation.get();
            spawn_local(async move {
                let result = invoke_checked("resolve_docx_review", to_value(&serde_json::json!({
                "path":path,"sourceSha256":current.source_sha256,"selection":selection,"action":action
            })).unwrap()).await.map_err(js_error_text);
                if !alive.get() || generation.get() != request_generation {
                    return;
                }
                match result {
                    Ok(_) => reload.update(|n| *n = n.wrapping_add(1)),
                    Err(message) => {
                        error.set(Some(message));
                        stale.set(true);
                        busy.set(false);
                    }
                }
            });
        },
    );
    let disabled = move || busy.get() || stale.get() || !snapshot.get().is_some_and(|s| s.writable);
    view! {
        <div class="docx-review" data-testid="docx-review">
            <section class="docx-review-panel" aria-label=move || t(locale.get(),"docx.review")>
                <div class="docx-review-toolbar">
                    <strong>{move || t(locale.get(),"docx.review")}</strong>
                    <button type="button" disabled=move || busy.get() on:click=move |_| reload.update(|n| *n=n.wrapping_add(1))>{move || t(locale.get(),"docx.refresh")}</button>
                    <button type="button" data-testid="docx-accept-deletions"
                        disabled=move || disabled() || !snapshot.get().is_some_and(|s| s.changes.iter().any(|c| c.kind == DocxChangeKind::Deletion))
                        on:click=move |_| act.call((DocxReviewSelection::Kind(DocxChangeKind::Deletion),DocxReviewAction::Accept))>{move || t(locale.get(),"docx.accept_deletions")}</button>
                    <button type="button" data-testid="docx-accept-rewrites"
                        disabled=move || disabled() || !snapshot.get().is_some_and(|s| s.changes.iter().any(|c| c.kind == DocxChangeKind::Rewrite))
                        on:click=move |_| act.call((DocxReviewSelection::Kind(DocxChangeKind::Rewrite),DocxReviewAction::Accept))>{move || t(locale.get(),"docx.accept_rewrites")}</button>
                </div>
                <p class="hint">{move || t(locale.get(),"docx.scope")}</p>
                {move || error.get().map(|message| view! {<p role="alert" class="docx-review-error">{message}<span>{t(locale.get(),"docx.refresh_hint")}</span></p>})}
                {move || (read_only_reference || snapshot.get().is_some_and(|s| !s.writable)).then(|| view! {<p class="hint">{t(locale.get(),"docx.readonly")}</p>})}
                {move || snapshot.get().map(|s| {
                    let loc = locale.get();
                    view! {
                        {(s.unsupported_count > 0).then(|| view! {<p class="hint">{format!("{} ({})",t(loc,"docx.unsupported"),s.unsupported_count)}</p>})}
                        {s.changes.is_empty().then(|| view! {<p data-testid="docx-no-changes" class="hint">{t(loc,"docx.empty")}</p>})}
                        <div class="docx-review-changes">
                            {s.changes.into_iter().map(|change| {
                                let accept_id = change.id.clone();
                                let reject_id = change.id.clone();
                                let label = match change.kind { DocxChangeKind::Deletion=>"docx.deletion", DocxChangeKind::Insertion=>"docx.insertion", DocxChangeKind::Rewrite=>"docx.rewrite" };
                                view! {<div class="docx-review-change" data-testid="docx-change" data-change-id=change.id>
                                    <div class="docx-review-change-head"><strong>{t(loc,label)}</strong><span>{change.author}</span>
                                        <button type="button" disabled=disabled on:click=move |_| act.call((DocxReviewSelection::Change(accept_id.clone()),DocxReviewAction::Accept))>{t(loc,"docx.accept")}</button>
                                        <button type="button" disabled=disabled on:click=move |_| act.call((DocxReviewSelection::Change(reject_id.clone()),DocxReviewAction::Reject))>{t(loc,"docx.reject")}</button>
                                    </div>
                                    {(!change.old.is_empty()).then(|| view! {<del>{change.old}</del>})}
                                    {(!change.new.is_empty()).then(|| view! {<ins>{change.new}</ins>})}
                                </div>}
                            }).collect_view()}
                        </div>
                    }
                })}
            </section>
            <div class="rp-heavy" id=dom_id>{move || t(locale.get(),"loading")}</div>
        </div>
    }
}
