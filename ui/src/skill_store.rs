use crate::app_support::{compose_icon, js_error_text};
use crate::bindings::{invoke_checked, open_external_url};
use crate::dto::*;
use crate::i18n::{t, Locale};
use crate::text::event_target_value;
use leptos::*;
use serde_wasm_bindgen::{from_value, to_value};

pub(crate) const GUIDE: &str =
    "https://github.com/xuzhougeng/wisp-science/blob/main/docs/skill-authoring.md";

fn github_path(value: &str) -> String {
    value
        .split('/')
        .map(|part| String::from(js_sys::encode_uri_component(part)))
        .collect::<Vec<_>>()
        .join("/")
}

#[component]
fn SourceDetails(source: SkillInstallSource) -> impl IntoView {
    let url = format!(
        "https://github.com/{}/tree/{}/{}",
        source.repository,
        source.commit,
        github_path(&source.package_path)
    );
    let original_url = source.source_url.clone();
    let skill_url = format!(
        "https://github.com/{}/blob/{}/{}SKILL.md",
        source.repository,
        source.commit,
        if source.package_path.is_empty() {
            String::new()
        } else {
            format!("{}/", github_path(&source.package_path))
        }
    );
    view! {
        <dl class="skill-store-source">
            <dt>"GitHub"</dt><dd><button class="link-button" on:click=move |_| open_external_url(url.clone())>{source.repository}</button></dd>
            <dt>"Source URL"</dt><dd><button class="link-button" on:click=move |_| open_external_url(original_url.clone())>{source.source_url}</button></dd>
            <dt>"SKILL.md"</dt><dd><button class="link-button" on:click=move |_| open_external_url(skill_url.clone())>"SKILL.md"</button></dd>
            <dt>"Ref"</dt><dd>{source.git_ref}</dd>
            <dt>"Commit"</dt><dd><code>{source.commit}</code></dd>
            <dt>"Package"</dt><dd>{if source.package_path.is_empty() { "/".into() } else { source.package_path }}</dd>
        </dl>
    }
}

