use crate::app_support::compose_icon;
use crate::bindings::invoke_checked;
use crate::dto::{
    PublicationEvidenceBinding, PublicationFreezeOutcome, PublicationItemInfo,
    PublicationReadinessFinding, PublicationReadinessInfo, PublicationWaiverInfo,
    PublicationWorkspaceInfo,
};
use crate::i18n::{t, tf, Locale};
use crate::publication_sources::PublicationSourcePicker;
use crate::text::event_target_value;
use crate::window_capture_escape;
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::collections::{HashMap, HashSet};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PublicationEvidenceSource {
    pub(super) kind: &'static str,
    pub(super) id: String,
    pub(super) label: String,
}

fn error_text(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

fn install_workspace(
    value: JsValue,
    workspace: RwSignal<Option<PublicationWorkspaceInfo>>,
    publication_id: RwSignal<String>,
    revision_id: RwSignal<String>,
    selected_item_id: RwSignal<Option<String>>,
) -> Result<(), String> {
    let next = serde_wasm_bindgen::from_value::<PublicationWorkspaceInfo>(value)
        .map_err(|error| error.to_string())?;
    publication_id.set(
        next.publication
            .as_ref()
            .map(|publication| publication.id.clone())
            .unwrap_or_default(),
    );
    revision_id.set(
        next.revision
            .as_ref()
            .map(|revision| revision.id.clone())
            .unwrap_or_default(),
    );
    let selected = selected_item_id.get_untracked();
    if selected
        .as_ref()
        .is_some_and(|id| !next.items.iter().any(|item| &item.id == id))
    {
        selected_item_id.set(None);
    }
    workspace.set(Some(next));
    Ok(())
}

fn refresh_workspace(
    workspace: RwSignal<Option<PublicationWorkspaceInfo>>,
    publication_id: RwSignal<String>,
    revision_id: RwSignal<String>,
    selected_item_id: RwSignal<Option<String>>,
    loading: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) {
    loading.set(true);
    error.set(None);
    spawn_local(async move {
        let publication = publication_id.get_untracked();
        let revision = revision_id.get_untracked();
        let args = serde_json::json!({
            "publicationId": (!publication.is_empty()).then_some(publication.clone()),
            "revisionId": (!revision.is_empty()).then_some(revision.clone()),
        });
        match invoke_checked(
            "get_publication_workspace",
            to_value(&args).unwrap_or(JsValue::UNDEFINED),
        )
        .await
        {
            Ok(value) => {
                if publication_id.try_get_untracked().as_deref() != Some(publication.as_str())
                    || revision_id.try_get_untracked().as_deref() != Some(revision.as_str())
                {
                    return;
                }
                if let Err(message) = install_workspace(
                    value,
                    workspace,
                    publication_id,
                    revision_id,
                    selected_item_id,
                ) {
                    error.set(Some(message));
                }
            }
            Err(value) => error.set(Some(error_text(value))),
        }
        loading.set(false);
    });
}

fn invoke_workspace(
    command: &'static str,
    args: serde_json::Value,
    workspace: RwSignal<Option<PublicationWorkspaceInfo>>,
    publication_id: RwSignal<String>,
    revision_id: RwSignal<String>,
    selected_item_id: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    after: impl FnOnce(bool) + 'static,
) {
    busy.set(true);
    error.set(None);
    spawn_local(async move {
        let succeeded =
            match invoke_checked(command, to_value(&args).unwrap_or(JsValue::UNDEFINED)).await {
                Ok(value) => match install_workspace(
                    value,
                    workspace,
                    publication_id,
                    revision_id,
                    selected_item_id,
                ) {
                    Ok(()) => true,
                    Err(message) => {
                        error.set(Some(message));
                        false
                    }
                },
                Err(value) => {
                    error.set(Some(error_text(value)));
                    false
                }
            };
        busy.set(false);
        after(succeeded);
    });
}

pub(super) fn manuscript_rows(items: &[PublicationItemInfo]) -> Vec<(PublicationItemInfo, usize)> {
    fn visit(
        parent: Option<&str>,
        children: &HashMap<Option<String>, Vec<PublicationItemInfo>>,
        seen: &mut HashSet<String>,
        depth: usize,
        rows: &mut Vec<(PublicationItemInfo, usize)>,
    ) {
        let key = parent.map(str::to_string);
        let Some(items) = children.get(&key) else {
            return;
        };
        for item in items {
            if !seen.insert(item.id.clone()) {
                continue;
            }
            rows.push((item.clone(), depth));
            visit(Some(&item.id), children, seen, depth + 1, rows);
        }
    }

    let mut children = HashMap::<Option<String>, Vec<PublicationItemInfo>>::new();
    for item in items {
        children
            .entry(item.parent_item_id.clone())
            .or_default()
            .push(item.clone());
    }
    for siblings in children.values_mut() {
        siblings.sort_by(|left, right| {
            left.ordinal
                .cmp(&right.ordinal)
                .then_with(|| left.title.cmp(&right.title))
        });
    }
    let mut rows = Vec::with_capacity(items.len());
    let mut seen = HashSet::new();
    visit(None, &children, &mut seen, 0, &mut rows);
    for item in items {
        if seen.insert(item.id.clone()) {
            rows.push((item.clone(), 0));
            visit(Some(&item.id), &children, &mut seen, 1, &mut rows);
        }
    }
    rows
}

fn finding_rows(
    locale: Locale,
    label_key: &'static str,
    class_name: &'static str,
    findings: Vec<PublicationReadinessFinding>,
    draft: bool,
    on_waive: Callback<String>,
) -> View {
    if findings.is_empty() {
        return ().into_view();
    }
    view! {
        <section class=format!("publication-findings {class_name}")>
            <h4>{t(locale, label_key)}</h4>
            {findings.into_iter().map(|finding| {
                let code = finding.code.clone();
                let source = finding.source_id.clone();
                let can_waive = draft && finding.waivable && !finding.waived;
                view! {
                    <article class="publication-finding" data-finding-code=code.clone()>
                        <div>
                            <strong>{finding.message}</strong>
                            <code>{code.clone()}</code>
                            {source.map(|source| view! { <span>{source}</span> })}
                            {finding.waived.then(|| view! {
                                <span class="publication-waived">{t(locale, "publication.waived")}</span>
                            })}
                        </div>
                        {can_waive.then(|| view! {
                            <button type="button" class="secondary"
                                on:click=move |_| on_waive.call(code.clone())>
                                {t(locale, "publication.waive")}
                            </button>
                        })}
                    </article>
                }
            }).collect_view()}
        </section>
    }
    .into_view()
}

fn evidence_for_item(
    bindings: &[PublicationEvidenceBinding],
    item_id: Option<&str>,
) -> Vec<PublicationEvidenceBinding> {
    match item_id {
        Some(item_id) => bindings
            .iter()
            .filter(|binding| binding.item_id.as_deref() == Some(item_id))
            .cloned()
            .collect(),
        None => bindings.to_vec(),
    }
}

fn precise_evidence_source(
    kind: &str,
    frame_id: &str,
    message_seq: &str,
    byte_start: &str,
    byte_end: &str,
    tool_call_id: &str,
    exact_id: &str,
) -> Result<PublicationEvidenceSource, String> {
    let exact_id = exact_id.trim();
    match kind {
        "execution_log" | "code_cell" | "external_resource" => {
            if exact_id.is_empty() {
                return Err("An exact source ID is required.".into());
            }
            let (kind, label) = match kind {
                "execution_log" => ("execution_log", "Execution log"),
                "code_cell" => ("code_cell", "Code cell"),
                _ => ("external_resource", "External resource"),
            };
            Ok(PublicationEvidenceSource {
                kind,
                id: exact_id.into(),
                label: format!("{label}: {exact_id}"),
            })
        }
        "message_span" => {
            let seq = message_seq
                .trim()
                .parse::<i64>()
                .map_err(|_| "Message sequence must be a positive integer.")?;
            let start = byte_start
                .trim()
                .parse::<usize>()
                .map_err(|_| "Span start must be a byte offset.")?;
            let end = byte_end
                .trim()
                .parse::<usize>()
                .map_err(|_| "Span end must be a byte offset.")?;
            if frame_id.trim().is_empty() || seq < 1 || start >= end {
                return Err(
                    "MessageSpan requires a frame, message sequence, and valid range.".into(),
                );
            }
            Ok(PublicationEvidenceSource {
                kind: "message_span",
                id: serde_json::to_string(&serde_json::json!({
                    "byte_end": end,
                    "byte_start": start,
                    "frame_id": frame_id.trim(),
                    "message_seq": seq,
                }))
                .map_err(|error| error.to_string())?,
                label: format!("Message {seq} bytes {start}–{end}"),
            })
        }
        "tool_call" => {
            let seq = message_seq
                .trim()
                .parse::<i64>()
                .map_err(|_| "Message sequence must be a positive integer.")?;
            if frame_id.trim().is_empty() || seq < 1 || tool_call_id.trim().is_empty() {
                return Err("ToolCall requires a frame, message sequence, and call ID.".into());
            }
            Ok(PublicationEvidenceSource {
                kind: "tool_call",
                id: serde_json::to_string(&serde_json::json!({
                    "frame_id": frame_id.trim(),
                    "message_seq": seq,
                    "tool_call_id": tool_call_id.trim(),
                }))
                .map_err(|error| error.to_string())?,
                label: format!("Tool result: {}", tool_call_id.trim()),
            })
        }
        _ => Err("Unsupported precise evidence kind.".into()),
    }
}

#[component]
pub(super) fn PublicationWorkspacePage(
    locale: ReadSignal<Locale>,
    binding_source: RwSignal<Option<PublicationEvidenceSource>>,
    on_close: Callback<()>,
) -> impl IntoView {
    let workspace = create_rw_signal::<Option<PublicationWorkspaceInfo>>(None);
    let publication_id = create_rw_signal(String::new());
    let revision_id = create_rw_signal(String::new());
    let selected_item_id = create_rw_signal::<Option<String>>(None);
    let loading = create_rw_signal(false);
    let busy = create_rw_signal(false);
    let error = create_rw_signal::<Option<String>>(None);
    let transient_readiness = create_rw_signal::<Option<PublicationReadinessInfo>>(None);
    let page = create_rw_signal("evidence");

    let new_title = create_rw_signal(String::new());
    let new_description = create_rw_signal(String::new());
    let new_revision_label =
        create_rw_signal(t(locale.get_untracked(), "publication.initial_revision").to_string());

    let item_editor_open = create_rw_signal(false);
    let item_kind = create_rw_signal("figure".to_string());
    let item_title = create_rw_signal(String::new());
    let item_parent = create_rw_signal(String::new());

    let anchor_open = create_rw_signal(false);
    let anchor_kind = create_rw_signal("message_span".to_string());
    let anchor_frame = create_rw_signal(String::new());
    let anchor_message_seq = create_rw_signal(String::new());
    let anchor_byte_start = create_rw_signal(String::new());
    let anchor_byte_end = create_rw_signal(String::new());
    let anchor_tool_call = create_rw_signal(String::new());
    let anchor_exact_id = create_rw_signal(String::new());

    let freeze_visibility = create_rw_signal("private".to_string());
    let freeze_phi_reviewed = create_rw_signal(false);
    let freeze_redistribution_reviewed = create_rw_signal(false);
    let freeze_restricted_bytes = create_rw_signal(false);

    let waiver_code = create_rw_signal::<Option<String>>(None);
    let waiver_author = create_rw_signal("Local user".to_string());
    let waiver_reason = create_rw_signal(String::new());

    let binding_seen = create_rw_signal(String::new());
    let binding_revision = create_rw_signal(String::new());
    let binding_item = create_rw_signal(String::new());
    let binding_purpose = create_rw_signal(String::new());
    let binding_claim = create_rw_signal(String::new());
    let binding_selection = create_rw_signal("selected".to_string());
    let binding_visibility = create_rw_signal("private".to_string());

    window_capture_escape(move || {
        if anchor_open.get_untracked() {
            anchor_open.set(false);
        } else if waiver_code.get_untracked().is_some() {
            waiver_code.set(None);
            waiver_reason.set(String::new());
        } else if binding_source.get_untracked().is_some() {
            binding_source.set(None);
            binding_seen.set(String::new());
        } else if page.get_untracked() == "add" {
            page.set("evidence");
        } else if item_editor_open.get_untracked() {
            item_editor_open.set(false);
        } else {
            return false;
        }
        true
    });

    refresh_workspace(
        workspace,
        publication_id,
        revision_id,
        selected_item_id,
        loading,
        error,
    );

    create_effect(move |_| {
        let Some(source) = binding_source.get() else {
            return;
        };
        page.set("add");
        if binding_seen.get_untracked() != source.id {
            binding_seen.set(source.id);
            binding_revision.set(String::new());
            binding_item.set(selected_item_id.get_untracked().unwrap_or_default());
            binding_purpose.set(String::new());
            binding_claim.set(String::new());
            binding_selection.set("selected".into());
            binding_visibility.set("private".into());
        }
        if !binding_revision.get_untracked().is_empty() {
            return;
        }
        let selected = workspace.with(|workspace| {
            workspace.as_ref().and_then(|workspace| {
                workspace
                    .revision
                    .as_ref()
                    .filter(|revision| revision.state == "draft")
                    .or_else(|| {
                        workspace
                            .revisions
                            .iter()
                            .find(|revision| revision.state == "draft")
                    })
                    .map(|revision| revision.id.clone())
            })
        });
        if let Some(selected) = selected {
            binding_revision.set(selected);
        }
    });

    let refresh = Callback::new(move |_| {
        transient_readiness.set(None);
        refresh_workspace(
            workspace,
            publication_id,
            revision_id,
            selected_item_id,
            loading,
            error,
        );
    });

    // Any change to the evidence set or policy invalidates the previous check.
    create_effect(move |_| {
        workspace.track();
        freeze_visibility.track();
        freeze_phi_reviewed.track();
        freeze_redistribution_reviewed.track();
        freeze_restricted_bytes.track();
        transient_readiness.set(None);
    });
    let run_check = Callback::new(move |commit: bool| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        error.set(None);
        let selected_revision = revision_id.get_untracked();
        let args = serde_json::json!({
            "revisionId": selected_revision,
            "policy": {
                "target_visibility": freeze_visibility.get_untracked(),
                "phi_pii_reviewed": freeze_phi_reviewed.get_untracked(),
                "redistribution_reviewed": freeze_redistribution_reviewed.get_untracked(),
                "snapshot_restricted_bytes": freeze_restricted_bytes.get_untracked(),
            },
        });
        spawn_local(async move {
            let command = if commit {
                "freeze_publication_revision"
            } else {
                "check_publication_revision"
            };
            let result =
                invoke_checked(command, to_value(&args).unwrap_or(JsValue::UNDEFINED)).await;
            if revision_id.try_get_untracked().as_deref() != Some(selected_revision.as_str()) {
                return;
            }
            match result {
                Ok(value) => {
                    match serde_wasm_bindgen::from_value::<PublicationFreezeOutcome>(value) {
                        Ok(outcome) => {
                            transient_readiness.set(Some(outcome.readiness));
                            if outcome.frozen {
                                refresh_workspace(
                                    workspace,
                                    publication_id,
                                    revision_id,
                                    selected_item_id,
                                    loading,
                                    error,
                                );
                            }
                        }
                        Err(e) => error.set(Some(e.to_string())),
                    }
                }
                Err(e) => error.set(Some(error_text(e))),
            }
            busy.set(false);
        });
    });

    view! {
        <div class="publication-page">
            <section class="publication-workspace-page" role="region"
                aria-labelledby="publication-workspace-title"
                data-testid="publication-workspace"
                on:click=|event| event.stop_propagation()>
                <header class="publication-workspace-head">
                    <div>
                        <h2 id="publication-workspace-title">
                            {move || t(locale.get(), "publication.title")}
                        </h2>
                        <p>{move || t(locale.get(), "publication.subtitle")}</p>
                    </div>
                    <div class="publication-head-actions">
                        <button type="button" class="icon-btn"
                            title=move || t(locale.get(), "publication.refresh")
                            aria-label=move || t(locale.get(), "publication.refresh")
                            on:click=move |event| refresh.call(event)>
                            {compose_icon("sync")}
                        </button>
                        <button type="button" class="ps-close"
                            title=move || t(locale.get(), "publication.close")
                            aria-label=move || t(locale.get(), "publication.close")
                            on:click=move |_| on_close.call(())>
                            {compose_icon("arrow-left")}
                        </button>
                    </div>
                </header>

                {move || error.get().map(|message| view! {
                    <div class="publication-error" role="alert">{message}</div>
                })}

                {move || if loading.get() && workspace.get().is_none() {
                    view! {
                        <div class="publication-empty">{t(locale.get(), "publication.loading")}</div>
                    }.into_view()
                } else {
                    let current = workspace.get();
                    let has_publications = current.as_ref()
                        .is_some_and(|workspace| !workspace.publications.is_empty());
                    if !has_publications {
                        view! {
                            <div class="publication-create">
                                <div class="publication-create-intro">
                                <h3>{t(locale.get(), "publication.create")}</h3>
                                <p class="publication-help">{t(locale.get(), "publication.create_hint")}</p>
                                <ol class="publication-onboarding">
                                    <li>{t(locale.get(), "publication.choose_source")}</li>
                                    <li>{t(locale.get(), "publication.explain_use")}</li>
                                    <li>{t(locale.get(), "publication.tab_readiness")}</li>
                                </ol>
                                </div>
                                <div class="publication-create-fields">
                                <label>
                                    <span>{t(locale.get(), "publication.paper_title")}</span>
                                    <input type="text" data-testid="publication-new-title"
                                        prop:value=move || new_title.get()
                                        on:input=move |event| new_title.set(event_target_value(&event)) />
                                </label>
                                <label>
                                    <span>{t(locale.get(), "publication.description")}</span>
                                    <textarea prop:value=move || new_description.get()
                                        on:input=move |event| new_description.set(event_target_value(&event))></textarea>
                                </label>
                                <label>
                                    <span>{t(locale.get(), "publication.revision_label")}</span>
                                    <input type="text" prop:value=move || new_revision_label.get()
                                        on:input=move |event| new_revision_label.set(event_target_value(&event)) />
                                </label>
                                <button type="button" class="primary"
                                    disabled=move || busy.get() || new_title.get().trim().is_empty()
                                        || new_revision_label.get().trim().is_empty()
                                    on:click=move |_| {
                                        let args = serde_json::json!({ "input": {
                                            "title": new_title.get_untracked(),
                                            "description": new_description.get_untracked(),
                                            "revisionLabel": new_revision_label.get_untracked(),
                                        }});
                                        invoke_workspace(
                                            "create_publication_workspace", args, workspace,
                                            publication_id, revision_id, selected_item_id, busy, error,
                                            move |_| {},
                                        );
                                    }>
                                    {t(locale.get(), "publication.create_action")}
                                </button>
                                </div>
                            </div>
                        }.into_view()
                    } else {
                        let current = current.expect("checked workspace");
                        let revision = current.revision.clone();
                        let draft = revision.as_ref().is_some_and(|revision| revision.state == "draft");
                        let capsule_ready = revision.as_ref().is_some_and(|revision| {
                            matches!(revision.state.as_str(), "frozen" | "published")
                        });
                        let selected_item = selected_item_id.get();
                        let evidence = evidence_for_item(&current.bindings, selected_item.as_deref());
                        let readiness = transient_readiness.get().or_else(|| current.readiness.clone());
                        let next_revision_number = current.revisions.iter()
                            .map(|revision| revision.revision_number)
                            .max()
                            .unwrap_or(0) + 1;
                        let clone_label = tf(
                            locale.get(),
                            "publication.clone_label",
                            &[("number", &next_revision_number.to_string())],
                        );
                        let selected_revision_id = revision.as_ref()
                            .map(|revision| revision.id.clone())
                            .unwrap_or_default();
                        let current_for_select = current.clone();
                        let current_for_tree = current.clone();
                        let current_for_evidence = current.clone();
                        let current_for_readiness = current.clone();
                        let capsule_builds = current.capsule_builds.clone();
                        let reproduction_runs = current.reproduction_runs.clone();
                        let reproduction_results = current.reproduction_results.clone();
                        let effective_capability = current.effective_capability_level.clone();
                        let items_for_editor = current.items.clone();
                        view! {
                            <div class="publication-toolbar">
                                <label>
                                    <span>{t(locale.get(), "publication.paper")}</span>
                                    <select prop:value=move || publication_id.get()
                                        on:change=move |event| {
                                            publication_id.set(event_target_value(&event));
                                            revision_id.set(String::new());
                                            transient_readiness.set(None);
                                            refresh_workspace(
                                                workspace, publication_id, revision_id,
                                                selected_item_id, loading, error,
                                            );
                                        }>
                                        {current_for_select.publications.into_iter().map(|publication| {
                                            let id = publication.id.clone();
                                            view! {
                                                <option value=publication.id
                                                    prop:selected=move || publication_id.get() == id>
                                                    {publication.title}
                                                </option>
                                            }
                                        }).collect_view()}
                                    </select>
                                </label>
                                <label>
                                    <span>{t(locale.get(), "publication.revision")}</span>
                                    <select prop:value=move || revision_id.get()
                                        on:change=move |event| {
                                            revision_id.set(event_target_value(&event));
                                            transient_readiness.set(None);
                                            refresh_workspace(
                                                workspace, publication_id, revision_id,
                                                selected_item_id, loading, error,
                                            );
                                        }>
                                        {current.revisions.clone().into_iter().map(|revision| {
                                            let label = format!(
                                                "{} · {}",
                                                revision.label,
                                                t(locale.get(), &format!("publication.state.{}", revision.state)),
                                            );
                                            let id = revision.id.clone();
                                            view! {
                                                <option value=revision.id
                                                    prop:selected=move || revision_id.get() == id>
                                                    {label}
                                                </option>
                                            }
                                        }).collect_view()}
                                    </select>
                                </label>
                                {revision.clone().map(|revision| view! {
                                    <span class=format!("publication-state {}", revision.state)>
                                        {t(locale.get(), &format!("publication.state.{}", revision.state))}
                                    </span>
                                    <span class="publication-capability" hidden=draft>
                                        {t(locale.get(), &format!(
                                            "publication.capability.{}",
                                            effective_capability.clone()
                                                .unwrap_or(revision.capability_level),
                                        ))}
                                    </span>
                                })}
                                <div class="publication-toolbar-actions">
                                    <button type="button" class="secondary"
                                        disabled=move || busy.get() || revision_id.get().is_empty()
                                        on:click={
                                            let selected_revision_id = selected_revision_id.clone();
                                            let clone_label = clone_label.clone();
                                            move |_| {
                                                invoke_workspace(
                                                    "clone_publication_revision",
                                                    serde_json::json!({
                                                        "revisionId": selected_revision_id,
                                                        "label": clone_label,
                                                    }),
                                                    workspace, publication_id, revision_id,
                                                    selected_item_id, busy, error,
                                                    move |ok| if ok { transient_readiness.set(None) },
                                                );
                                            }
                                        }>
                                        {t(locale.get(), "publication.clone")}
                                    </button>
                                    {draft.then(|| view! {
                                        <button type="button" class="primary" hidden=move || page.get() == "add"
                                            data-testid="add-publication-evidence"
                                            on:click=move |_| { binding_source.set(None); page.set("add"); }>
                                            {compose_icon("plus")}{t(locale.get(), "publication.add_evidence")}
                                        </button>
                                    })}
                                    {capsule_ready.then(|| {
                                        let selected_revision_id = selected_revision_id.clone();
                                        view! {
                                            <button type="button" class="primary"
                                                data-testid="build-publication-capsule"
                                                disabled=move || busy.get()
                                                on:click=move |_| {
                                                    busy.set(true);
                                                    error.set(None);
                                                    let selected_revision_id = selected_revision_id.clone();
                                                    spawn_local(async move {
                                                        match invoke_checked(
                                                            "build_publication_capsule",
                                                            to_value(&serde_json::json!({
                                                                "revisionId": selected_revision_id,
                                                            })).unwrap_or(JsValue::UNDEFINED),
                                                        ).await {
                                                            Ok(_) => refresh_workspace(
                                                                workspace, publication_id, revision_id,
                                                                selected_item_id, loading, error,
                                                            ),
                                                            Err(value) => error.set(Some(error_text(value))),
                                                        }
                                                        busy.set(false);
                                                    });
                                                }>
                                                {t(locale.get(), "publication.build_capsule")}
                                            </button>
                                        }
                                    })}
                                </div>
                            </div>


                            <nav class="publication-page-tabs" aria-label=t(locale.get(), "publication.title")>
                                {[("evidence", "publication.tab_evidence"), ("readiness", "publication.tab_readiness"), ("versions", "publication.tab_versions")].into_iter().map(|(value,key)| {
                                    // Adding a source is a step of the organize tab, not a fourth tab.
                                    let current = move || page.get() == value || (value == "evidence" && page.get() == "add");
                                    view! {
                                        <button type="button" class:active=current aria-pressed=move || current().to_string() on:click=move |_| { binding_source.set(None); page.set(value); }>{t(locale.get(), key)}</button>
                                    }
                                }).collect_view()}
                            </nav>
                            <Show when=move || page.get() == "add" && binding_source.get().is_none()>
                                <PublicationSourcePicker locale=locale on_select=Callback::new(move |source| binding_source.set(Some(source))) on_advanced=Callback::new(move |_| anchor_open.set(true)) on_cancel=Callback::new(move |_| page.set("evidence")) />
                            </Show>
                            <section class="publication-versions" hidden=move || page.get() != "versions">
                                <h3>{t(locale.get(), "publication.versions_hint")}</h3>
                                {current.revisions.clone().into_iter().map(|old| {
                                    let id=old.id.clone();
                                    view! {
                                        <article class="publication-version-row">
                                            <div><h4>{format!("{} · v{}",old.label,old.revision_number)}</h4>
                                                <span class=format!("publication-state {}",old.state)>{t(locale.get(), &format!("publication.state.{}",old.state))}</span>
                                                {old.manifest_sha256.map(|hash| view! { <details><summary>{t(locale.get(), "publication.frozen_manifest")}</summary><code>{hash}</code></details> })}
                                            </div>
                                            <button type="button" class="secondary" disabled=move || busy.get() on:click=move |_| { revision_id.set(id.clone()); transient_readiness.set(None); page.set("evidence"); refresh_workspace(workspace,publication_id,revision_id,selected_item_id,loading,error); }>{t(locale.get(), "publication.view_version")}</button>
                                        </article>
                                    }
                                }).collect_view()}
                            </section>
                            <div class="publication-workspace-grid" class:checking=move || page.get() == "readiness" hidden=move || !matches!(page.get(), "evidence" | "readiness")>
                                <aside class="publication-manuscript" hidden=move || page.get() != "evidence">
                                    <div class="publication-pane-head">
                                        <h3>{t(locale.get(), "publication.manuscript")}</h3>
                                        {draft.then(|| view! {
                                            <button type="button" class="secondary"
                                                on:click=move |_| item_editor_open.update(|open| *open = !*open)>
                                                {t(locale.get(), "publication.add_item")}
                                            </button>
                                        })}
                                    </div>
                                    {move || item_editor_open.get().then(|| view! {
                                        <div class="publication-item-editor">
                                            <select aria-label=t(locale.get(), "publication.item_kind")
                                                prop:value=move || item_kind.get()
                                                on:change=move |event| item_kind.set(event_target_value(&event))>
                                                {["section", "claim", "figure", "table", "methods", "supplement"]
                                                    .into_iter().map(|kind| view! {
                                                        <option value=kind prop:selected=move || item_kind.get() == kind>
                                                            {t(locale.get(), &format!("publication.item.{kind}"))}
                                                        </option>
                                                    }).collect_view()}
                                            </select>
                                            <input type="text"
                                                placeholder=t(locale.get(), "publication.item_title")
                                                prop:value=move || item_title.get()
                                                on:input=move |event| item_title.set(event_target_value(&event)) />
                                            <select aria-label=t(locale.get(), "publication.parent_item")
                                                prop:value=move || item_parent.get()
                                                on:change=move |event| item_parent.set(event_target_value(&event))>
                                                <option value="" prop:selected=move || item_parent.get().is_empty()>
                                                    {t(locale.get(), "publication.no_parent")}
                                                </option>
                                                {items_for_editor.clone().into_iter().map(|item| {
                                                    let id = item.id.clone();
                                                    view! {
                                                        <option value=item.id prop:selected=move || item_parent.get() == id>
                                                            {item.title}
                                                        </option>
                                                    }
                                                }).collect_view()}
                                            </select>
                                            <button type="button" class="primary"
                                                disabled=move || busy.get() || item_title.get().trim().is_empty()
                                                on:click=move |_| {
                                                    let current_revision = revision_id.get_untracked();
                                                    let ordinal = workspace.with_untracked(|workspace| {
                                                        workspace.as_ref().map(|workspace| workspace.items.len() as i64).unwrap_or(0)
                                                    });
                                                    invoke_workspace(
                                                        "save_publication_item",
                                                        serde_json::json!({ "input": {
                                                            "id": null,
                                                            "revisionId": current_revision,
                                                            "parentItemId": match item_parent.get_untracked() {
                                                                value if value.is_empty() => None,
                                                                value => Some(value),
                                                            },
                                                            "kind": item_kind.get_untracked(),
                                                            "title": item_title.get_untracked(),
                                                            "content": "",
                                                            "ordinal": ordinal,
                                                        }}),
                                                        workspace, publication_id, revision_id,
                                                        selected_item_id, busy, error,
                                                        move |ok| if ok {
                                                            item_title.set(String::new());
                                                            item_editor_open.set(false);
                                                        },
                                                    );
                                                }>
                                                {t(locale.get(), "publication.save_item")}
                                            </button>
                                        </div>
                                    })}
                                    <button type="button" class="publication-tree-all"
                                        class:active=move || selected_item_id.get().is_none()
                                        on:click=move |_| selected_item_id.set(None)>
                                        <span>{t(locale.get(), "publication.all_evidence")}</span>
                                        <span>{current_for_tree.bindings.len()}</span>
                                    </button>
                                    <div class="publication-tree" data-testid="publication-manuscript-tree">
                                        {manuscript_rows(&current_for_tree.items).into_iter()
                                            .map(|(item, depth)| {
                                                let item_id = item.id.clone();
                                                let evidence_count = current_for_tree.bindings.iter()
                                                    .filter(|binding| binding.item_id.as_deref() == Some(&item.id))
                                                    .count();
                                                view! {
                                                    <button type="button" class="publication-tree-item"
                                                        class:active=move || selected_item_id.get().as_deref() == Some(item_id.as_str())
                                                        style=format!("--publication-depth:{depth}")
                                                        on:click={
                                                            let item_id = item.id.clone();
                                                            move |_| selected_item_id.set(Some(item_id.clone()))
                                                        }>
                                                        <span class="publication-item-kind">
                                                            {compose_icon(match item.kind.as_str() { "figure" => "image", "table" => "table", "claim" => "check", _ => "book" })}
                                                            {t(locale.get(), &format!("publication.item.{}", item.kind))}
                                                        </span>
                                                        <strong>{item.title}</strong>
                                                        <span>{evidence_count}</span>
                                                    </button>
                                                }
                                            }).collect_view()}
                                    </div>
                                    {(!current_for_tree.item_links.is_empty()).then(|| view! {
                                        <div class="publication-item-links">
                                            {current_for_tree.item_links.into_iter().map(|link| view! {
                                                <span>{format!(
                                                    "{} {} {}",
                                                    current_for_tree.items.iter().find(|item| item.id == link.source_item_id).map(|item| item.title.as_str()).unwrap_or(""),
                                                    link.relation,
                                                    current_for_tree.items.iter().find(|item| item.id == link.target_item_id).map(|item| item.title.as_str()).unwrap_or(""),
                                                )}</span>
                                            }).collect_view()}
                                        </div>
                                    })}
                                </aside>

                                <section class="publication-evidence" hidden=move || page.get() != "evidence">
                                    <div class="publication-pane-head">
                                        <h3>{t(locale.get(), "publication.evidence")}</h3>
                                        <span>{evidence.len()}</span>
                                    </div>
                                    {if evidence.is_empty() {
                                        view! {
                                            <div class="publication-empty">
                                                <h3>{t(locale.get(), "publication.start_figure")}</h3>
                                                <p>{t(locale.get(), "publication.start_hint")}</p>
                                                {draft.then(|| view! { <button type="button" class="primary" on:click=move |_| page.set("add")>{compose_icon("plus")}{t(locale.get(), "publication.add_evidence")}</button> })}
                                            </div>
                                        }.into_view()
                                    } else {
                                        evidence.into_iter().map(|binding| {
                                            let lineage = current_for_evidence.lineage.iter()
                                                .find(|lineage| lineage.binding_id == binding.id)
                                                .cloned();
                                            let drift = current_for_evidence.drift.iter()
                                                .find(|drift| drift.binding_id == binding.id)
                                                .cloned();
                                            let reviews = current_for_evidence.reviews.iter()
                                                .filter(|review| review.binding_id == binding.id)
                                                .cloned()
                                                .collect::<Vec<_>>();
                                            let supersession = current_for_evidence.supersessions.iter()
                                                .find(|supersession| {
                                                    supersession.old_binding_id == binding.id
                                                        || supersession.new_binding_id == binding.id
                                                })
                                                .cloned();
                                            let current_selection = binding.selection_state.clone();
                                            // A <select> ignores `prop:value` set before its options exist,
                                            // so each option carries its own selected state.
                                            let selection_for_options = binding.selection_state.clone();
                                            let visibility_for_options = binding.visibility.clone();
                                            let current_visibility = binding.visibility.clone();
                                            let binding_id_for_selection = binding.id.clone();
                                            let binding_id_for_visibility = binding.id.clone();
                                            let reproduction_source_run = lineage.as_ref()
                                                .and_then(|lineage| lineage.producing_run_id.clone())
                                                .or_else(|| {
                                                    (binding.source_kind == "run")
                                                        .then(|| binding.source_id.clone())
                                                });
                                            view! {
                                                <article class="publication-evidence-card"
                                                    data-testid="publication-evidence-card"
                                                    data-binding-id=binding.id.clone()>
                                                    <header>
                                                        <div>
                                                            <span class="publication-source-kind">
                                                                {t(locale.get(), &format!(
                                                                    "publication.source.{}", binding.source_kind,
                                                                ))}
                                                            </span>
                                                            <strong>{lineage.as_ref()
                                                                .map(|lineage| lineage.source_label.clone())
                                                                .unwrap_or_else(|| binding.source_id.clone())}</strong>
                                                        </div>
                                                        <span class=format!("publication-visibility {}", binding.visibility)>
                                                            {t(locale.get(), &format!(
                                                                "publication.visibility.{}", binding.visibility,
                                                            ))}
                                                        </span>
                                                        <span class="publication-selection">{t(locale.get(), &format!("publication.selection.{}", binding.selection_state))}</span>
                                                    </header>
                                                    <div class="publication-purpose"><span>{t(locale.get(), "publication.purpose")}</span><p>{binding.purpose.clone()}</p></div>
                                                    {binding.supported_claim_item_id.as_ref().and_then(|id| current_for_evidence.items.iter().find(|item| &item.id == id)).map(|claim| view! { <p class="publication-supported-claim">{t(locale.get(), "publication.supported_claim")}{": "}{claim.title.clone()}</p> })}
                                                    <details class="publication-technical"><summary>{t(locale.get(), "publication.technical")}</summary>
                                                    <dl>
                                                        <div>
                                                            <dt>{t(locale.get(), "publication.exact_source")}</dt>
                                                            <dd><code data-testid="publication-exact-source">
                                                                {binding.source_id.clone()}
                                                            </code></dd>
                                                        </div>
                                                        <div>
                                                            <dt>{t(locale.get(), "publication.purpose")}</dt>
                                                            <dd>{binding.purpose.clone()}</dd>
                                                        </div>
                                                        <div>
                                                            <dt>{t(locale.get(), "publication.review")}</dt>
                                                            <dd>{t(locale.get(), &format!(
                                                                "publication.review.{}", binding.review_state,
                                                            ))}</dd>
                                                        </div>
                                                        <div>
                                                            <dt>{t(locale.get(), "publication.reproduction")}</dt>
                                                            <dd>{t(locale.get(), &format!(
                                                                "publication.reproduction.{}", binding.reproduction_state,
                                                            ))}</dd>
                                                        </div>
                                                    </dl>
                                                    </details>
                                                    <h4 class="publication-lineage-title">{t(locale.get(), "publication.analysis_source")}</h4>
                                                    {lineage.map(|lineage| view! {
                                                        <ol class="publication-source-chain">
                                                            <li><span>{t(locale.get(), "publication.sources.files")}</span><strong>{if lineage.input_labels.is_empty() { t(locale.get(), "publication.source_unrecorded").to_string() } else { lineage.input_labels.join(", ") }}</strong></li>
                                                            <li><span>{t(locale.get(), "publication.sources.runs")}</span><strong>{lineage.producing_run_title.clone().unwrap_or_else(|| t(locale.get(), "publication.source_unrecorded").to_string())}</strong>
                                                                {(!lineage.code_labels.is_empty()).then(|| view! { <span>{lineage.code_labels.join(", ")}</span> })}
                                                            </li>
                                                            <li><span>{t(locale.get(), "publication.selected_result")}</span><strong>{lineage.source_label.clone()}</strong></li>
                                                        </ol>
                                                        <details class="publication-lineage-details"><summary>{t(locale.get(), "publication.lineage_details")}</summary>
                                                        <div class="publication-lineage">
                                                            <span class=format!("lineage-quality {}", lineage.quality)>
                                                                {tf(
                                                                    locale.get(),
                                                                    "publication.lineage_quality",
                                                                    &[("quality", &t(
                                                                        locale.get(),
                                                                        &format!("publication.quality.{}", lineage.quality),
                                                                    ))],
                                                                )}
                                                            </span>
                                                            {lineage.version_number.map(|version| view! {
                                                                <span>{format!("v{version}")}</span>
                                                            })}
                                                            {lineage.checksum.map(|checksum| view! {
                                                                <code>{format!("sha256:{}", &checksum[..checksum.len().min(12)])}</code>
                                                            })}
                                                            {lineage.capture_timing.as_deref().map(|timing| view! {
                                                                <span class:warning=timing == "late">
                                                                    {t(locale.get(), &format!("publication.capture.{timing}"))}
                                                                </span>
                                                            })}
                                                            <span>{tf(
                                                                locale.get(),
                                                                "publication.lineage_counts",
                                                                &[
                                                                    ("inputs", &lineage.run_input_count.to_string()),
                                                                    ("outputs", &lineage.run_output_count.to_string()),
                                                                    ("code", &lineage.code_snapshot_count.to_string()),
                                                                ],
                                                            )}</span>
                                                            <span>{if lineage.environment_captured {
                                                                t(locale.get(), "publication.environment_captured")
                                                            } else {
                                                                t(locale.get(), "publication.environment_missing")
                                                            }}</span>
                                                            {(!lineage.bases.is_empty()).then(|| view! {
                                                                <span>{lineage.bases.join(", ")}</span>
                                                            })}
                                                            {lineage.producing_run_id.map(|run_id| view! {
                                                                <code>{format!("run:{run_id}")}</code>
                                                            })}
                                                        </div>
                                                        </details>
                                                    })}
                                                    {drift.filter(|drift| drift.has_drift).map(|drift| view! {
                                                        <div class="publication-drift" role="status">
                                                            {tf(
                                                                locale.get(),
                                                                "publication.drift",
                                                                &[
                                                                    ("bound", &format!("v{} · {}", drift.bound_version_number, drift.bound_version_id)),
                                                                    ("latest", &format!("v{} · {}", drift.latest_version_number, drift.latest_version_id)),
                                                                ],
                                                            )}
                                                        </div>
                                                    })}
                                                    {supersession.map(|supersession| view! {
                                                        <div class="publication-supersession">
                                                            {tf(
                                                                locale.get(),
                                                                "publication.supersession",
                                                                &[
                                                                    ("old", &supersession.old_binding_id),
                                                                    ("new", &supersession.new_binding_id),
                                                                ],
                                                            )}
                                                            <span>{supersession.reason}</span>
                                                        </div>
                                                    })}
                                                    {(!reviews.is_empty()).then(|| view! {
                                                        <div class="publication-reviews">
                                                            {reviews.into_iter().map(|review| view! {
                                                                <span>{format!(
                                                                    "{} · {} · {}",
                                                                    review.reviewer, review.method, review.result,
                                                                )}</span>
                                                            }).collect_view()}
                                                        </div>
                                                    })}
                                                    {(capsule_ready).then(|| {
                                                        reproduction_source_run.map(|source_run_id| {
                                                            let revision = selected_revision_id.clone();
                                                            view! {
                                                                <button type="button"
                                                                    class="secondary publication-verify-run"
                                                                    data-testid="verify-publication-run"
                                                                    disabled=move || busy.get()
                                                                    on:click=move |_| {
                                                                        invoke_workspace(
                                                                            "verify_publication_revision",
                                                                            serde_json::json!({ "input": {
                                                                                "revisionId": revision,
                                                                                "sourceRunId": source_run_id,
                                                                                "comparisons": [],
                                                                            }}),
                                                                            workspace, publication_id, revision_id,
                                                                            selected_item_id, busy, error, move |_| {},
                                                                        );
                                                                    }>
                                                                    {t(locale.get(), "publication.verify_run")}
                                                                </button>
                                                            }
                                                        })
                                                    })}
                                                    {draft.then(|| view! {
                                                        <div class="publication-binding-controls">
                                                            <label>
                                                                <span>{t(locale.get(), "publication.selection")}</span>
                                                                <select prop:value=current_selection.clone()
                                                                    on:change=move |event| {
                                                                        let selection = event_target_value(&event);
                                                                        invoke_workspace(
                                                                            "update_publication_evidence_binding",
                                                                            serde_json::json!({ "input": {
                                                                                "bindingId": binding_id_for_selection,
                                                                                "selectionState": selection,
                                                                                "visibility": current_visibility,
                                                                            }}),
                                                                            workspace, publication_id, revision_id,
                                                                            selected_item_id, busy, error, move |_| {},
                                                                        );
                                                                    }>
                                                                    {["candidate", "selected", "rejected"].into_iter().map(|value| view! {
                                                                        <option value=value selected=selection_for_options == value>
                                                                            {t(locale.get(), &format!("publication.selection.{value}"))}
                                                                        </option>
                                                                    }).collect_view()}
                                                                </select>
                                                            </label>
                                                            <label>
                                                                <span>{t(locale.get(), "publication.visibility")}</span>
                                                                <select prop:value=binding.visibility.clone()
                                                                    on:change=move |event| {
                                                                        let visibility = event_target_value(&event);
                                                                        invoke_workspace(
                                                                            "update_publication_evidence_binding",
                                                                            serde_json::json!({ "input": {
                                                                                "bindingId": binding_id_for_visibility,
                                                                                "selectionState": binding.selection_state,
                                                                                "visibility": visibility,
                                                                            }}),
                                                                            workspace, publication_id, revision_id,
                                                                            selected_item_id, busy, error, move |_| {},
                                                                        );
                                                                    }>
                                                                    {["public", "restricted", "private"].into_iter().map(|value| view! {
                                                                        <option value=value selected=visibility_for_options == value>
                                                                            {t(locale.get(), &format!("publication.visibility.{value}"))}
                                                                        </option>
                                                                    }).collect_view()}
                                                                </select>
                                                            </label>
                                                        </div>
                                                    })}
                                                </article>
                                            }
                                        }).collect_view()
                                    }}
                                </section>

                                <aside class="publication-readiness" hidden=move || page.get() != "readiness">
                                    <h3>{t(locale.get(), "publication.readiness")}</h3>
                                    {draft.then(|| view! {
                                        <section class="publication-policy-inline">
                                            <p class="publication-help">{t(locale.get(), "publication.finalize_hint")}</p>
                                            <label><span>{t(locale.get(), "publication.target_visibility")}</span>
                                                <select disabled=move || busy.get() prop:value=move || freeze_visibility.get()
                                                    on:change=move |event| freeze_visibility.set(event_target_value(&event))>
                                                    {["private", "restricted", "public"].into_iter().map(|value| view! { <option value=value prop:selected=move || freeze_visibility.get() == value>{t(locale.get(), &format!("publication.visibility.{value}"))}</option> }).collect_view()}
                                                </select>
                                            </label>
                                            <Show when=move || freeze_visibility.get() == "public">
                                                <label class="publication-check"><input type="checkbox" disabled=move || busy.get() prop:checked=move || freeze_phi_reviewed.get() on:change=move |event| freeze_phi_reviewed.set(event_target_checked(&event)) /><span>{t(locale.get(), "publication.phi_reviewed")}</span></label>
                                                <label class="publication-check"><input type="checkbox" disabled=move || busy.get() prop:checked=move || freeze_redistribution_reviewed.get() on:change=move |event| freeze_redistribution_reviewed.set(event_target_checked(&event)) /><span>{t(locale.get(), "publication.redistribution_reviewed")}</span></label>
                                            </Show>
                                            <Show when=move || freeze_visibility.get() != "public">
                                                <label class="publication-check"><input type="checkbox" disabled=move || busy.get() prop:checked=move || freeze_restricted_bytes.get() on:change=move |event| freeze_restricted_bytes.set(event_target_checked(&event)) /><span>{t(locale.get(), "publication.snapshot_restricted")}</span></label>
                                            </Show>
                                            <div class="publication-policy-actions">
                                                <button type="button" class="secondary" data-testid="check-publication" disabled=move || busy.get() on:click=move |_| run_check.call(false)>{compose_icon("check")}{t(locale.get(), "publication.check_action")}</button>
                                                <button type="button" class="primary" data-testid="freeze-publication" disabled=move || busy.get() || !transient_readiness.get().is_some_and(|r| r.can_freeze) on:click=move |_| run_check.call(true)>{compose_icon("lock")}{t(locale.get(), "publication.freeze_action")}</button>
                                            </div>
                                        </section>
                                    })}
                                    <p class="publication-help">{t(locale.get(), "publication.reproduction_hint")}</p>

                                    {readiness.map(|readiness| {
                                        let on_waive = Callback::new(move |code: String| {
                                            waiver_code.set(Some(code));
                                            waiver_reason.set(String::new());
                                        });
                                        view! {
                                            <div class="publication-readiness-summary">
                                                <span>{t(locale.get(), &format!(
                                                    "publication.capability.{}", readiness.capability_level,
                                                ))}</span>
                                                <span>{t(locale.get(), &format!(
                                                    "publication.visibility.{}", readiness.target_visibility,
                                                ))}</span>
                                                {(!readiness.manifest_sha256.is_empty()).then(|| view! {
                                                    <code>{format!(
                                                        "sha256:{}",
                                                        &readiness.manifest_sha256[
                                                            ..readiness.manifest_sha256.len().min(12)
                                                        ],
                                                    )}</code>
                                                })}
                                            </div>
                                            {finding_rows(
                                                locale.get(),
                                                "publication.blockers",
                                                "blockers",
                                                readiness.blockers,
                                                draft,
                                                on_waive,
                                            )}
                                            {finding_rows(
                                                locale.get(),
                                                "publication.warnings",
                                                "warnings",
                                                readiness.warnings,
                                                draft,
                                                on_waive,
                                            )}
                                            {finding_rows(
                                                locale.get(),
                                                "publication.omissions",
                                                "omissions",
                                                readiness.omissions,
                                                draft,
                                                on_waive,
                                            )}
                                        }
                                        .into_view()
                                    }).unwrap_or_else(|| view! {
                                            <div class="publication-empty">
                                                {t(locale.get(), "publication.readiness_empty")}
                                            </div>
                                        }
                                        .into_view())}
                                    {(!current_for_readiness.waivers.is_empty()).then(|| view! {
                                        <section class="publication-waiver-list">
                                            <h4>{t(locale.get(), "publication.waivers")}</h4>
                                            {current_for_readiness.waivers.into_iter().map(|waiver| view! {
                                                <div>
                                                    <code>{waiver.finding_code}</code>
                                                    <span>{waiver.reason}</span>
                                                </div>
                                            }).collect_view()}
                                        </section>
                                    })}
                                    {revision.and_then(|revision| revision.manifest_sha256).map(|hash| view! {
                                        <div class="publication-frozen-manifest">
                                            <span>{t(locale.get(), "publication.frozen_manifest")}</span>
                                            <code>{hash}</code>
                                        </div>
                                    })}
                                    {(!reproduction_runs.is_empty()).then(|| {
                                        let all_results = reproduction_results.clone();
                                        view! {
                                            <section class="publication-reproduction-runs"
                                                data-testid="publication-reproduction-runs">
                                                <h4>{t(locale.get(), "publication.reproduction_runs")}</h4>
                                                {reproduction_runs.into_iter().map(|run| {
                                                    let results = all_results.iter()
                                                        .filter(|result| result.reproduction_run_id == run.id)
                                                        .cloned()
                                                        .collect::<Vec<_>>();
                                                    let environment = if run.environment_matched {
                                                        t(locale.get(), "publication.environment_matched")
                                                    } else {
                                                        t(locale.get(), "publication.environment_mismatch")
                                                    };
                                                    view! {
                                                        <article data-reproduction-run-id=run.id>
                                                            <div>
                                                                <strong>{t(locale.get(), &format!(
                                                                    "publication.reproduction_status.{}", run.status,
                                                                ))}</strong>
                                                                <span>{t(locale.get(), &format!(
                                                                    "publication.capability.{}", run.capability_level,
                                                                ))}</span>
                                                            </div>
                                                            <code>{format!("run:{}", run.source_run_id)}</code>
                                                            <span class:warning=!run.environment_matched>{environment}</span>
                                                            {run.exit_code.map(|code| view! {
                                                                <span>{format!("exit {code}")}</span>
                                                            })}
                                                            {results.into_iter().map(|result| view! {
                                                                <div class="publication-reproduction-result"
                                                                    data-passed=result.passed.to_string()>
                                                                    <span>{if result.passed { "✓" } else { "✗" }}</span>
                                                                    <code>{result.output_path}</code>
                                                                    <span>{result.comparator_kind}</span>
                                                                </div>
                                                            }).collect_view()}
                                                            {run.error.map(|message| view! {
                                                                <span class="publication-capsule-error">{message}</span>
                                                            })}
                                                        </article>
                                                    }
                                                }).collect_view()}
                                            </section>
                                        }
                                    })}
                                    {(!capsule_builds.is_empty()).then(|| view! {
                                        <section class="publication-capsule-builds"
                                            data-testid="publication-capsule-builds">
                                            <h4>{t(locale.get(), "publication.capsule_builds")}</h4>
                                            {capsule_builds.into_iter().map(|build| {
                                                let status = t(
                                                    locale.get(),
                                                    &format!("publication.capsule_status.{}", build.status),
                                                );
                                                view! {
                                                    <article data-capsule-build-id=build.id>
                                                        <div>
                                                            <strong>{status}</strong>
                                                            <span>{format!("{} · {}", build.format, build.visibility)}</span>
                                                        </div>
                                                        {build.archive_sha256.map(|hash| view! {
                                                            <code>{format!(
                                                                "sha256:{}",
                                                                &hash[..hash.len().min(12)],
                                                            )}</code>
                                                        })}
                                                        {build.output_path.map(|path| view! {
                                                            <span class="publication-capsule-path">{path}</span>
                                                        })}
                                                        {build.error.map(|message| view! {
                                                            <span class="publication-capsule-error">{message}</span>
                                                        })}
                                                    </article>
                                                }
                                            }).collect_view()}
                                        </section>
                                    })}
                                </aside>
                            </div>
                        }.into_view()
                    }
                }}

                // Inside the page section: as a sibling it rendered below the
                // full-height section, off-screen right after "Explain its use".
                {move || binding_source.get().map(|source| {
                    let current = workspace.get();
                    let revisions = current.as_ref().map(|workspace| workspace.revisions.clone()).unwrap_or_default();
                    let items = current.as_ref().map(|workspace| workspace.items.clone()).unwrap_or_default();
                    let has_draft = revisions.iter().any(|revision| revision.state == "draft");
                    let selected_is_draft = revisions.iter().any(|revision| {
                        revision.id == binding_revision.get() && revision.state == "draft"
                    });
                    view! {
                        <div class="publication-binding-step">
                            <section class="publication-binding-dialog" role="region"
                                data-testid="publication-binding-dialog"
                                aria-labelledby="publication-binding-title"
                                on:click=|event| event.stop_propagation()>
                                <header>
                                    <div>
                                        <h3 id="publication-binding-title">
                                            {t(locale.get(), "publication.bind_title")}
                                        </h3>
                                        <p>{source.label.clone()}</p>
                                    </div>
                                    <button type="button" class="ps-close"
                                        aria-label=t(locale.get(), "publication.close")
                                        on:click=move |_| {
                                            binding_source.set(None);
                                            binding_seen.set(String::new());
                                        }>
                                        {compose_icon("close")}
                                    </button>
                                </header>
                                {if !has_draft {
                                    view! {
                                        <div class="publication-empty">
                                            {t(locale.get(), "publication.bind_requires_draft")}
                                        </div>
                                    }.into_view()
                                } else {
                                    view! {
                                        <div class="publication-form-grid">
                                            <label>
                                                <span>{t(locale.get(), "publication.revision")}</span>
                                                <select prop:value=move || binding_revision.get()
                                                    on:change=move |event| {
                                                        let selected = event_target_value(&event);
                                                        binding_revision.set(selected.clone());
                                                        binding_item.set(String::new());
                                                        binding_claim.set(String::new());
                                                        revision_id.set(selected);
                                                        refresh_workspace(
                                                            workspace, publication_id, revision_id,
                                                            selected_item_id, loading, error,
                                                        );
                                                    }>
                                                    {revisions.into_iter().map(|revision| {
                                                        let id = revision.id.clone();
                                                        let label = format!(
                                                            "{} · {}",
                                                            revision.label,
                                                            t(locale.get(), &format!("publication.state.{}", revision.state)),
                                                        );
                                                        view! {
                                                            <option value=revision.id disabled=revision.state != "draft"
                                                                prop:selected=move || binding_revision.get() == id>
                                                                {label}
                                                            </option>
                                                        }
                                                    }).collect_view()}
                                                </select>
                                            </label>
                                            <label>
                                                <span>{t(locale.get(), "publication.item_target")}</span>
                                                <select prop:value=move || binding_item.get()
                                                    on:change=move |event| binding_item.set(event_target_value(&event))>
                                                    <option value="" prop:selected=move || binding_item.get().is_empty()>
                                                        {t(locale.get(), "publication.unassigned")}
                                                    </option>
                                                    {items.clone().into_iter().map(|item| {
                                                        let id = item.id.clone();
                                                        view! {
                                                            <option value=item.id prop:selected=move || binding_item.get() == id>
                                                                {item.title}
                                                            </option>
                                                        }
                                                    }).collect_view()}
                                                </select>
                                            </label>
                                            <label class="wide">
                                                <span>{t(locale.get(), "publication.purpose")}</span>
                                                <textarea prop:value=move || binding_purpose.get()
                                                    on:input=move |event| binding_purpose.set(event_target_value(&event))></textarea>
                                            </label>
                                            <label>
                                                <span>{t(locale.get(), "publication.supported_claim")}</span>
                                                <select prop:value=move || binding_claim.get()
                                                    on:change=move |event| binding_claim.set(event_target_value(&event))>
                                                    <option value="" prop:selected=move || binding_claim.get().is_empty()>
                                                        {t(locale.get(), "publication.no_claim")}
                                                    </option>
                                                    {items.into_iter().filter(|item| item.kind == "claim")
                                                        .map(|item| {
                                                            let id = item.id.clone();
                                                            view! {
                                                                <option value=item.id prop:selected=move || binding_claim.get() == id>
                                                                    {item.title}
                                                                </option>
                                                            }
                                                        }).collect_view()}
                                                </select>
                                            </label>
                                            <label>
                                                <span>{t(locale.get(), "publication.selection")}</span>
                                                <select prop:value=move || binding_selection.get()
                                                    on:change=move |event| binding_selection.set(event_target_value(&event))>
                                                    {["candidate", "selected", "rejected"].into_iter().map(|value| view! {
                                                        <option value=value prop:selected=move || binding_selection.get() == value>
                                                            {t(locale.get(), &format!("publication.selection.{value}"))}
                                                        </option>
                                                    }).collect_view()}
                                                </select>
                                            </label>
                                            <label>
                                                <span>{t(locale.get(), "publication.visibility")}</span>
                                                <select prop:value=move || binding_visibility.get()
                                                    on:change=move |event| binding_visibility.set(event_target_value(&event))>
                                                    {["public", "restricted", "private"].into_iter().map(|value| view! {
                                                        <option value=value prop:selected=move || binding_visibility.get() == value>
                                                            {t(locale.get(), &format!("publication.visibility.{value}"))}
                                                        </option>
                                                    }).collect_view()}
                                                </select>
                                            </label>
                                        </div>
                                        <footer>
                                            <button type="button" class="secondary"
                                                on:click=move |_| {
                                                    binding_source.set(None);
                                                    binding_seen.set(String::new());
                                                }>
                                                {t(locale.get(), "publication.cancel")}
                                            </button>
                                            <button type="button" class="primary"
                                                disabled=move || busy.get() || !selected_is_draft
                                                    || binding_purpose.get().trim().is_empty()
                                                on:click={
                                                    let source = source.clone();
                                                    move |_| {
                                                        let selected_revision = binding_revision.get_untracked();
                                                        let args = serde_json::json!({ "input": {
                                                            "revisionId": selected_revision,
                                                            "itemId": match binding_item.get_untracked() {
                                                                value if value.is_empty() => None,
                                                                value => Some(value),
                                                            },
                                                            "sourceKind": source.kind,
                                                            "sourceId": source.id,
                                                            "purpose": binding_purpose.get_untracked(),
                                                            "supportedClaimItemId": match binding_claim.get_untracked() {
                                                                value if value.is_empty() => None,
                                                                value => Some(value),
                                                            },
                                                            "selectionState": binding_selection.get_untracked(),
                                                            "visibility": binding_visibility.get_untracked(),
                                                        }});
                                                        invoke_workspace(
                                                            "bind_publication_evidence", args,
                                                            workspace, publication_id, revision_id,
                                                            selected_item_id, busy, error,
                                                            move |ok| if ok {
                                                                transient_readiness.set(None);
                                                                page.set("evidence");
                                                                binding_source.set(None);
                                                                binding_seen.set(String::new());
                                                            },
                                                        );
                                                    }
                                                }>
                                                {t(locale.get(), "publication.bind_action")}
                                            </button>
                                        </footer>
                                    }.into_view()
                                }}
                            </section>
                        </div>
                    }
                })}
            </section>

            {move || anchor_open.get().then(|| view! {
                <div class="overlay publication-nested-overlay" role="presentation"
                    on:click=move |_| anchor_open.set(false)>
                    <section class="modal publication-anchor-dialog" role="dialog"
                        aria-modal="true" aria-labelledby="publication-anchor-title"
                        data-testid="publication-anchor-dialog"
                        on:click=|event| event.stop_propagation()>
                        <header>
                            <div>
                                <h3 id="publication-anchor-title">
                                    {t(locale.get(), "publication.add_precise")}
                                </h3>
                                <p>{t(locale.get(), "publication.anchor_hint")}</p>
                            </div>
                            <button type="button" class="ps-close"
                                aria-label=t(locale.get(), "publication.close")
                                on:click=move |_| anchor_open.set(false)>
                                {compose_icon("close")}
                            </button>
                        </header>
                        <label>
                            <span>{t(locale.get(), "publication.anchor_kind")}</span>
                            <select data-testid="publication-anchor-kind"
                                prop:value=move || anchor_kind.get()
                                on:change=move |event| {
                                    anchor_kind.set(event_target_value(&event));
                                    error.set(None);
                                }>
                                {[
                                    "message_span",
                                    "tool_call",
                                    "execution_log",
                                    "code_cell",
                                    "external_resource",
                                ].into_iter().map(|kind| view! {
                                    <option value=kind prop:selected=move || anchor_kind.get() == kind>
                                        {t(locale.get(), &format!("publication.source.{kind}"))}
                                    </option>
                                }).collect_view()}
                            </select>
                        </label>
                        {move || match anchor_kind.get().as_str() {
                            "message_span" => view! {
                                <div class="publication-form-grid">
                                    <label>
                                        <span>{t(locale.get(), "publication.frame_id")}</span>
                                        <input type="text" data-testid="publication-anchor-frame"
                                            prop:value=move || anchor_frame.get()
                                            on:input=move |event| anchor_frame.set(event_target_value(&event)) />
                                    </label>
                                    <label>
                                        <span>{t(locale.get(), "publication.message_seq")}</span>
                                        <input type="number" min="1"
                                            prop:value=move || anchor_message_seq.get()
                                            on:input=move |event| anchor_message_seq.set(event_target_value(&event)) />
                                    </label>
                                    <label>
                                        <span>{t(locale.get(), "publication.byte_start")}</span>
                                        <input type="number" min="0"
                                            prop:value=move || anchor_byte_start.get()
                                            on:input=move |event| anchor_byte_start.set(event_target_value(&event)) />
                                    </label>
                                    <label>
                                        <span>{t(locale.get(), "publication.byte_end")}</span>
                                        <input type="number" min="1"
                                            prop:value=move || anchor_byte_end.get()
                                            on:input=move |event| anchor_byte_end.set(event_target_value(&event)) />
                                    </label>
                                </div>
                            }.into_view(),
                            "tool_call" => view! {
                                <div class="publication-form-grid">
                                    <label>
                                        <span>{t(locale.get(), "publication.frame_id")}</span>
                                        <input type="text" prop:value=move || anchor_frame.get()
                                            on:input=move |event| anchor_frame.set(event_target_value(&event)) />
                                    </label>
                                    <label>
                                        <span>{t(locale.get(), "publication.message_seq")}</span>
                                        <input type="number" min="1"
                                            prop:value=move || anchor_message_seq.get()
                                            on:input=move |event| anchor_message_seq.set(event_target_value(&event)) />
                                    </label>
                                    <label class="wide">
                                        <span>{t(locale.get(), "publication.tool_call_id")}</span>
                                        <input type="text" prop:value=move || anchor_tool_call.get()
                                            on:input=move |event| anchor_tool_call.set(event_target_value(&event)) />
                                    </label>
                                </div>
                            }.into_view(),
                            _ => view! {
                                <label>
                                    <span>{t(locale.get(), "publication.exact_source")}</span>
                                    <input type="text" data-testid="publication-anchor-exact-id"
                                        prop:value=move || anchor_exact_id.get()
                                        on:input=move |event| anchor_exact_id.set(event_target_value(&event)) />
                                </label>
                            }.into_view(),
                        }}
                        <footer>
                            <button type="button" class="secondary"
                                on:click=move |_| anchor_open.set(false)>
                                {t(locale.get(), "publication.cancel")}
                            </button>
                            <button type="button" class="primary"
                                data-testid="publication-anchor-continue"
                                on:click=move |_| {
                                    match precise_evidence_source(
                                        &anchor_kind.get_untracked(),
                                        &anchor_frame.get_untracked(),
                                        &anchor_message_seq.get_untracked(),
                                        &anchor_byte_start.get_untracked(),
                                        &anchor_byte_end.get_untracked(),
                                        &anchor_tool_call.get_untracked(),
                                        &anchor_exact_id.get_untracked(),
                                    ) {
                                        Ok(source) => {
                                            error.set(None);
                                            anchor_open.set(false);
                                            binding_source.set(Some(source));
                                        }
                                        Err(message) => error.set(Some(message)),
                                    }
                                }>
                                {t(locale.get(), "publication.continue")}
                            </button>
                        </footer>
                    </section>
                </div>
            })}

            {move || waiver_code.get().map(|code| {
                let selected_revision = revision_id.get();
                view! {
                    <div class="overlay publication-nested-overlay" role="presentation"
                        on:click=move |_| waiver_code.set(None)>
                        <section class="modal publication-waiver-dialog" role="dialog"
                            aria-modal="true" aria-labelledby="publication-waiver-title"
                            on:click=|event| event.stop_propagation()>
                            <header>
                                <div>
                                    <h3 id="publication-waiver-title">
                                        {t(locale.get(), "publication.waive_title")}
                                    </h3>
                                    <code>{code.clone()}</code>
                                </div>
                                <button type="button" class="ps-close"
                                    aria-label=t(locale.get(), "publication.close")
                                    on:click=move |_| waiver_code.set(None)>
                                    {compose_icon("close")}
                                </button>
                            </header>
                            <label>
                                <span>{t(locale.get(), "publication.waiver_author")}</span>
                                <input type="text" prop:value=move || waiver_author.get()
                                    on:input=move |event| waiver_author.set(event_target_value(&event)) />
                            </label>
                            <label>
                                <span>{t(locale.get(), "publication.waiver_reason")}</span>
                                <textarea prop:value=move || waiver_reason.get()
                                    on:input=move |event| waiver_reason.set(event_target_value(&event))></textarea>
                            </label>
                            <footer>
                                <button type="button" class="secondary"
                                    on:click=move |_| waiver_code.set(None)>
                                    {t(locale.get(), "publication.cancel")}
                                </button>
                                <button type="button" class="primary"
                                    disabled=move || busy.get() || waiver_author.get().trim().is_empty()
                                        || waiver_reason.get().trim().is_empty()
                                    on:click={
                                        let code = code.clone();
                                        move |_| {
                                            let author = waiver_author.get_untracked();
                                            let reason = waiver_reason.get_untracked();
                                            let code_for_readiness = code.clone();
                                            invoke_workspace(
                                                "save_publication_waiver",
                                                serde_json::json!({ "input": {
                                                    "revisionId": selected_revision,
                                                    "findingCode": code,
                                                    "author": author.clone(),
                                                    "reason": reason.clone(),
                                                }}),
                                                workspace, publication_id, revision_id,
                                                selected_item_id, busy, error,
                                                move |ok| if ok {
                                                    transient_readiness.update(|readiness| {
                                                        let Some(readiness) = readiness else { return; };
                                                        for finding in readiness.blockers.iter_mut()
                                                            .chain(readiness.warnings.iter_mut())
                                                            .chain(readiness.omissions.iter_mut())
                                                        {
                                                            if finding.code == code_for_readiness {
                                                                finding.waived = true;
                                                                finding.waiver = Some(PublicationWaiverInfo {
                                                                    finding_code: code_for_readiness.clone(),
                                                                    author: author.clone(),
                                                                    reason: reason.clone(),
                                                                });
                                                            }
                                                        }
                                                    });
                                                    waiver_code.set(None);
                                                    waiver_reason.set(String::new());
                                                },
                                            );
                                        }
                                    }>
                                    {t(locale.get(), "publication.save_waiver")}
                                </button>
                            </footer>
                        </section>
                    </div>
                }
            })}
        </div>
    }
}

