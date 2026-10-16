//! Researcher-private project memos, shown as a right-pane tab.
//!
//! A memo is a note only the researcher sees: the agent never reads this list,
//! and nothing here injects text into a turn. The one bridge to the model is
//! the Insert action, which drops a memo's body into the composer; the memo
//! reaches the model only when the researcher sends that message.
//!
//! Escape is consumed top-down inside the pane through `window_capture_escape`:
//! an open card menu, then the discard confirmation, then an open editor. With
//! nothing of its own open the pane leaves Escape to the app stack, which
//! dismisses the right pane only where it is a drawer (narrow windows).
use crate::app_support::{compose_icon, copy_text, format_relative_time};
use crate::dto::{ProjectMemo, ProjectMemoInput, ProjectMemoPatch};
use crate::i18n::{t, Locale};
use crate::research_journey::call;
use crate::window_capture_escape;
use leptos::*;
use std::time::Duration;

/// Lines of the body a collapsed card shows before the fade.
const PREVIEW_LINES: usize = 2;
const EDITOR_MIN_ROWS: usize = 6;
const EDITOR_MAX_ROWS: usize = 24;

/// A memo being written or edited. Owned by the app so it survives closing the
/// Memos tab; it belongs to one project and is dropped when another project
/// becomes active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MemoDraft {
    pub(crate) project_id: String,
    /// `None` while creating a memo.
    pub(crate) id: Option<String>,
    pub(crate) title: String,
    pub(crate) body: String,
    original_title: String,
    original_body: String,
}

impl MemoDraft {
    pub(crate) fn new(project_id: String) -> Self {
        Self {
            project_id,
            id: None,
            title: String::new(),
            body: String::new(),
            original_title: String::new(),
            original_body: String::new(),
        }
    }

    pub(crate) fn edit(project_id: String, memo: &ProjectMemo) -> Self {
        Self {
            project_id,
            id: Some(memo.id.clone()),
            title: memo.title.clone(),
            body: memo.body.clone(),
            original_title: memo.title.clone(),
            original_body: memo.body.clone(),
        }
    }

    pub(crate) fn dirty(&self) -> bool {
        self.title != self.original_title || self.body != self.original_body
    }

    pub(crate) fn can_save(&self) -> bool {
        !self.body.trim().is_empty()
    }

    fn editor_rows(&self) -> usize {
        (self.body.lines().count() + 1).clamp(EDITOR_MIN_ROWS, EDITOR_MAX_ROWS)
    }
}

/// What the composer holds after inserting `body`: the memo alone when the
/// draft is blank, otherwise appended after one empty line. The title is the
/// researcher's index and never travels with the body.
pub(crate) fn compose_insert(current: &str, body: &str) -> String {
    if current.trim().is_empty() {
        body.to_string()
    } else {
        format!("{}\n\n{}", current.trim_end(), body)
    }
}

/// The card heading: the title, else the first non-empty body line, else
/// `None` so the UI can show an "untitled" placeholder.
pub(crate) fn memo_heading(memo: &ProjectMemo) -> Option<String> {
    let title = memo.title.trim();
    if !title.is_empty() {
        return Some(title.to_string());
    }
    memo.body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// Case-insensitive match on title and body.
pub(crate) fn filter_memos(memos: &[ProjectMemo], query: &str) -> Vec<ProjectMemo> {
    let query = query.trim().to_lowercase();
    memos
        .iter()
        .filter(|memo| {
            query.is_empty()
                || memo.title.to_lowercase().contains(&query)
                || memo.body.to_lowercase().contains(&query)
        })
        .cloned()
        .collect()
}

/// The store's order, kept locally after optimistic edits: pinned first, then
/// most recently updated.
pub(crate) fn sort_memos(memos: &mut [ProjectMemo]) {
    memos.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.updated_at.cmp(&a.updated_at))
            .then(b.id.cmp(&a.id))
    });
}

