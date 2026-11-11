//! Automation page, opened from the research assistant because it spans
//! projects: the built-in daily recap, research mandates (`mandates.rs`) and
//! every project's scheduled prompts, created from a template or from
//! scratch. All of it fires only while Wisp runs; a missed slot runs once on
//! the next launch.
use crate::app_support::compose_icon;
use crate::dto::{DailyRecapAutomation, MemoryTidyAutomation, ProjectSummary, ScheduleRecord};
use crate::i18n::Locale;
use crate::research_journey::{call, clock, date, day_key, now};
use leptos::*;

const DAY: i64 = 86_400;
const WEEK: i64 = 7 * DAY;

#[derive(Clone, Copy, PartialEq)]
enum Cadence {
    Daily,
    Weekly,
    Hourly,
}

#[derive(Clone)]
struct Draft {
    project_id: String,
    name: String,
    prompt: String,
    cadence: Cadence,
    time: String,
    /// JavaScript weekday, 0 = Sunday.
    weekday: u32,
    hours: i64,
}

struct Template {
    icon: &'static str,
    title: &'static str,
    prompt: &'static str,
    cadence: Cadence,
    time: &'static str,
    weekday: u32,
}

static TEMPLATES: [Template; 3] = [
    Template {
        icon: "book",
        title: "automation.template.literature_watch",
        prompt:
            "automation.template.search_for_papers_published_in_the_past_7_days_that_bear_on_this",
        cadence: Cadence::Weekly,
        time: "09:00",
        weekday: 1,
    },
    Template {
        icon: "gauge",
        title: "automation.template.run_check",
        prompt:
            "automation.template.check_this_project_s_runs_list_runs_that_are_still_running_failed",
        cadence: Cadence::Daily,
        time: "18:00",
        weekday: 1,
    },
    Template {
        icon: "doc",
        title: "automation.template.weekly_report",
        prompt:
            "automation.template.summarize_this_week_s_research_journey_for_this_project_including",
        cadence: Cadence::Weekly,
        time: "17:00",
        weekday: 5,
    },
];

fn weekday_name(loc: Locale, day: u32) -> &'static str {
    let keys = [
        "calendar.weekday.sun",
        "calendar.weekday.mon",
        "calendar.weekday.tue",
        "calendar.weekday.wed",
        "calendar.weekday.thu",
        "calendar.weekday.fri",
        "calendar.weekday.sat",
    ];
    crate::i18n::tr(loc, keys[(day % 7) as usize])
}

/// Human cadence from the interval and the next slot's local wall clock.
fn cadence_label(loc: Locale, schedule: &ScheduleRecord) -> String {
    let at = clock(schedule.next_run_at);
    match schedule.interval_secs {
        DAY => format!("{} {at}", crate::i18n::tr(loc, "automation.daily")),
        WEEK => format!(
            "{} {at}",
            weekday_name(loc, date(schedule.next_run_at).get_day())
        ),
        n if n % 3600 == 0 => {
            let h = n / 3600;
            crate::i18n::tf(
                loc,
                "automation.cadence_hours",
                &[("count", &h.to_string())],
            )
        }
        n => {
            let m = n / 60;
            crate::i18n::tf(
                loc,
                "automation.every_minutes",
                &[("count", &m.to_string())],
            )
        }
    }
}

/// The next local `HH:MM` strictly after now, optionally on a weekday.
fn next_slot(time: &str, weekday: Option<u32>) -> Option<i64> {
    let (h, m) = time.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    let today = date(now());
    for offset in 0..=7 {
        let candidate = js_sys::Date::new_with_year_month_day_hr_min(
            today.get_full_year(),
            today.get_month() as i32,
            today.get_date() as i32 + offset,
            h as i32,
            m as i32,
        );
        let ts = (candidate.get_time() / 1000.0) as i64;
        if ts > now() && weekday.is_none_or(|w| candidate.get_day() == w) {
            return Some(ts);
        }
    }
    None
}

