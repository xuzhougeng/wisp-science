//! Notice that the project's database was found damaged and replaced (#1635).
//!
//! The backend records every automatic recovery in the application database,
//! whichever entry point found the damage (explicit open, project list, sync).
//! Entering a project reads its unacknowledged outcomes and shows them as an
//! inline banner above the composer; closing one acknowledges it, so it is
//! shown again after a restart until then and never afterwards. An inline
//! banner, not an overlay, so it takes no part in the Escape stack.
use crate::app_support::{compose_icon, format_message_datetime};
use crate::dto::{ProjectDatabaseRecovery, ProjectDatabaseRecoveryKind, ProjectInfo};
use crate::i18n::{t, tf, Locale};
use crate::research_journey::call;
use leptos::*;

/// The backup and recovery guide in the user's language.
pub(crate) fn recovery_guide_url(locale: Locale) -> String {
    let (page, section) = if locale == Locale::Zh {
        ("upgrade-backups.zh-CN.md", "项目数据库损坏时自动恢复")
    } else {
        (
            "upgrade-backups.md",
            "a-damaged-project-database-is-restored-by-itself",
        )
    };
    format!("https://github.com/xuzhougeng/wisp-science/blob/main/docs/{page}#{section}")
}

#[component]
pub(crate) fn DatabaseRecoveryNotice(
    locale: RwSignal<Locale>,
    project_info: RwSignal<Option<ProjectInfo>>,
) -> impl IntoView {
    let pending = create_rw_signal(Vec::<ProjectDatabaseRecovery>::new());
    // Changes only when a project is entered or left, not on a rename.
    let project_id = create_memo(move |_| project_info.with(|p| p.as_ref().map(|p| p.id.clone())));
    create_effect(move |_| {
        let Some(project) = project_id.get() else {
            pending.set(Vec::new());
            return;
        };
        pending.set(Vec::new());
        spawn_local(async move {
            // Hosts without the command (or an empty reply) show nothing.
            let list = call::<Vec<ProjectDatabaseRecovery>>(
                "list_project_database_recoveries",
                serde_json::json!({}),
            )
            .await
            .unwrap_or_default();
            if project_id.get_untracked().as_deref() == Some(project.as_str()) {
                pending.set(
                    list.into_iter()
                        .filter(|outcome| outcome.project_id == project)
                        .collect(),
                );
            }
        });
    });
    let dismiss = move |id: i64| {
        pending.update(|list| list.retain(|outcome| outcome.id != id));
        spawn_local(async move {
            let _ = call::<bool>(
                "acknowledge_project_database_recovery",
                serde_json::json!({ "id": id }),
            )
            .await;
        });
    };
    view! {
        <For each=move || pending.get() key=|outcome| outcome.id let:outcome>
            {move || {
                let outcome = outcome.clone();
                let loc = locale.get();
                let rebuilt = outcome.kind == ProjectDatabaseRecoveryKind::Rebuilt;
                let time = format_message_datetime(outcome.occurred_at, loc);
                let (eyebrow, title, body) = if rebuilt {
                    (
                        t(loc, "dbrecovery.rebuilt.eyebrow"),
                        t(loc, "dbrecovery.rebuilt.title"),
                        tf(loc, "dbrecovery.rebuilt.body", &[("time", &time)]),
                    )
                } else {
                    let copy = outcome.copy_name.clone().unwrap_or_default();
                    (
                        t(loc, "dbrecovery.restored.eyebrow"),
                        t(loc, "dbrecovery.restored.title"),
                        tf(
                            loc,
                            "dbrecovery.restored.body",
                            &[("copy", &copy), ("time", &time)],
                        ),
                    )
                };
                let id = outcome.id;
                let dismiss_label = t(loc, "dbrecovery.dismiss");
                view! {
                    <section
                        class="exploration-banner database-recovery"
                        class:rebuilt=rebuilt
                        class:restored=!rebuilt
                        data-testid="database-recovery-notice"
                        data-kind=outcome.kind.as_str()
                        role=if rebuilt { "alert" } else { "status" }>
                        <span class="database-recovery-icon" aria-hidden="true">
                            {compose_icon(if rebuilt { "circle-alert" } else { "database" })}
                        </span>
                        <div class="exploration-banner-copy">
                            <span class="exploration-banner-eyebrow">{eyebrow}</span>
                            <strong>{title}</strong>
                            <span>{body}</span>
                            <span class="database-recovery-label">{t(loc, "dbrecovery.damaged_label")}</span>
                            <code class="browser-extension-path" data-testid="database-recovery-damaged"
                                title=outcome.damaged_path.clone()>{outcome.damaged_path.clone()}</code>
                            {outcome.backup_dir.clone().map(|dir| view! {
                                <span class="database-recovery-label">{t(loc, "dbrecovery.backups_label")}</span>
                                <code class="browser-extension-path" data-testid="database-recovery-backups"
                                    title=dir.clone()>{dir}</code>
                            })}
                        </div>
                        <div class="exploration-banner-actions">
                            <button type="button"
                                on:click=move |_| crate::bindings::open_external_url(recovery_guide_url(loc))>
                                {t(loc, "dbrecovery.guide")}
                            </button>
                            <button type="button" class="icon-btn database-recovery-dismiss"
                                title=dismiss_label.clone() aria-label=dismiss_label
                                on:click=move |_| dismiss(id)>
                                {compose_icon("close")}
                            </button>
                        </div>
                    </section>
                }
            }}
        </For>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_link_follows_the_locale() {
        assert!(recovery_guide_url(Locale::En).contains("/docs/upgrade-backups.md#"));
        assert!(recovery_guide_url(Locale::Zh).contains("/docs/upgrade-backups.zh-CN.md#"));
    }
}