#[component]
pub(crate) fn SkillStore(
    locale: RwSignal<Locale>,
    skills: RwSignal<Vec<SkillRow>>,
    close: Callback<()>,
    refresh_skills: Callback<()>,
    github_first: bool,
    external_link_confirm: RwSignal<Option<String>>,
) -> impl IntoView {
    let catalog = create_rw_signal(Vec::<CommunitySkillEntry>::new());
    let catalog_notice = create_rw_signal(None::<String>);
    let catalog_loading = create_rw_signal(false);
    let query = create_rw_signal(String::new());
    let tag = create_rw_signal(String::new());
    let github = create_rw_signal(github_first);
    let source_url = create_rw_signal(String::new());
    let exact_ref = create_rw_signal(String::new());
    let selected_entry = create_rw_signal(None::<CommunitySkillEntry>);
    let candidates = create_rw_signal(Vec::<SkillStoreCandidate>::new());
    let selected = create_rw_signal(None::<usize>);
    let confirming = create_rw_signal(false);
    let loading = create_rw_signal(false);
    let installing = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);
    let success = create_rw_signal(None::<String>);
    let generation = create_rw_signal(0u64);

    let load_catalog = Callback::new(move |refresh: bool| {
        catalog_loading.set(true);
        spawn_local(async move {
            let result = invoke_checked(
                "list_community_skills",
                to_value(&serde_json::json!({"refresh": refresh})).unwrap(),
            )
            .await
            .map_err(js_error_text)
            .and_then(|v| from_value::<CommunitySkillCatalog>(v).map_err(|e| e.to_string()));
            if catalog_loading.try_get_untracked().is_none() {
                return;
            }
            catalog_loading.set(false);
            match result {
                Ok(value) => {
                    catalog.set(value.entries);
                    catalog_notice.set(value.notice);
                }
                Err(e) => catalog_notice.set(Some(e)),
            }
        });
    });
    load_catalog.call(false);

    let preview = Callback::new(move |_| {
        generation.update(|v| *v += 1);
        let request = generation.get_untracked();
        let url = source_url.get_untracked();
        let git_ref = exact_ref.get_untracked();
        loading.set(true);
        error.set(None);
        success.set(None);
        candidates.set(vec![]);
        selected.set(None);
        spawn_local(async move {
            let result = invoke_checked(
                "preview_github_skills",
                to_value(&serde_json::json!({"sourceUrl": url, "exactRef": git_ref})).unwrap(),
            )
            .await
            .map_err(js_error_text)
            .and_then(|v| from_value::<Vec<SkillStoreCandidate>>(v).map_err(|e| e.to_string()));
            if generation.try_get_untracked() != Some(request) {
                return;
            }
            loading.set(false);
            match result {
                Ok(value) => {
                    candidates.set(value);
                }
                Err(e) => error.set(Some(e)),
            }
        });
    });
    let cancel_preview = Callback::new(move |_| {
        generation.update(|v| *v += 1);
        loading.set(false);
        error.set(None);
        candidates.set(vec![]);
        selected.set(None);
    });
    let back = Callback::new(move |_| {
        if installing.get_untracked() {
            return;
        }
        if confirming.get_untracked() {
            confirming.set(false);
        } else if selected.get_untracked().is_some() {
            selected.set(None);
        } else if loading.get_untracked() {
            cancel_preview.call(());
        } else {
            close.call(());
        }
    });
    crate::window_capture_escape(move || {
        if external_link_confirm.get_untracked().is_some() {
            return false;
        }
        back.call(());
        true
    });

    let install = Callback::new(move |_| {
        if installing.get_untracked() {
            return;
        }
        let Some(candidate) = selected
            .get_untracked()
            .and_then(|i| candidates.get_untracked().get(i).cloned())
        else {
            return;
        };
        let Some(index) = selected.get_untracked() else {
            return;
        };
        installing.set(true);
        error.set(None);
        spawn_local(async move {
            let result = invoke_checked(
                "install_github_skill",
                to_value(&serde_json::json!({"source": candidate.source})).unwrap(),
            )
            .await
            .map_err(js_error_text)
            .and_then(|v| from_value::<SkillInstallResult>(v).map_err(|e| e.to_string()));
            if installing.try_get_untracked().is_none() {
                return;
            }
            installing.set(false);
            match result {
                Ok(result) => {
                    confirming.set(false);
                    success.set(Some(format!(
                        "{}: {}{}",
                        t(locale.get_untracked(), "store.installed"),
                        result.directory,
                        result.notice.map(|s| format!("\n{s}")).unwrap_or_default()
                    )));
                    candidates.update(|all| {
                        if let Some(value) = all.get_mut(index) {
                            value.conflict =
                                Some(t(locale.get_untracked(), "store.already_installed"));
                            value.installed_source = Some(value.source.clone());
                        }
                    });
                    refresh_skills.call(());
                }
                Err(e) => error.set(Some(e)),
            }
        });
    });
    let visible = create_memo(move |_| {
        let q = query.get().trim().to_lowercase();
        let filter = tag.get();
        catalog
            .get()
            .into_iter()
            .filter(|entry| {
                (filter.is_empty() || entry.tags.contains(&filter))
                    && (q.is_empty()
                        || format!(
                            "{} {} {} {} {}",
                            entry.name,
                            entry.description,
                            entry.author,
                            entry.repository,
                            entry.tags.join(" ")
                        )
                        .to_lowercase()
                        .contains(&q))
            })
            .collect::<Vec<_>>()
    });
    view! {
        <div class="settings-pane skill-store" data-testid="skill-store">
            <div class="settings-toolbar">
                <button disabled=move || installing.get() on:click=move |_| back.call(())>{compose_icon("chevron-left")}{move || t(locale.get(), "store.back")}</button>
                <h3>{move || t(locale.get(), "store.title")}</h3>
                <button on:click=move |_| open_external_url(GUIDE.into())>{move || t(locale.get(), "store.guide")}</button>
            </div>
            <p class="settings-note">{move || t(locale.get(), "store.scope")}</p>
            {move || selected.get().is_none().then(|| view! {
                <div class="skill-tags-filter">
                    <button class:active=move || !github.get() disabled=move || loading.get() on:click=move |_| { github.set(false); error.set(None); candidates.set(vec![]); selected_entry.set(None); }>{move || t(locale.get(), "store.browse")}</button>
                    <button class:active=move || github.get() disabled=move || loading.get() on:click=move |_| { github.set(true); selected_entry.set(None); candidates.set(vec![]); error.set(None); }>{move || t(locale.get(), "store.github")}</button>
                </div>
                {move || (!github.get()).then(|| view! {
                    <p class="settings-note">{move || t(locale.get(), "store.community_notice")}</p>
                    <div class="settings-toolbar">
                        <input aria-label=move || t(locale.get(), "store.search") placeholder=move || t(locale.get(), "store.search") prop:value=move || query.get() on:input=move |ev| query.set(event_target_value(&ev)) />
                        <select aria-label=move || t(locale.get(), "store.tags") prop:value=move || tag.get() on:change=move |ev| tag.set(event_target_value(&ev))>
                            <option value="">{move || t(locale.get(), "store.all_tags")}</option>
                            {move || catalog.get().iter().flat_map(|e| e.tags.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().map(|tag| view! { <option value=tag.clone()>{tag}</option> }).collect_view()}
                        </select>
                        <button disabled=move || catalog_loading.get() on:click=move |_| load_catalog.call(true)>{move || t(locale.get(), "store.refresh")}</button>
                    </div>
                    {move || catalog_notice.get().map(|notice| view! { <p role="status">{notice}</p> })}
                    <p class="settings-note">{move || format!("{} / {} {}", visible.get().len(), catalog.get().len(), t(locale.get(), "store.directory_entries"))}</p>
                    {move || visible.get().is_empty().then(|| view! { <p>{move || t(locale.get(), "store.empty")}</p> })}
                    <div class="skill-store-grid">
                        {move || visible.get().into_iter().map(|entry| {
                            let name = entry.name.clone();
                            let to_preview = entry.clone();
                            view! {
                                <article class="skill-store-card">
                                    <h4>{entry.name}</h4><p>{entry.description}</p>
                                    <p class="hint">{entry.author} " · " {entry.license}</p>
                                    <p class="hint">{entry.repository} " @ " {entry.git_ref}</p>
                                    <div class="skill-row-tags">{entry.tags.into_iter().map(|tag| view! { <span class="skill-tag">{tag}</span> }).collect_view()}</div>
                                    <span class="skill-scope-badge">{move || t(locale.get(), "store.community")}</span>
                                    <p>{move || if skills.get().iter().any(|s| s.name.eq_ignore_ascii_case(&name)) { t(locale.get(), "store.name_present") } else { t(locale.get(), "store.not_installed") }}</p>
                                    <button disabled=move || loading.get() on:click=move |_| {
                                        let entry = to_preview.clone();
                                        let path = github_path(format!("{}/{}", entry.git_ref, entry.package_path).trim_end_matches('/'));
                                        source_url.set(format!("https://github.com/{}/tree/{path}", entry.repository));
                                        exact_ref.set(entry.git_ref.clone()); selected_entry.set(Some(entry)); preview.call(());
                                    }>{move || t(locale.get(), "store.preview")}</button>
                                </article>
                            }
                        }).collect_view()}
                    </div>
                })}
                {move || github.get().then(|| view! {
                    <div class="skill-store-github">
                        <label>{move || t(locale.get(), "store.url")}<input type="url" data-testid="skill-github-url" prop:value=move || source_url.get() disabled=move || loading.get() on:input=move |ev| { source_url.set(event_target_value(&ev)); candidates.set(vec![]); } placeholder="https://github.com/owner/repository/tree/ref/skills/example" /></label>
                        <label>{move || t(locale.get(), "store.exact_ref")}<input data-testid="skill-github-ref" prop:value=move || exact_ref.get() disabled=move || loading.get() on:input=move |ev| { exact_ref.set(event_target_value(&ev)); candidates.set(vec![]); } /></label>
                        <p class="hint">{move || t(locale.get(), "store.ref_help")}</p>
                        <button disabled=move || loading.get() || source_url.get().trim().is_empty() on:click=move |_| { selected_entry.set(None); preview.call(()); }>{move || t(locale.get(), "store.discover")}</button>
                    </div>
                })}
                {move || loading.get().then(|| view! { <p role="status">{move || t(locale.get(), "store.loading")} <button on:click=move |_| cancel_preview.call(())>{move || t(locale.get(), "store.cancel")}</button></p> })}
                <div class="skill-store-candidates">
                    {move || candidates.get().into_iter().enumerate().map(|(i, candidate)| view! {
                        <button class="skill-store-candidate" on:click=move |_| { selected.set(Some(i)); error.set(None); success.set(None); }>
                            <strong>{candidate.name}</strong><span>{candidate.source.package_path}</span>
                            <span>{if candidate.conflict.is_some() { t(locale.get(), "store.conflict") } else if !candidate.format_errors.is_empty() || !candidate.resource_errors.is_empty() { t(locale.get(), "store.invalid") } else { t(locale.get(), "store.format_pass") }}</span>
                            {compose_icon("chevron-right")}
                        </button>
                    }).collect_view()}
                </div>
            })}
            {move || selected.get().and_then(|i| candidates.get().get(i).cloned()).map(|candidate| {
                let blocked = candidate.conflict.is_some() || !candidate.format_errors.is_empty() || !candidate.resource_errors.is_empty();
                let format_valid = candidate.format_errors.is_empty();
                view! {
                    <section class="skill-store-preview" data-testid="skill-store-preview">
                        <h3>{candidate.name.clone()}</h3><p>{candidate.description.clone()}</p>
                        <SourceDetails source=candidate.source.clone() />
                        <p class="skill-scope-badge">{if selected_entry.get().is_some() { t(locale.get(), "store.community") } else { t(locale.get(), "store.user_source") }}</p>
                        {selected_entry.get().map(|entry| {
                            let mismatch = entry.name != candidate.name || entry.description != candidate.description;
                            view! {
                                {mismatch.then(|| view! { <p class="settings-status">{move || t(locale.get(), "store.metadata_drift")}</p> })}
                                <dl class="skill-store-source">
                                    <dt>{move || t(locale.get(), "store.responsibilities")}</dt><dd>{entry.responsibilities}</dd>
                                    <dt>{move || t(locale.get(), "store.trigger")}</dt><dd>{entry.when_to_use}</dd>
                                    <dt>{move || t(locale.get(), "store.inputs")}</dt><dd>{entry.inputs}</dd>
                                    <dt>{move || t(locale.get(), "store.outputs")}</dt><dd>{entry.outputs}</dd>
                                    <dt>{move || t(locale.get(), "store.excludes")}</dt><dd>{entry.out_of_scope}</dd>
                                    <dt>{move || t(locale.get(), "store.required")}</dt><dd>{entry.required_dependencies.join("; ")}</dd>
                                    <dt>{move || t(locale.get(), "store.optional")}</dt><dd>{entry.optional_dependencies.join("; ")}</dd>
                                    <dt>{move || t(locale.get(), "store.boundaries")}</dt><dd>{entry.operation_boundary}</dd>
                                    <dt>{move || t(locale.get(), "store.declared")}</dt><dd>{entry.supported_wisp}</dd>
                                    <dt>{move || t(locale.get(), "store.verified")}</dt><dd>{entry.verified_wisp.unwrap_or_else(|| t(locale.get(), "store.unverified"))}</dd>
                                    <dt>{move || t(locale.get(), "store.limits")}</dt><dd>{entry.known_limits}</dd>
                                    <dt>{move || t(locale.get(), "store.author")}</dt><dd>{entry.author} " · " {entry.license}</dd>
                                </dl>
                                <button on:click=move |_| open_external_url(entry.feedback_url.clone())>{move || t(locale.get(), "store.feedback")}</button>
                            }
                        })}
                        {format_valid.then(|| view! { <p class="settings-status ok">{move || t(locale.get(), "store.format_pass")}</p> })}
                        {candidate.format_errors.into_iter().map(|e| view! { <p class="settings-status fail">{move || t(locale.get(), "store.format_error")} ": " {e}</p> }).collect_view()}
                        {candidate.resource_errors.into_iter().map(|e| view! { <p class="settings-status fail">{move || t(locale.get(), "store.resource_error")} ": " {e}</p> }).collect_view()}
                        {candidate.conflict.map(|reason| view! { <p class="settings-status fail">{move || t(locale.get(), "store.conflict")} ": " {reason}</p> })}
                        {candidate.installed_source.map(|source| view! { <h4>{move || t(locale.get(), "store.installed_source")}</h4><SourceDetails source=source /> })}
                        <p>{move || t(locale.get(), "store.dependencies_pending")}</p>
                        <p>{move || t(locale.get(), "store.unverified")}</p>
                        {candidate.warnings.into_iter().map(|e| view! { <p class="hint">{e}</p> }).collect_view()}
                        <h4>"SKILL.md"</h4><pre class="skill-store-markdown">{candidate.markdown}</pre>
                        <button disabled=blocked on:click=move |_| confirming.set(true)>{move || t(locale.get(), "store.review_install")}</button>
                    </section>
                }
            })}
            {move || success.get().map(|text| view! { <p class="settings-status ok" role="status">{text}</p> })}
            {move || (!confirming.get()).then(|| error.get().map(|text| view! { <p class="settings-status fail" role="alert">{text}</p> }))}
            {move || confirming.get().then(|| view! {
                <div class="overlay skill-store-overlay">
                    <section class="modal skill-store-confirm" role="dialog" aria-modal="true" aria-label=t(locale.get(), "store.confirm_title")>
                        <h3>{move || t(locale.get(), "store.confirm_title")}</h3>
                        {move || selected.get().and_then(|i| candidates.get().get(i).cloned()).map(|c| view! { <h4>{c.name.clone()}</h4><SourceDetails source=c.source /><p><code>{format!("~/.wisp/skills/{}", c.name)}</code></p> })}
                        <p>{move || t(locale.get(), "store.scope")}</p>
                        <p>{move || t(locale.get(), "store.no_execution")}</p>
                        {move || error.get().map(|text| view! { <p class="settings-status fail" role="alert">{text}</p> })}
                        <div class="modal-actions">
                            <button disabled=move || installing.get() on:click=move |_| confirming.set(false)>{move || t(locale.get(), "store.cancel")}</button>
                            <button disabled=move || installing.get() on:click=move |_| install.call(())>{move || t(locale.get(), if installing.get() { "store.installing" } else { "store.confirm_install" })}</button>
                        </div>
                    </section>
                </div>
            })}
        </div>
    }
}

#[component]
pub(crate) fn SkillOrigin(name: String, locale: RwSignal<Locale>) -> impl IntoView {
    let origin = create_rw_signal(None::<SkillInstallSource>);
    spawn_local(async move {
        if let Ok(value) = invoke_checked(
            "get_skill_install_source",
            to_value(&serde_json::json!({"name": name})).unwrap(),
        )
        .await
        {
            if origin.try_get_untracked().is_some() {
                if let Ok(source) = from_value::<Option<SkillInstallSource>>(value) {
                    origin.set(source);
                }
            }
        }
    });
    view! { <div class="skill-origin">{move || origin.get().map(|source| view! { <h4>{move || t(locale.get(), "store.installed_source")}</h4><SourceDetails source=source /> })}</div> }
}
