use crate::app_support::{compose_icon, join_tags, js_error_text};
use crate::bindings::invoke_checked;
use crate::dto::{
    SkillCopy, SkillFileContent, SkillInstallLocations, SkillInstallReceipt, SkillRow,
};
use crate::i18n::{localize_backend, t, tf, Locale};
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

/// Install/promotion receipt: scope, absolute path, who can discover it, and
/// the copy that still wins when the new one is shadowed.
pub(crate) fn skill_install_receipt_text(locale: Locale, receipt: &SkillInstallReceipt) -> String {
    let reach = if receipt.scope == "project" {
        "skills.receipt.project_only"
    } else {
        "skills.receipt.all_projects"
    };
    let mut text = tf(
        locale,
        "skills.receipt",
        &[
            ("skill", &receipt.name),
            (
                "scope",
                &t(locale, &format!("skills.scope.{}", receipt.scope)),
            ),
            ("path", &receipt.dir),
            ("reach", &t(locale, reach)),
        ],
    );
    if let Some(winner) = &receipt.shadowed_by {
        text.push(' ');
        text.push_str(&tf(
            locale,
            "skills.receipt.shadowed",
            &[
                (
                    "scope",
                    &t(locale, &format!("skills.scope.{}", winner.scope)),
                ),
                ("path", &winner.dir),
            ],
        ));
    }
    text
}

/// What the scope section may offer for one Skill's discovered copies.
#[derive(Debug, PartialEq)]
struct ScopeActions {
    can_promote: bool,
    can_remove_project_copy: bool,
    /// The removable project copy duplicates another copy rather than having
    /// been promoted.
    duplicate: bool,
}

fn scope_actions(copies: &[SkillCopy]) -> ScopeActions {
    let project = copies.iter().find(|copy| copy.scope == "project");
    let has_global = copies.iter().any(|copy| copy.scope == "global");
    // Mirrors `removable_project_copy` in the backend: another copy must keep
    // the name available (a global copy, or identical content).
    let duplicate = project.is_some_and(|project| {
        project.same_as_effective
            && copies
                .iter()
                .any(|copy| copy.scope != "project" && copy.same_as_effective)
    });
    ScopeActions {
        can_promote: project.is_some() && !has_global,
        can_remove_project_copy: project.is_some() && (has_global || duplicate),
        duplicate: duplicate && !has_global,
    }
}

