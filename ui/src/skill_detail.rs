use crate::app_support::{compose_icon, join_tags, js_error_text};
use crate::bindings::invoke_checked;
use crate::dto::{SkillFileContent, SkillRow};
use crate::i18n::{t, Locale};
use crate::settings_view::DeleteConfirm;
use crate::text::{event_target_checked, event_target_value};
use leptos::*;
use serde_wasm_bindgen::to_value;

fn markdown_preview(source: &str) -> String {
    use pulldown_cmark::{html, Event, Options, Parser, Tag, TagEnd};
    let normalized = source.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let body = normalized
        .strip_prefix("---\n")
        .and_then(|s| s.split_once("\n---\n").map(|(_, body)| body))
        .unwrap_or(&normalized);
    // Packages can come from outside Wisp. Escape raw HTML, disable active URLs,
    // and render image alt text without loading remote package resources.
    let events = Parser::new_ext(body, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).map(
        |event| match event {
            Event::Html(text) | Event::InlineHtml(text) => Event::Text(text),
            Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => Event::Text("".into()),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let safe = dest_url.starts_with("https://")
                    || dest_url.starts_with("http://")
                    || dest_url.starts_with('#');
                Event::Start(Tag::Link {
                    link_type,
                    dest_url: if safe { dest_url } else { "#".into() },
                    title,
                    id,
                })
            }
            other => other,
        },
    );
    let mut html = String::new();
    html::push_html(&mut html, events);
    html
}