fn event_target_checked(event: &web_sys::Event) -> bool {
    event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|input| input.checked())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, parent: Option<&str>, ordinal: i64) -> PublicationItemInfo {
        PublicationItemInfo {
            id: id.into(),
            revision_id: "revision".into(),
            parent_item_id: parent.map(str::to_string),
            kind: "section".into(),
            title: id.into(),
            content: String::new(),
            ordinal,
        }
    }

    #[test]
    fn manuscript_rows_follow_parent_order_and_keep_orphans() {
        let rows = manuscript_rows(&[
            item("child", Some("root"), 0),
            item("later", None, 2),
            item("root", None, 1),
            item("orphan", Some("missing"), 0),
        ]);
        assert_eq!(
            rows.iter()
                .map(|(item, depth)| (item.id.as_str(), *depth))
                .collect::<Vec<_>>(),
            [("root", 0), ("child", 1), ("later", 0), ("orphan", 0),]
        );
    }

    #[test]
    fn message_span_source_uses_canonical_exact_locator() {
        let source =
            precise_evidence_source("message_span", "frame", "8", "2", "11", "", "").unwrap();
        assert_eq!(source.kind, "message_span");
        assert_eq!(
            source.id,
            r#"{"byte_end":11,"byte_start":2,"frame_id":"frame","message_seq":8}"#
        );
    }
}