fn upsert(memos: RwSignal<Vec<ProjectMemo>>, saved: ProjectMemo) {
    memos.update(|list| {
        match list.iter_mut().find(|memo| memo.id == saved.id) {
            Some(existing) => *existing = saved,
            None => list.push(saved),
        }
        sort_memos(list);
    });
}

#[component]
pub(crate) fn MemosPane(
    locale: RwSignal<Locale>,
    /// The window's working project. The list reloads when it changes and a
    /// draft from another project is dropped.
    project_id: Signal<Option<String>>,
    draft: RwSignal<Option<MemoDraft>>,
    /// Receives a memo body for the composer; the pane never sends anything.
    on_insert: Callback<String>,
) -> impl IntoView {
    let memos = create_rw_signal(Vec::<ProjectMemo>::new());
    let loading = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);
    let save_error = create_rw_signal(None::<String>);
    let busy = create_rw_signal(false);
    let query = create_rw_signal(String::new());
    let expanded = create_rw_signal(None::<String>);
    let menu = create_rw_signal(None::<String>);
    let inserted = create_rw_signal(None::<String>);
    let confirm_discard = create_rw_signal(false);
    // Which memo is open in the editor, if any. Views that mount or place the
    // editor track this instead of `draft`: the draft changes on every
    // keystroke, and re-rendering the editor then would replace the focused
    // input after each character.
    let open_draft = create_memo(move |_| {
        draft.with(|d| d.as_ref().map(|d| (d.project_id.clone(), d.id.clone())))
    });

    create_effect(move |_| {
        let project = project_id.get();
        if let Some(current) = draft.get_untracked() {
            if project.as_deref() != Some(current.project_id.as_str()) {
                draft.set(None);
                confirm_discard.set(false);
            }
        }
        memos.set(Vec::new());
        error.set(None);
        if project.is_none() {
            return;
        }
        loading.set(true);
        spawn_local(async move {
            match call::<Vec<ProjectMemo>>("list_project_memos", serde_json::json!({})).await {
                Ok(list) => memos.set(list),
                Err(message) => error.set(Some(message)),
            }
            loading.set(false);
        });
    });

    // Top-down: card menu → discard confirmation → editor. A clean editor
    // closes at once; a dirty one asks first.
    let close_editor = move || match draft.get_untracked() {
        Some(current) if current.dirty() => confirm_discard.set(true),
        _ => {
            draft.set(None);
            save_error.set(None);
        }
    };
    window_capture_escape(move || {
        if menu.get_untracked().is_some() {
            menu.set(None);
            return true;
        }
        if confirm_discard.get_untracked() {
            confirm_discard.set(false);
            return true;
        }
        if draft.get_untracked().is_some() {
            close_editor();
            return true;
        }
        false
    });

    let start_new = move || {
        let Some(project) = project_id.get_untracked() else {
            return;
        };
        if draft.get_untracked().is_some() {
            return;
        }
        menu.set(None);
        save_error.set(None);
        draft.set(Some(MemoDraft::new(project)));
    };
    let start_edit = move |memo: ProjectMemo| {
        let Some(project) = project_id.get_untracked() else {
            return;
        };
        if draft.get_untracked().is_some() {
            return;
        }
        menu.set(None);
        save_error.set(None);
        draft.set(Some(MemoDraft::edit(project, &memo)));
    };
    let save = move || {
        let Some(current) = draft.get_untracked() else {
            return;
        };
        if !current.can_save() || busy.get_untracked() {
            return;
        }
        busy.set(true);
        save_error.set(None);
        spawn_local(async move {
            let result = match &current.id {
                None => {
                    call::<ProjectMemo>(
                        "add_project_memo",
                        serde_json::json!({ "input": ProjectMemoInput {
                            title: current.title.clone(),
                            body: current.body.clone(),
                        } }),
                    )
                    .await
                }
                Some(id) => {
                    call::<ProjectMemo>(
                        "update_project_memo",
                        serde_json::json!({ "patch": ProjectMemoPatch {
                            id: id.clone(),
                            title: Some(current.title.clone()),
                            body: Some(current.body.clone()),
                            pinned: None,
                        } }),
                    )
                    .await
                }
            };
            busy.set(false);
            match result {
                Ok(saved) => {
                    upsert(memos, saved);
                    draft.set(None);
                    confirm_discard.set(false);
                }
                Err(message) => save_error.set(Some(message)),
            }
        });
    };
    let toggle_pin = move |memo: ProjectMemo| {
        menu.set(None);
        let before = memos.get_untracked();
        let pinned = !memo.pinned;
        memos.update(|list| {
            if let Some(target) = list.iter_mut().find(|entry| entry.id == memo.id) {
                target.pinned = pinned;
            }
            sort_memos(list);
        });
        spawn_local(async move {
            let result = call::<ProjectMemo>(
                "update_project_memo",
                serde_json::json!({ "patch": ProjectMemoPatch {
                    id: memo.id.clone(),
                    pinned: Some(pinned),
                    ..Default::default()
                } }),
            )
            .await;
            match result {
                Ok(saved) => upsert(memos, saved),
                Err(message) => {
                    memos.set(before);
                    error.set(Some(message));
                }
            }
        });
    };
    let delete = move |memo: ProjectMemo| {
        menu.set(None);
        let before = memos.get_untracked();
        memos.update(|list| list.retain(|entry| entry.id != memo.id));
        spawn_local(async move {
            if let Err(message) = call::<()>(
                "delete_project_memo",
                serde_json::json!({ "memoId": memo.id }),
            )
            .await
            {
                memos.set(before);
                error.set(Some(message));
            }
        });
    };
    let insert = move |memo: ProjectMemo| {
        let id = memo.id.clone();
        on_insert.call(memo.body);
        inserted.set(Some(id.clone()));
        set_timeout(
            move || {
                if inserted.get_untracked().as_deref() == Some(id.as_str()) {
                    inserted.set(None);
                }
            },
            Duration::from_millis(1500),
        );
    };

    // Built once per open draft: the inputs take their starting text untracked
    // and only write back, so typing never rebuilds them.
    let editor = move || {
        let current = draft.get_untracked()?;
        let rows =
            move || draft.with(|d| d.as_ref().map_or(EDITOR_MIN_ROWS, MemoDraft::editor_rows));
        let can_save = move || draft.with(|d| d.as_ref().is_some_and(MemoDraft::can_save));
        let discard = move |_| {
            confirm_discard.set(false);
            draft.set(None);
            save_error.set(None);
        };
        Some(view! {
            <article class="memo-card memo-editor" data-testid="memo-editor"
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if ev.key() == "Enter" && (ev.meta_key() || ev.ctrl_key()) {
                        ev.prevent_default();
                        save();
                    }
                }>
                <input type="text" class="memo-title-input" data-testid="memo-title-input"
                    placeholder=move || t(locale.get(), "memos.title_placeholder")
                    aria-label=move || t(locale.get(), "memos.title_placeholder")
                    prop:value=current.title
                    on:input=move |ev| draft.update(|d| if let Some(d) = d { d.title = event_target_value(&ev); }) />
                <textarea class="memo-body-input" data-testid="memo-body-input" rows=rows
                    placeholder=move || t(locale.get(), "memos.body_placeholder")
                    aria-label=move || t(locale.get(), "memos.body_placeholder")
                    prop:value=current.body
                    on:input=move |ev| draft.update(|d| if let Some(d) = d { d.body = event_target_value(&ev); })></textarea>
                {move || save_error.get().map(|message| view! { <div class="memo-error" role="alert">{message}</div> })}
                {move || if confirm_discard.get() {
                    view! {
                        <div class="memo-confirm" role="alertdialog" data-testid="memo-discard-confirm"
                            aria-label=move || t(locale.get(), "memos.discard_q")>
                            <span>{move || t(locale.get(), "memos.discard_q")}</span>
                            <button type="button" class="memo-btn danger" on:click=discard>{move || t(locale.get(), "memos.discard")}</button>
                            <button type="button" class="memo-btn" on:click=move |_| confirm_discard.set(false)>{move || t(locale.get(), "memos.keep_editing")}</button>
                        </div>
                    }.into_view()
                } else {
                    view! {
                        <div class="memo-editor-actions">
                            <span class="memo-shortcut" aria-hidden="true">"⌘↩ / Ctrl+↩"</span>
                            <button type="button" class="memo-btn" on:click=move |_| close_editor()>{move || t(locale.get(), "memos.cancel")}</button>
                            <button type="button" class="memo-btn primary" data-testid="memo-save"
                                disabled=move || !can_save() || busy.get()
                                on:click=move |_| save()>{move || t(locale.get(), "memos.save")}</button>
                        </div>
                    }.into_view()
                }}
            </article>
        })
    };

    let card = move |memo: ProjectMemo| {
        let id = memo.id.clone();
        let heading = memo_heading(&memo);
        let is_expanded = Signal::derive({
            let id = id.clone();
            move || expanded.get().as_deref() == Some(id.as_str())
        });
        let toggle_expanded = {
            let id = id.clone();
            move |_| {
                expanded.update(|current| {
                    if current.as_deref() == Some(id.as_str()) {
                        *current = None;
                    } else {
                        *current = Some(id.clone());
                    }
                })
            }
        };
        let menu_open = Signal::derive({
            let id = id.clone();
            move || menu.get().as_deref() == Some(id.as_str())
        });
        let just_inserted = Signal::derive({
            let id = id.clone();
            move || inserted.get().as_deref() == Some(id.as_str())
        });
        let edit_memo = memo.clone();
        let insert_memo = memo.clone();
        let pin_memo = memo.clone();
        let copy_body = memo.body.clone();
        let delete_memo = memo.clone();
        let pinned = memo.pinned;
        let updated_at = memo.updated_at;
        let body = memo.body.clone();
        let menu_id = id.clone();
        view! {
            <article class="memo-card" data-testid="memo-card" data-memo-id=id.clone()
                class:pinned=pinned class:expanded=move || is_expanded.get()>
                <div class="memo-card-head">
                    {pinned.then(|| view! { <span class="memo-pin" title=move || t(locale.get(), "memos.pinned")>{compose_icon("pin")}</span> })}
                    <button type="button" class="memo-heading"
                        aria-expanded=move || is_expanded.get().to_string()
                        title=move || if is_expanded.get() { t(locale.get(), "memos.collapse") } else { t(locale.get(), "memos.expand") }
                        on:click=toggle_expanded.clone()
                        on:dblclick={let edit_memo = edit_memo.clone(); move |_| start_edit(edit_memo.clone())}>
                        {match heading {
                            Some(text) => view! { <span class="memo-title">{text}</span> }.into_view(),
                            None => view! { <span class="memo-title untitled">{move || t(locale.get(), "memos.untitled")}</span> }.into_view(),
                        }}
                    </button>
                    <div class="memo-actions">
                        <button type="button" class="memo-insert" data-testid="memo-insert"
                            class:done=move || just_inserted.get()
                            title=move || t(locale.get(), "memos.insert")
                            aria-label=move || t(locale.get(), "memos.insert")
                            on:click=move |_| insert(insert_memo.clone())>
                            {move || if just_inserted.get() { compose_icon("check") } else { compose_icon("arrow-left") }}
                            <span>{move || if just_inserted.get() { t(locale.get(), "memos.inserted") } else { t(locale.get(), "memos.insert") }}</span>
                        </button>
                        <button type="button" class="icon-btn memo-edit" data-testid="memo-edit"
                            title=move || t(locale.get(), "memos.edit")
                            aria-label=move || t(locale.get(), "memos.edit")
                            on:click=move |_| start_edit(edit_memo.clone())>{compose_icon("square-pen")}</button>
                        <div class="memo-more-wrap">
                            <button type="button" class="icon-btn memo-more" data-testid="memo-more"
                                title=move || t(locale.get(), "memos.more")
                                aria-label=move || t(locale.get(), "memos.more")
                                aria-haspopup="menu"
                                aria-expanded=move || menu_open.get().to_string()
                                on:click=move |_| menu.update(|current| {
                                    if current.as_deref() == Some(menu_id.as_str()) {
                                        *current = None;
                                    } else {
                                        *current = Some(menu_id.clone());
                                    }
                                })>{compose_icon("more")}</button>
                            {move || menu_open.get().then(|| {
                                let pin_memo = pin_memo.clone();
                                let copy_body = copy_body.clone();
                                let delete_memo = delete_memo.clone();
                                view! {
                                    <button type="button" class="memo-menu-backdrop" aria-hidden="true" tabindex="-1"
                                        on:click=move |_| menu.set(None)></button>
                                    <div class="memo-menu" role="menu" data-testid="memo-menu">
                                        <button type="button" role="menuitem" class="memo-menu-item"
                                            on:click=move |_| toggle_pin(pin_memo.clone())>
                                            {compose_icon("pin")}
                                            <span>{move || t(locale.get(), if pinned { "memos.unpin" } else { "memos.pin" })}</span>
                                        </button>
                                        <button type="button" role="menuitem" class="memo-menu-item"
                                            on:click=move |_| { menu.set(None); copy_text(copy_body.clone()); }>
                                            {compose_icon("copy")}
                                            <span>{move || t(locale.get(), "memos.copy")}</span>
                                        </button>
                                        <button type="button" role="menuitem" class="memo-menu-item danger"
                                            on:click=move |_| delete(delete_memo.clone())>
                                            {compose_icon("trash")}
                                            <span>{move || t(locale.get(), "memos.delete")}</span>
                                        </button>
                                    </div>
                                }
                            })}
                        </div>
                    </div>
                </div>
                <pre class="memo-body" class:clamped=move || !is_expanded.get()
                    on:click=toggle_expanded
                    on:dblclick=move |_| start_edit(memo.clone())>{body}</pre>
                <div class="memo-meta">{move || format_relative_time(updated_at, locale.get())}</div>
            </article>
        }
    };

    view! {
        <section class="memos-pane" data-testid="memos-pane" aria-label=move || t(locale.get(), "right.memos")>
            <div class="memos-head">
                <span class="memos-private" data-testid="memos-private">
                    {compose_icon("lock")}
                    <span>{move || t(locale.get(), "memos.private")}</span>
                </span>
                <button type="button" class="memo-btn primary memos-new" data-testid="memos-new"
                    disabled=move || open_draft.get().is_some() || project_id.get().is_none()
                    on:click=move |_| start_new()>
                    {compose_icon("plus")}
                    <span>{move || t(locale.get(), "memos.new")}</span>
                </button>
            </div>
            {move || (!memos.get().is_empty()).then(|| view! {
                <label class="memos-search">
                    {compose_icon("search")}
                    <input type="search" data-testid="memos-search"
                        placeholder=move || t(locale.get(), "memos.search")
                        aria-label=move || t(locale.get(), "memos.search")
                        prop:value=move || query.get()
                        on:input=move |ev| query.set(event_target_value(&ev)) />
                </label>
            })}
            {move || error.get().map(|message| view! {
                <div class="memo-error" role="alert">{move || t(locale.get(), "memos.load_error")}" "{message}</div>
            })}
            <div class="memos-list">
                {move || open_draft.get().filter(|(_, id)| id.is_none()).and_then(|_| editor())}
                {move || {
                    let all = memos.get();
                    let shown = filter_memos(&all, &query.get());
                    let open = open_draft.get();
                    let editing = open.clone().and_then(|(_, id)| id);
                    if all.is_empty() {
                        if loading.get() || open.is_some() {
                            return ().into_view();
                        }
                        return view! {
                            <div class="memos-empty" data-testid="memos-empty">
                                <span class="memos-empty-icon">{compose_icon("notebook")}</span>
                                <p>{move || t(locale.get(), "memos.empty")}</p>
                            </div>
                        }.into_view();
                    }
                    if shown.is_empty() {
                        return view! { <div class="memos-empty muted">{move || t(locale.get(), "memos.no_match")}</div> }.into_view();
                    }
                    shown.into_iter().map(|memo| {
                        if editing.as_deref() == Some(memo.id.as_str()) {
                            editor().into_view()
                        } else {
                            card(memo).into_view()
                        }
                    }).collect_view()
                }}
            </div>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memo(id: &str, title: &str, body: &str, pinned: bool, updated_at: i64) -> ProjectMemo {
        ProjectMemo {
            id: id.into(),
            project_id: "p".into(),
            title: title.into(),
            body: body.into(),
            pinned,
            created_at: updated_at,
            updated_at,
        }
    }

    #[test]
    fn insert_fills_an_empty_composer_and_appends_after_a_blank_line() {
        assert_eq!(compose_insert("", "ssh gpu02"), "ssh gpu02");
        assert_eq!(compose_insert("   \n", "ssh gpu02"), "ssh gpu02");
        assert_eq!(
            compose_insert("Compare methods\n", "ssh gpu02"),
            "Compare methods\n\nssh gpu02"
        );
    }

    #[test]
    fn heading_prefers_the_title_then_the_first_body_line() {
        assert_eq!(
            memo_heading(&memo("a", " Server ", "body", false, 1)),
            Some("Server".into())
        );
        assert_eq!(
            memo_heading(&memo("a", "", "\n\n  first line \nsecond", false, 1)),
            Some("first line".into())
        );
        assert_eq!(memo_heading(&memo("a", "", "   ", false, 1)), None);
    }

    #[test]
    fn filter_matches_title_or_body_case_insensitively() {
        let memos = vec![
            memo("a", "Server habits", "ssh gpu02", false, 1),
            memo("b", "", "Opening PROMPT", false, 2),
        ];
        assert_eq!(filter_memos(&memos, "").len(), 2);
        assert_eq!(filter_memos(&memos, "GPU")[0].id, "a");
        assert_eq!(filter_memos(&memos, "prompt")[0].id, "b");
        assert!(filter_memos(&memos, "nothing").is_empty());
    }

    #[test]
    fn sort_keeps_pinned_first_then_newest() {
        let mut memos = vec![
            memo("old", "", "x", false, 1),
            memo("new", "", "x", false, 3),
            memo("pinned", "", "x", true, 2),
        ];
        sort_memos(&mut memos);
        assert_eq!(
            memos.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["pinned", "new", "old"]
        );
    }

    #[test]
    fn drafts_know_when_they_changed_and_when_they_can_save() {
        let existing = memo("a", "Title", "Body", false, 1);
        let mut draft = MemoDraft::edit("p".into(), &existing);
        assert!(!draft.dirty());
        assert!(draft.can_save());
        draft.body = "Body changed".into();
        assert!(draft.dirty());
        let mut fresh = MemoDraft::new("p".into());
        assert!(!fresh.dirty() && !fresh.can_save());
        fresh.body = "   ".into();
        assert!(!fresh.can_save());
        fresh.body = "note".into();
        assert!(fresh.can_save() && fresh.dirty());
        assert_eq!(fresh.editor_rows(), EDITOR_MIN_ROWS);
        fresh.body = "l\n".repeat(40);
        assert_eq!(fresh.editor_rows(), EDITOR_MAX_ROWS);
    }
}