#[component]
pub(crate) fn SkillDetail(
    name: String,
    skills: RwSignal<Vec<SkillRow>>,
    locale: RwSignal<Locale>,
    refresh: Callback<()>,
    save_tags: Callback<(String, String)>,
    delete_confirm: RwSignal<Option<DeleteConfirm>>,
) -> impl IntoView {
    let name = store_value(name);
    let skill = create_memo(move |_| {
        skills
            .get()
            .into_iter()
            .find(|s| s.name == name.get_value())
    });
    let files = create_rw_signal(Vec::<String>::new());
    let selected = create_rw_signal(String::new());
    let content = create_rw_signal(None::<SkillFileContent>);
    let error = create_rw_signal(None::<String>);
    let loading = create_rw_signal(true);
    let source_mode = create_rw_signal(false);
    let retry = create_rw_signal(0u32);
    let generation = create_rw_signal(0u32);
    let disposed = std::rc::Rc::new(std::cell::Cell::new(false));
    let disposed_cleanup = disposed.clone();
    on_cleanup(move || disposed_cleanup.set(true));
    let disposed_list = disposed.clone();
    create_effect(move |_| {
        retry.get();
        loading.set(true);
        error.set(None);
        let disposed = disposed_list.clone();
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "name": name.get_value() })).unwrap();
            let result = invoke_checked("list_skill_files", args)
                .await
                .map_err(js_error_text)
                .and_then(|v| {
                    serde_wasm_bindgen::from_value::<Vec<String>>(v).map_err(|e| e.to_string())
                });
            if disposed.get() {
                return;
            }
            match result {
                Ok(paths) => {
                    let first = paths
                        .iter()
                        .find(|p| p.as_str() == "SKILL.md")
                        .or(paths.first())
                        .cloned()
                        .unwrap_or_default();
                    files.set(paths);
                    selected.set(first);
                    if selected.get_untracked().is_empty() {
                        loading.set(false);
                    }
                }
                Err(e) => {
                    loading.set(false);
                    error.set(Some(e));
                }
            }
        });
    });
    create_effect(move |_| {
        let path = selected.get();
        generation.update(|v| *v += 1);
        let request = generation.get_untracked();
        content.set(None);
        if path.is_empty() {
            return;
        }
        loading.set(true);
        error.set(None);
        let disposed = disposed.clone();
        spawn_local(async move {
            let args =
                to_value(&serde_json::json!({ "name": name.get_value(), "path": path })).unwrap();
            let result = invoke_checked("read_skill_file", args)
                .await
                .map_err(js_error_text)
                .and_then(|v| {
                    serde_wasm_bindgen::from_value::<SkillFileContent>(v).map_err(|e| e.to_string())
                });
            if disposed.get() || generation.get_untracked() != request {
                return;
            }
            loading.set(false);
            match result {
                Ok(file) => content.set(Some(file)),
                Err(e) => error.set(Some(e)),
            }
        });
    });
    view! {
        <div class="settings-pane skill-detail" data-testid="skill-detail">
            {move || skill.get().map(|s| {
                let toggle_name = s.name.clone();
                let tags_name = s.name.clone();
                let remove_name = s.name.clone();
                view! {
                    <div class="skill-detail-heading">
                        <h3>{s.name.clone()}</h3>
                        <span class="skill-scope-badge">{t(locale.get(), &format!("skills.scope.{}", s.scope))}</span>
                        <div class="skill-detail-actions">
                            {(!s.managed).then(|| view! {
                                <label class="toggle">
                                    <input type="checkbox" aria-label=t(locale.get(), "skills.enabled") prop:checked=s.enabled
                                        on:change=move |ev| {
                                            let name = toggle_name.clone();
                                            let enabled = event_target_checked(&ev);
                                            spawn_local(async move {
                                                let args = to_value(&serde_json::json!({ "name": name, "enabled": enabled })).unwrap();
                                                match invoke_checked("set_skill_enabled", args).await {
                                                    Ok(_) => refresh.call(()),
                                                    Err(e) => { error.set(Some(js_error_text(e))); refresh.call(()); }
                                                }
                                            });
                                        } />
                                    <span class="toggle-track" aria-hidden="true"></span>
                                </label>
                            })}
                            {(s.scope == "global" && !s.builtin && !s.managed).then(|| view! {
                                <button type="button" class="settings-skill-remove" on:click=move |_| delete_confirm.set(Some(DeleteConfirm::Skill {
                                    name: remove_name.clone(), label: remove_name.clone(),
                                }))>{compose_icon("trash")}{move || t(locale.get(), "skills.remove")}</button>
                            })}
                        </div>
                    </div>
                    {s.managed_by.map(|provider| view! { <p class="skill-managed-badge">{crate::i18n::tf(locale.get(), "skills.managed_by", &[("plugin", &provider)])}</p> })}
                    <p class="skill-detail-description">{s.description}</p>
                    <p class="skill-detail-path">{s.dir}</p>
                    <crate::skill_store::SkillOrigin name=s.name.clone() locale=locale />
                    <label class="skill-detail-tags">
                        <span>{move || t(locale.get(), "skills.edit_tags")}</span>
                        <input class="skill-tags-input" prop:value=join_tags(&s.tags)
                            placeholder=move || t(locale.get(), "skills.tags_placeholder")
                            on:change=move |ev| save_tags.call((tags_name.clone(), event_target_value(&ev))) />
                    </label>
                }
            })}
            <div class="skill-files-heading">
                <h4>{move || t(locale.get(), "skills.files")} <small>{move || files.get().len()}</small></h4>
                <label>
                    <span class="sr-only">{move || t(locale.get(), "skills.select_file")}</span>
                    <select aria-label=move || t(locale.get(), "skills.select_file") prop:value=move || selected.get()
                        on:change=move |ev| selected.set(event_target_value(&ev))>
                        <For each=move || files.get() key=|path| path.clone() let:path>
                            <option value=path.clone()>{path}</option>
                        </For>
                    </select>
                </label>
            </div>
            <div class="skill-file-toolbar">
                <span>{move || selected.get()}</span>
                {move || selected.get().to_ascii_lowercase().ends_with(".md").then(|| view! {
                    <button type="button" class:active=move || !source_mode.get() on:click=move |_| source_mode.set(false)>{move || t(locale.get(), "skills.preview")}</button>
                    <button type="button" class:active=move || source_mode.get() on:click=move |_| source_mode.set(true)>{move || t(locale.get(), "skills.source")}</button>
                })}
            </div>
            {move || loading.get().then(|| view! {
                <p class="skill-file-loading" role="status">
                    <span class="skills-loading-icon" aria-hidden="true">{compose_icon("loader")}</span>
                    {move || t(locale.get(), "skills.loading")}
                </p>
            })}
            {move || error.get().map(|message| view! {
                <div class="settings-status fail" role="alert">{message}
                    <button type="button" on:click=move |_| { selected.set(String::new()); retry.update(|v| *v += 1); }>{move || t(locale.get(), "skills.retry")}</button>
                </div>
            })}
            {move || (!loading.get() && error.get().is_none() && files.get().is_empty()).then(|| view! { <p>{move || t(locale.get(), "skills.no_files")}</p> })}
            {move || content.get().map(|file| {
                if file.path.to_ascii_lowercase().ends_with(".md") && !source_mode.get() {
                    view! { <article class="skill-file-preview md" data-testid="skill-file-preview" inner_html=markdown_preview(&file.content)></article> }.into_view()
                } else {
                    view! { <pre class="skill-file-source" data-testid="skill-file-source"><code>{file.content}</code></pre> }.into_view()
                }
            })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_markdown_escapes_active_content_and_hides_frontmatter() {
        let html = markdown_preview("---\nname: hidden\n---\n# Demo\n<script>alert(1)</script>\n\n[bad](javascript:alert) ![plot](https://example.com/track.png)");
        assert!(html.contains("<h1>Demo</h1>"));
        assert!(!html.contains("name: hidden"));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("javascript:"));
        assert!(!html.contains("<img"));
    }
}