#[component]
pub(crate) fn AutomationPage(
    locale: RwSignal<Locale>,
    projects: Signal<Vec<ProjectSummary>>,
    form_open: RwSignal<bool>,
    on_open_specialists: Callback<()>,
    on_close: Callback<()>,
) -> impl IntoView {
    let refresh = create_rw_signal(0u32);
    let daily = create_rw_signal(None::<DailyRecapAutomation>);
    let tidy = create_rw_signal(None::<MemoryTidyAutomation>);
    let schedules = create_rw_signal(None::<Result<Vec<ScheduleRecord>, String>>);
    let notice = create_rw_signal(None::<String>);
    let deleting = create_rw_signal(None::<String>);
    let blank = move || Draft {
        project_id: projects
            .get_untracked()
            .first()
            .map(|p| p.id.clone())
            .unwrap_or_default(),
        name: String::new(),
        prompt: String::new(),
        cadence: Cadence::Daily,
        time: "09:00".into(),
        weekday: 1,
        hours: 6,
    };
    let draft = create_rw_signal(blank());
    let form_error = create_rw_signal(None::<String>);
    // `form_open` is one Escape layer; this says which form it is showing.
    let mandate_form = create_rw_signal(false);
    create_effect(move |_| {
        refresh.get();
        spawn_local(async move {
            daily.set(
                call::<DailyRecapAutomation>("get_daily_recap_automation", serde_json::json!({}))
                    .await
                    .ok(),
            );
            schedules.set(Some(
                call::<Vec<ScheduleRecord>>("list_all_schedules", serde_json::json!({})).await,
            ));
            tidy.set(
                call::<MemoryTidyAutomation>("get_memory_tidy_automation", serde_json::json!({}))
                    .await
                    .ok(),
            );
        });
    });
    // Every memory tidy command answers with the automation's new state.
    let tidy_act = move |command: &'static str, args: serde_json::Value| {
        spawn_local(async move {
            match call::<MemoryTidyAutomation>(command, args).await {
                Ok(value) => tidy.set(Some(value)),
                Err(e) => {
                    notice.set(Some(e));
                    refresh.update(|n| *n += 1);
                }
            }
        });
    };
    // While a recap or tidy run is in flight, look again shortly.
    create_effect(move |_| {
        if daily.get().is_some_and(|d| d.running) || tidy.get().is_some_and(|t| t.running) {
            set_timeout(
                move || refresh.update(|n| *n += 1),
                std::time::Duration::from_secs(3),
            );
        }
    });
    let save_daily = move |enabled: bool, time: String| {
        spawn_local(async move {
            match call::<DailyRecapAutomation>(
                "set_daily_recap_automation",
                serde_json::json!({"enabled":enabled,"time":time}),
            )
            .await
            {
                Ok(value) => daily.set(Some(value)),
                Err(e) => notice.set(Some(e)),
            }
        });
    };
    let act = move |command: &'static str, args: serde_json::Value| {
        spawn_local(async move {
            match call::<serde_json::Value>(command, args).await {
                Ok(_) => refresh.update(|n| *n += 1),
                Err(e) => notice.set(Some(e)),
            }
        });
    };
    let start = move |template: Option<&'static Template>| {
        let loc = locale.get_untracked();
        let mut next = blank();
        if let Some(t) = template {
            next.name = crate::i18n::tr(loc, t.title).into();
            next.prompt = crate::i18n::tr(loc, t.prompt).into();
            next.cadence = t.cadence;
            next.time = t.time.into();
            next.weekday = t.weekday;
        }
        draft.set(next);
        form_error.set(None);
        mandate_form.set(false);
        form_open.set(true);
    };
    let create = move |_| {
        let d = draft.get_untracked();
        let loc = locale.get_untracked();
        if d.project_id.is_empty() || d.prompt.trim().is_empty() {
            form_error.set(Some(
                crate::i18n::tr(loc, "automation.choose_a_project_and_write_a_prompt").into(),
            ));
            return;
        }
        let (interval, start_at) = match d.cadence {
            Cadence::Daily => (DAY, next_slot(&d.time, None)),
            Cadence::Weekly => (WEEK, next_slot(&d.time, Some(d.weekday))),
            Cadence::Hourly => (d.hours.clamp(1, 168) * 3600, None),
        };
        if d.cadence != Cadence::Hourly && start_at.is_none() {
            form_error.set(Some(
                crate::i18n::tr(loc, "automation.use_a_24_hour_hh_mm_time").into(),
            ));
            return;
        }
        spawn_local(async move {
            match call::<ScheduleRecord>(
                "create_schedule",
                serde_json::json!({
                    "projectId": d.project_id, "name": d.name, "prompt": d.prompt,
                    "intervalSecs": interval, "startAt": start_at, "sessionId": null, "skill": null,
                }),
            )
            .await
            {
                Ok(_) => {
                    form_open.set(false);
                    refresh.update(|n| *n += 1);
                }
                Err(e) => form_error.set(Some(e)),
            }
        });
    };
    let project_name =
        move |id: &str| projects.with(|ps| ps.iter().find(|p| p.id == id).map(|p| p.name.clone()));
    view! {
        <section class="home-calendar home-automation" data-testid="home-automation" aria-label=move || crate::i18n::tr(locale.get(), "automation.automation")>
            <button type="button" class="calendar-back" on:click=move |_| on_close.call(())>{compose_icon("arrow-left")}{move || crate::i18n::tr(locale.get(), "automation.back_to_research_assistant")}</button>
            <header class="home-calendar-heading"><div><h2>{move || crate::i18n::tr(locale.get(), "automation.automation")}</h2><p>{move || crate::i18n::tr(locale.get(), "automation.run_tasks_on_a_schedule_while_wisp_is_open_or_any_time_you_need_t")}</p></div>
                <div class="automation-heading-actions">
                    <button type="button" class="calendar-icon" aria-label=move || crate::i18n::tr(locale.get(), "automation.refresh_automation") on:click=move |_| refresh.update(|n| *n += 1)>{compose_icon("refresh")}</button>
                    <button type="button" class="btn-primary" data-testid="automation-create" on:click=move |_| start(None)>{compose_icon("plus")}{move || crate::i18n::tr(locale.get(), "automation.new_scheduled_task")}</button>
                </div>
            </header>
            {move || notice.get().map(|e| view! {<p class="automation-notice" role="alert">{e}</p>})}
            <section class="automation-card automation-builtin" data-testid="automation-daily-recap">
                {move || {
                    let loc = locale.get();
                    let Some(d) = daily.get() else { return view! {<p class="automation-meta" role="status">{crate::i18n::tr(loc, "privacy.loading")}</p>}.into_view(); };
                    let (enabled, time) = (d.enabled, d.time.clone());
                    let toggle_time = time.clone();
                    let status = if d.running {
                        crate::i18n::tr(loc, "automation.running_now").to_string()
                    } else if let Some(at) = d.last_run_at {
                        let when = format!("{} {}", day_key(at), clock(at));
                        crate::i18n::tf(loc, "automation.last_drafted", &[("when", &when), ("count", &d.drafted.to_string())])
                    } else {
                        crate::i18n::tr(loc, "automation.not_run_yet").to_string()
                    };
                    view! {
                        <div class="automation-card-head">{compose_icon("sparkles")}<h3>{crate::i18n::tr(loc, "automation.daily_research_recap")}</h3><span class="automation-tag">{crate::i18n::tr(loc, "conn.bundled_badge")}</span>
                            <label class="automation-toggle"><input type="checkbox" role="switch" data-testid="daily-recap-enabled" prop:checked=enabled on:change=move |ev| save_daily(event_target_checked(&ev), toggle_time.clone())/><span>{if enabled {crate::i18n::tr(loc, "automation.on")} else {crate::i18n::tr(loc, "automation.off")}}</span></label>
                        </div>
                        <p class="automation-desc">{crate::i18n::tr(loc, "automation.every_day_after")}" "
                            <input type="time" class="automation-time" data-testid="daily-recap-time" aria-label=crate::i18n::tr(loc, "automation.daily_recap_time") prop:value=time on:change=move |ev| save_daily(enabled, event_target_value(&ev))/>
                            " "{crate::i18n::tr(loc, "automation.it_drafts_a_recap_for_each_project_s_recent_days_with_recorded_ac")}</p>
                        <p class="automation-meta" data-testid="daily-recap-status">{status}</p>
                        {d.error.map(|e| view! {<p class="automation-error" role="alert">{e}</p>})}
                        <div class="automation-actions">
                            <button type="button" class="btn-ghost" data-testid="daily-recap-run" prop:disabled=d.running on:click=move |_| spawn_local(async move {
                                if let Ok(value) = call::<DailyRecapAutomation>("run_daily_recap_now", serde_json::json!({})).await { daily.set(Some(value)); }
                            })>{compose_icon("play")}{crate::i18n::tr(loc, "automation.run_now")}</button>
                            <button type="button" class="journey-link" on:click=move |_| on_open_specialists.call(())>{compose_icon("gear")}{crate::i18n::tr(loc, "automation.change_model_in_settings_specialists_recap")}</button>
                        </div>
                    }.into_view()
                }}
            </section>
            <section class="automation-card automation-builtin" data-testid="automation-memory-tidy">
                {move || {
                    let loc = locale.get();
                    let Some(t) = tidy.get() else { return view! {<p class="automation-meta" role="status">{crate::i18n::tr(loc, "privacy.loading")}</p>}.into_view(); };
                    let count = t.proposals.len();
                    let status = if t.running {
                        crate::i18n::tr(loc, "automation.running_now").to_string()
                    } else if let Some(at) = t.last_run_at {
                        let when = format!("{} {}", day_key(at), clock(at));
                        crate::i18n::tf(loc, "automation.last_proposals", &[("when", &when), ("count", &count.to_string())])
                    } else {
                        crate::i18n::tr(loc, "automation.not_run_yet").to_string()
                    };
                    view! {
                        <div class="automation-card-head">{compose_icon("memory")}<h3>{crate::i18n::tr(loc, "automation.memory_tidy")}</h3><span class="automation-tag">{crate::i18n::tr(loc, "conn.bundled_badge")}</span>
                            <label class="automation-toggle"><input type="checkbox" role="switch" data-testid="memory-tidy-enabled" prop:checked=t.enabled on:change=move |ev| tidy_act("set_memory_tidy_automation", serde_json::json!({"enabled":event_target_checked(&ev)}))/><span>{if t.enabled {crate::i18n::tr(loc, "automation.on")} else {crate::i18n::tr(loc, "automation.off")}}</span></label>
                        </div>
                        <p class="automation-desc">{crate::i18n::tr(loc, "automation.once_a_week_it_looks_over_your_global_habits_and_proposes_merging")}</p>
                        <p class="automation-meta" data-testid="memory-tidy-status">{status}</p>
                        {t.error.map(|e| view! {<p class="automation-error" role="alert">{e}</p>})}
                        {t.proposals.into_iter().map(|p| {
                            let merge = p.action == "merge";
                            let n = p.memories.len();
                            let title = crate::i18n::tf(loc, if merge { "automation.merge_memories" } else { "automation.retire_memories" }, &[("count", &n.to_string())]);
                            let (apply_id, dismiss_id) = (p.id.clone(), p.id.clone());
                            view! {<article class="memory-tidy-proposal" data-testid="memory-tidy-proposal" data-action=p.action.clone()>
                                <strong>{title}</strong>
                                {(!p.reason.is_empty()).then(|| view! {<small>{p.reason.clone()}</small>})}
                                <ul>{p.memories.iter().map(|m| view! {<li>{m.content.clone()}</li>}).collect_view()}</ul>
                                {merge.then(|| view! {<p class="memory-tidy-merged"><b>{crate::i18n::tr(loc, "automation.becomes")}</b>{p.text.clone()}</p>})}
                                <div class="automation-actions">
                                    <button type="button" class="btn-primary" data-testid="memory-tidy-apply" on:click=move |_| tidy_act("apply_memory_tidy_proposal", serde_json::json!({"id":apply_id.clone()}))>{crate::i18n::tr(loc, "automation.apply")}</button>
                                    <button type="button" class="btn-ghost" data-testid="memory-tidy-dismiss" on:click=move |_| tidy_act("dismiss_memory_tidy_proposal", serde_json::json!({"id":dismiss_id.clone()}))>{crate::i18n::tr(loc, "automation.dismiss")}</button>
                                </div>
                            </article>}
                        }).collect_view()}
                        <div class="automation-actions">
                            <button type="button" class="btn-ghost" data-testid="memory-tidy-run" prop:disabled=t.running on:click=move |_| tidy_act("run_memory_tidy_now", serde_json::json!({}))>{compose_icon("play")}{crate::i18n::tr(loc, "automation.run_now")}</button>
                            <button type="button" class="journey-link" on:click=move |_| on_open_specialists.call(())>{compose_icon("gear")}{crate::i18n::tr(loc, "automation.it_uses_the_recap_specialist_s_model")}</button>
                        </div>
                    }.into_view()
                }}
            </section>
            <crate::mandates::MandateSection locale=locale projects=projects form_open=form_open mandate_form=mandate_form refresh=refresh notice=notice/>
            {move || (form_open.get() && !mandate_form.get()).then(|| {
                let loc = locale.get();
                view! {<section class="automation-card automation-form" data-testid="automation-form" aria-label=crate::i18n::tr(loc, "automation.new_scheduled_task")>
                    <h3>{crate::i18n::tr(loc, "automation.new_scheduled_task")}</h3>
                    <div class="automation-form-grid">
                        <label>{crate::i18n::tr(loc, "permissions.scope.project")}<select on:change=move |ev| draft.update(|d| d.project_id = event_target_value(&ev))>
                            {projects.get().into_iter().map(|p| { let id = p.id.clone(); view! {<option value=p.id prop:selected=move || draft.with(|d| d.project_id == id)>{p.name}</option>} }).collect_view()}
                        </select></label>
                        <label>{crate::i18n::tr(loc, "proj_settings.name")}<input type="text" prop:value=move || draft.with(|d| d.name.clone()) on:input=move |ev| draft.update(|d| d.name = event_target_value(&ev))/></label>
                        <label>{crate::i18n::tr(loc, "automation.repeat")}<select on:change=move |ev| draft.update(|d| d.cadence = match event_target_value(&ev).as_str() { "weekly" => Cadence::Weekly, "hourly" => Cadence::Hourly, _ => Cadence::Daily })>
                            <option value="daily" prop:selected=move || draft.with(|d| d.cadence == Cadence::Daily)>{crate::i18n::tr(loc, "automation.daily")}</option>
                            <option value="weekly" prop:selected=move || draft.with(|d| d.cadence == Cadence::Weekly)>{crate::i18n::tr(loc, "settings.usage.weekly")}</option>
                            <option value="hourly" prop:selected=move || draft.with(|d| d.cadence == Cadence::Hourly)>{crate::i18n::tr(loc, "automation.every_few_hours")}</option>
                        </select></label>
                        {move || match draft.with(|d| d.cadence) {
                            Cadence::Hourly => view! {<label>{crate::i18n::tr(loc, "automation.every_hours")}<input type="number" min="1" max="168" prop:value=move || draft.with(|d| d.hours.to_string()) on:input=move |ev| draft.update(|d| d.hours = event_target_value(&ev).parse().unwrap_or(1))/></label>}.into_view(),
                            cadence => view! {
                                {(cadence == Cadence::Weekly).then(|| view! {<label>{crate::i18n::tr(loc, "automation.day")}<select on:change=move |ev| draft.update(|d| d.weekday = event_target_value(&ev).parse().unwrap_or(1))>
                                    {[1u32, 2, 3, 4, 5, 6, 0].into_iter().map(|w| view! {<option value=w.to_string() prop:selected=move || draft.with(|d| d.weekday == w)>{weekday_name(loc, w)}</option>}).collect_view()}
                                </select></label>})}
                                <label>{crate::i18n::tr(loc, "automation.time")}<input type="time" prop:value=move || draft.with(|d| d.time.clone()) on:input=move |ev| draft.update(|d| d.time = event_target_value(&ev))/></label>
                            }.into_view(),
                        }}
                        <label class="automation-span">{crate::i18n::tr(loc, "automation.prompt")}<textarea rows="6" prop:value=move || draft.with(|d| d.prompt.clone()) on:input=move |ev| draft.update(|d| d.prompt = event_target_value(&ev))></textarea></label>
                    </div>
                    <p class="automation-meta">{crate::i18n::tr(loc, "automation.when_due_wisp_starts_a_new_session_in_the_project_and_sends_this")}</p>
                    {move || form_error.get().map(|e| view! {<p class="automation-error" role="alert">{e}</p>})}
                    <div class="automation-actions automation-form-actions">
                        <button type="button" class="btn-ghost" on:click=move |_| form_open.set(false)>{crate::i18n::tr(loc, "projects.cancel")}</button>
                        <button type="button" class="btn-primary" data-testid="automation-save" on:click=create>{crate::i18n::tr(loc, "projects.create")}</button>
                    </div>
                </section>}
            })}
            <section class="automation-section">
                <h3>{move || crate::i18n::tr(locale.get(), "automation.scheduled_tasks")}</h3>
                {move || {
                    let loc = locale.get();
                    match schedules.get() {
                        None => view! {<p class="automation-meta" role="status">{crate::i18n::tr(loc, "privacy.loading")}</p>}.into_view(),
                        Some(Err(e)) => view! {<p class="automation-error" role="alert">{e}</p>}.into_view(),
                        Some(Ok(rows)) => {
                            // Privacy mode hides a project's tasks with the project.
                            let rows: Vec<_> = rows.into_iter().filter_map(|s| project_name(&s.project_id).map(|name| (s, name))).collect();
                            if rows.is_empty() {
                                return view! {<div class="automation-empty" data-testid="automation-empty">{crate::i18n::tr(loc, "automation.no_scheduled_tasks_yet_start_from_a_template_below_or_write_your")}</div>}.into_view();
                            }
                            rows.into_iter().map(|(s, name)| {
                                let (id, toggle_id, run_id, delete_id, confirm_id) = (s.id.clone(), s.id.clone(), s.id.clone(), s.id.clone(), s.id.clone());
                                let enabled = s.enabled;
                                let timer = s.replace_previous_turn;
                                let next = if enabled { format!("{} {} {}", crate::i18n::tr(loc, "automation.next"), day_key(s.next_run_at), clock(s.next_run_at)) } else { crate::i18n::tr(loc, "runs.status.paused").into() };
                                view! {<article class="automation-row" data-schedule-id=id class:paused=!enabled>
                                    <div class="automation-row-main"><strong>{s.name.clone()}</strong>
                                        <small>{format!("{name} · {} · {next}", cadence_label(loc, &s))}</small>
                                        <p class="automation-prompt">{s.prompt.clone()}</p></div>
                                    <div class="automation-row-actions">
                                        <label class="automation-toggle"><input type="checkbox" role="switch" aria-label=format!("{} {}", crate::i18n::tr(loc, "automation.enable"), s.name) prop:checked=enabled on:change=move |ev| act("set_schedule_enabled", serde_json::json!({"id":toggle_id,"enabled":event_target_checked(&ev)}))/></label>
                                        <button type="button" class="calendar-icon" title=crate::i18n::tr(loc, "automation.run_now") aria-label=format!("{} {}", crate::i18n::tr(loc, "automation.run_now"), s.name) on:click=move |_| { act("run_schedule_now", serde_json::json!({"id":run_id})); notice.set(Some(if timer { crate::i18n::tr(locale.get_untracked(), "automation.requested_a_check_in_the_bound_conversation") } else { crate::i18n::tr(locale.get_untracked(), "automation.started_the_result_appears_in_a_new_session_of_that_project") }.into())); }>{compose_icon("play")}</button>
                                        <button type="button" class="calendar-icon automation-delete" class:confirming=move || deleting.get().as_ref() == Some(&confirm_id)
                                            title=move || if deleting.get().as_ref() == Some(&delete_id) {crate::i18n::tr(locale.get(), "automation.click_again_to_delete")} else {crate::i18n::tr(locale.get(), "projects.delete")}
                                            aria-label=format!("{} {}", crate::i18n::tr(loc, "projects.delete"), s.name)
                                            on:click=move |_| if deleting.get_untracked().as_ref() == Some(&s.id) { deleting.set(None); act("delete_schedule", serde_json::json!({"id":s.id.clone()})); } else { deleting.set(Some(s.id.clone())); }>{compose_icon("trash")}</button>
                                    </div>
                                </article>}
                            }).collect_view()
                        }
                    }
                }}
            </section>
            <section class="automation-section">
                <h3>{move || crate::i18n::tr(locale.get(), "automation.templates")}</h3>
                <div class="automation-templates">
                    {TEMPLATES.iter().map(|t| view! {
                        <button type="button" class="automation-template" on:click=move |_| start(Some(t))>
                            <span class="automation-template-head">{compose_icon(t.icon)}<strong>{move || crate::i18n::tr(locale.get(), t.title)}</strong></span>
                            <span class="automation-template-desc">{move || crate::i18n::tr(locale.get(), t.prompt)}</span>
                            <small>{move || { let loc = locale.get(); let when = if t.cadence == Cadence::Weekly { weekday_name(loc, t.weekday).to_string() } else { crate::i18n::tr(loc, "automation.daily").to_string() }; format!("{when} {}", t.time) }}</small>
                        </button>
                    }).collect_view()}
                </div>
            </section>
        </section>
    }
}