#[component]
pub(crate) fn SkillDetail(
    name: String,
    skills: RwSignal<Vec<SkillRow>>,
    locale: RwSignal<Locale>,
    refresh: Callback<()>,
    save_tags: Callback<(String, String)>,
    delete_confirm: RwSignal<Option<DeleteConfirm>>,
    install_locations: RwSignal<Option<SkillInstallLocations>>,
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
    let promoting = create_rw_signal(false);
    let scope_msg = create_rw_signal(None::<(bool, String)>);
    let promote = move |_| {
        if promoting.get_untracked() {
            return;
        }
        promoting.set(true);
        scope_msg.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "name": name.get_value() })).unwrap();
            let result = invoke_checked("promote_skill_to_global", args)
                .await
                .map_err(js_error_text)
                .and_then(|v| {
                    serde_wasm_bindgen::from_value::<SkillInstallReceipt>(v)
                        .map_err(|e| e.to_string())
                });
            promoting.set(false);
            match result {
                Ok(receipt) => {
                    scope_msg.set(Some((
                        true,
                        skill_install_receipt_text(locale.get_untracked(), &receipt),
                    )));
                    refresh.call(());
                }
                Err(e) => {
                    scope_msg.set(Some((false, localize_backend(locale.get_untracked(), &e))))
                }
            }
        });
    };
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
                let copy_name = s.name.clone();
                let actions = scope_actions(&s.copies);
                let lifetime_key = match s.scope.as_str() {
                    "project" => Some("skills.lifetime.project"),
                    "global" => Some("skills.lifetime.global"),
                    "bundled" => Some("skills.lifetime.bundled"),
                    _ => None,
                };
                let copies = s.copies.clone();
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
                    <section class="skill-scope-section" data-testid="skill-scope">
                        {lifetime_key.map(|key| view! {
                            <p class="skill-lifetime" class:project=key == "skills.lifetime.project">{move || t(locale.get(), key)}</p>
                        })}
                        {(copies.len() > 1).then(|| view! {
                            <h4>{move || t(locale.get(), "skills.copies")}</h4>
                            <p class="skill-copies-hint">{move || t(locale.get(), "skills.copies_hint")}</p>
                            <ul class="skill-copies" data-testid="skill-copies">
                                {copies.iter().cloned().map(|copy| {
                                    let status = if copy.effective { "skills.copy.active" } else { "skills.copy.shadowed" };
                                    let identical = !copy.effective && copy.same_as_effective;
                                    let project = copy.scope == "project";
                                    view! {
                                        <li class="skill-copy" class:active=copy.effective data-scope=copy.scope.clone()>
                                            <span class="skill-scope-badge">{t(locale.get(), &format!("skills.scope.{}", copy.scope))}</span>
                                            <span class="skill-copy-status">{move || t(locale.get(), status)}</span>
                                            {identical.then(|| view! { <span class="skill-copy-identical">{move || t(locale.get(), "skills.copy.identical")}</span> })}
                                            <span class="skill-copy-path">{copy.dir}</span>
                                            {project.then(|| view! { <small class="skill-copy-lifetime">{move || t(locale.get(), "skills.lifetime.project")}</small> })}
                                        </li>
                                    }
                                }).collect_view()}
                            </ul>
                        })}
                        {actions.can_promote.then(|| view! {
                            <div class="skill-scope-action">
                                <button type="button" class="skill-promote" disabled=move || promoting.get() on:click=promote>
                                    {compose_icon("upload")}
                                    {move || t(locale.get(), if promoting.get() { "skills.promoting" } else { "skills.promote" })}
                                </button>
                                <small>{move || install_locations.get().map(|locations| tf(locale.get(), "skills.promote_hint", &[("path", &locations.global_dir)]))}</small>
                            </div>
                        })}
                        {actions.can_remove_project_copy.then(|| view! {
                            <div class="skill-scope-action">
                                <button type="button" class="skill-remove-project-copy" on:click=move |_| delete_confirm.set(Some(DeleteConfirm::ProjectSkillCopy {
                                    name: copy_name.clone(), label: copy_name.clone(),
                                }))>{compose_icon("trash")}{move || t(locale.get(), "skills.remove_project_copy")}</button>
                                <small>{move || t(locale.get(), if actions.duplicate { "skills.duplicate_hint" } else { "skills.promoted_remove_hint" })}</small>
                            </div>
                        })}
                        {move || scope_msg.get().map(|(ok, text)| view! {
                            <div class="settings-status" class:ok=ok class:fail=move || !ok role="status">{text}</div>
                        })}
                    </section>
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

    fn copy(scope: &str, effective: bool, same: bool) -> SkillCopy {
        SkillCopy {
            scope: scope.into(),
            dir: format!("/{scope}/skill"),
            effective,
            same_as_effective: same,
        }
    }

    #[test]
    fn scope_actions_offer_promotion_then_removal_of_the_project_copy() {
        let only_project = [copy("project", true, true)];
        assert_eq!(
            scope_actions(&only_project),
            ScopeActions {
                can_promote: true,
                can_remove_project_copy: false,
                duplicate: false
            }
        );
        let promoted = [copy("project", true, true), copy("global", false, true)];
        assert_eq!(
            scope_actions(&promoted),
            ScopeActions {
                can_promote: false,
                can_remove_project_copy: true,
                duplicate: false
            }
        );
        let identical_to_bundled = [copy("bundled", true, true), copy("project", false, true)];
        assert_eq!(
            scope_actions(&identical_to_bundled),
            ScopeActions {
                can_promote: true,
                can_remove_project_copy: true,
                duplicate: true
            }
        );
        let edited_project = [copy("bundled", true, true), copy("project", false, false)];
        assert_eq!(
            scope_actions(&edited_project),
            ScopeActions {
                can_promote: true,
                can_remove_project_copy: false,
                duplicate: false
            }
        );
        assert_eq!(
            scope_actions(&[copy("global", true, true)]),
            ScopeActions {
                can_promote: false,
                can_remove_project_copy: false,
                duplicate: false
            }
        );
    }

    #[test]
    fn install_receipt_states_scope_path_reach_and_shadowing_copy() {
        let receipt = SkillInstallReceipt {
            name: "notes".into(),
            scope: "global".into(),
            dir: "/home/me/.wisp/skills/notes".into(),
            shadowed_by: Some(copy("project", true, true)),
        };
        let text = skill_install_receipt_text(Locale::En, &receipt);
        assert!(text.contains("notes as a Global skill at /home/me/.wisp/skills/notes"));
        assert!(text.contains("Every project can discover it."));
        assert!(text.contains("the Project copy at /project/skill still takes precedence"));

        let project = SkillInstallReceipt {
            scope: "project".into(),
            dir: "/work/.wisp/skills/notes".into(),
            shadowed_by: None,
            ..receipt
        };
        let text = skill_install_receipt_text(Locale::En, &project);
        assert!(text.contains("Only this project can discover it."));
        assert!(!text.contains("precedence"));
    }
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
