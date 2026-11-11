//! Research mandates on the Automation page: a long-running responsibility
//! defined by a goal, KPIs and constraints, carried round by round in its own
//! project conversation. Rounds run only while Wisp is open.
use crate::app_support::compose_icon;
use crate::dto::{
    MandateConstraints, MandateDraft, MandateKpi, MandateOverview, MandateRecord, MandateReport,
    ProjectSummary, ResearchRecapItem,
};
use crate::i18n::Locale;
use crate::research_journey::{call, clock, day_key, now};
use leptos::*;

const DAY: i64 = 86_400;

/// The form's own shape: text and whole units, converted on save.
#[derive(Clone, Default)]
struct Form {
    /// The mandate being edited; `None` creates one.
    id: Option<String>,
    project_id: String,
    name: String,
    goal: String,
    kpis: Vec<MandateKpi>,
    autonomous: String,
    review_first: String,
    ask_for_help: String,
    review_mutations: bool,
    /// `YYYY-MM-DD`, or empty for an open-ended mandate.
    end_date: String,
    hours: i64,
    report_days: i64,
    min_minutes: i64,
    max_days: i64,
    max_rounds: u32,
    /// Standing instructions, one per line: confirmed from feedback in the
    /// mandate's conversation, or written here.
    notes: String,
}

struct Template {
    icon: &'static str,
    title: &'static str,
    goal: &'static str,
    /// Catalog keys for name, definition, target and period.
    kpis: &'static [[&'static str; 4]],
    autonomous: &'static str,
    review_first: &'static str,
    ask_for_help: &'static str,
    review_mutations: bool,
    hours: i64,
    report_days: i64,
    days: i64,
}

static TEMPLATES: [Template; 4] = [
    Template {
        icon: "book",
        title: "automation.template.literature_watch",
        goal: "mandates.template.for_the_next_12_months_keep_this_project_s_literature_current_fin",
        kpis: &[[
            "mandates.template.papers_screened_and_filed",
            "mandates.template.read_at_abstract_level_or_deeper_judged_relevant_and_saved_to_the",
            "mandates.template.5",
            "mandates.template.per_week",
        ]],
        autonomous:
            "mandates.template.search_literature_databases_read_papers_and_save_references_and_n",
        review_first:
            "mandates.template.changing_the_project_s_analysis_plan_or_conclusions_because_of_a",
        ask_for_help:
            "mandates.template.a_paper_needs_an_institutional_login_or_a_finding_contradicts_our",
        review_mutations: false,
        hours: 24,
        report_days: 7,
        days: 365,
    },
    Template {
        icon: "doc",
        title: "mandates.template.submission_follow_up",
        goal: "mandates.template.follow_this_manuscript_from_submission_to_acceptance_track_the_jo",
        kpis: &[
            [
                "mandates.template.open_reviewer_points",
                "mandates.template.reviewer_comments_that_have_no_drafted_response_yet",
                "mandates.template.0",
                "mandates.template.before_the_revision_deadline",
            ],
            [
                "mandates.template.revision_checklist_done",
                "mandates.template.checklist_items_finished_and_checked_against_the_manuscript",
                "mandates.template.100",
                "mandates.template.before_the_revision_deadline",
            ],
        ],
        autonomous:
            "mandates.template.read_the_manuscript_and_reviews_draft_responses_and_checklists_an",
        review_first:
            "mandates.template.any_text_sent_to_editors_or_reviewers_and_any_change_to_the_manus",
        ask_for_help:
            "mandates.template.logging_in_to_the_submission_system_confirming_authorship_or_paym",
        review_mutations: true,
        hours: 72,
        report_days: 7,
        days: 180,
    },
    Template {
        icon: "server",
        title: "mandates.template.long_running_compute",
        goal: "mandates.template.see_this_computation_through_to_verified_results_submit_and_monit",
        kpis: &[[
            "mandates.template.samples_completed",
            "mandates.template.samples_whose_run_succeeded_and_whose_outputs_were_harvested_and",
            "mandates.template.12",
            "mandates.template.by_the_end_date",
        ]],
        autonomous:
            "mandates.template.submit_monitor_cancel_and_resubmit_runs_on_the_configured_servers",
        review_first:
            "mandates.template.changing_analysis_parameters_deleting_data_or_using_more_compute",
        ask_for_help:
            "mandates.template.a_server_needs_credentials_or_quota_or_a_failure_repeats_after_tw",
        review_mutations: false,
        hours: 6,
        report_days: 1,
        days: 30,
    },
    Template {
        icon: "gauge",
        title: "mandates.template.method_metric_iteration",
        goal: "mandates.template.over_the_next_3_months_improve_this_method_against_its_benchmark",
        kpis: &[[
            "mandates.template.benchmark_metric",
            "mandates.template.the_primary_metric_on_the_held_out_benchmark_measured_with_the_fi",
            "mandates.template.0_90",
            "mandates.template.by_the_end_date",
        ]],
        autonomous:
            "mandates.template.run_evaluations_and_experiments_analyze_results_and_prepare_code",
        review_first:
            "mandates.template.merging_code_or_changing_the_benchmark_or_the_evaluation_script",
        ask_for_help:
            "mandates.template.the_metric_stalls_for_three_rounds_or_a_result_looks_too_good_to",
        review_mutations: true,
        hours: 24,
        report_days: 7,
        days: 90,
    },
];

fn pick(loc: Locale, key: &'static str) -> &'static str {
    crate::i18n::tr(loc, key)
}

fn request_kind_label(loc: Locale, kind: &str) -> &'static str {
    match kind {
        "login" => crate::i18n::tr(loc, "mandates.sign_in"),
        "materials" => crate::i18n::tr(loc, "mandates.materials"),
        "payment" => crate::i18n::tr(loc, "mandates.payment"),
        "release" => crate::i18n::tr(loc, "mandates.release"),
        _ => crate::i18n::tr(loc, "mandates.your_judgement"),
    }
}

/// One report: its period, headline, and each section's items with the
/// records they cite.
fn report_view(loc: Locale, report: MandateReport) -> impl IntoView {
    let period = format!(
        "{} – {}",
        day_key(report.period_from),
        day_key(report.period_until)
    );
    let rounds = crate::i18n::tf(
        loc,
        "mandates.round_count",
        &[("count", &report.rounds.to_string())],
    );
    // A period with rounds but no model was assembled from the ledger.
    let ledger = (report.rounds > 0 && report.body.model.is_empty())
        .then(|| crate::i18n::tr(loc, "mandates.from_the_ledger"));
    let sources = report.body.sources;
    let section = move |label: &'static str, items: Vec<ResearchRecapItem>| {
        let sources = sources.clone();
        (!items.is_empty()).then(|| view! {<div class="mandate-report-section"><b>{label}</b><ul>
            {items.into_iter().map(|item| {
                let cited: Vec<_> = item.refs.iter().filter_map(|i| sources.get(*i)).map(|s| s.title.clone()).collect();
                view! {<li>{item.text}{(!cited.is_empty()).then(|| view! {<span class="mandate-cite">{cited.join(", ")}</span>})}</li>}
            }).collect_view()}
        </ul></div>})
    };
    view! {<article class="mandate-report" data-testid="mandate-report">
        <small>{format!("{period} · {rounds}")}{ledger}</small>
        <strong>{report.body.headline}</strong>
        {section.clone()(crate::i18n::tr(loc, "mandates.done"), report.body.done)}
        {section.clone()(crate::i18n::tr(loc, "mandates.progress"), report.body.findings)}
        {section.clone()(crate::i18n::tr(loc, "mandates.blocked"), report.body.issues)}
        {section(crate::i18n::tr(loc, "onboard.next"), report.body.next)}
    </article>}
}

fn status_label(loc: Locale, status: &str) -> &'static str {
    match status {
        "paused" => crate::i18n::tr(loc, "runs.status.paused"),
        "waiting" => crate::i18n::tr(loc, "mandates.needs_you"),
        "done" => crate::i18n::tr(loc, "mandates.closed"),
        _ => crate::i18n::tr(loc, "exploration.status_active"),
    }
}

/// Local end of day for a `YYYY-MM-DD` date input.
fn end_of_day(value: &str) -> Option<i64> {
    let mut parts = value.split('-').map(|part| part.parse::<i32>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    let ts =
        js_sys::Date::new_with_year_month_day_hr_min_sec(y as u32, m - 1, d, 23, 59, 59).get_time();
    ts.is_finite().then_some((ts / 1000.0) as i64)
}

fn form_from(mandate: &MandateRecord) -> Form {
    let c = &mandate.constraints;
    Form {
        id: Some(mandate.id.clone()),
        project_id: mandate.project_id.clone(),
        name: mandate.name.clone(),
        goal: mandate.goal.clone(),
        kpis: mandate.kpis.clone(),
        autonomous: c.autonomous.clone(),
        review_first: c.review_first.clone(),
        ask_for_help: c.ask_for_help.clone(),
        review_mutations: c.review_mutations,
        end_date: mandate.ends_at.map(day_key).unwrap_or_default(),
        hours: (mandate.interval_secs / 3600).max(1),
        report_days: (mandate.report_interval_secs / DAY).max(1),
        min_minutes: (c.min_interval_secs / 60).max(1),
        max_days: (c.max_interval_secs / DAY).max(1),
        max_rounds: c.max_rounds_per_day,
        notes: c.notes.join("\n"),
    }
}

fn draft_from(form: &Form) -> MandateDraft {
    MandateDraft {
        project_id: form.project_id.clone(),
        name: form.name.clone(),
        goal: form.goal.clone(),
        kpis: form.kpis.clone(),
        constraints: MandateConstraints {
            autonomous: form.autonomous.clone(),
            review_first: form.review_first.clone(),
            ask_for_help: form.ask_for_help.clone(),
            review_mutations: form.review_mutations,
            min_interval_secs: form.min_minutes.max(1) * 60,
            max_interval_secs: form.max_days.max(1) * DAY,
            max_rounds_per_day: form.max_rounds.max(1),
            notes: form
                .notes
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect(),
        },
        ends_at: end_of_day(&form.end_date),
        interval_secs: form.hours.max(1) * 3600,
        report_interval_secs: form.report_days.max(1) * DAY,
        // A new mandate starts its first round at the next poll.
        start_at: form.id.is_none().then(now),
    }
}

#[component]
pub(crate) fn MandateSection(
    locale: RwSignal<Locale>,
    projects: Signal<Vec<ProjectSummary>>,
    /// The page's one inline-form Escape layer, shared with the task form.
    form_open: RwSignal<bool>,
    /// True while that layer shows the mandate form.
    mandate_form: RwSignal<bool>,
    refresh: RwSignal<u32>,
    notice: RwSignal<Option<String>>,
) -> impl IntoView {
    let mandates = create_rw_signal(None::<Result<Vec<MandateOverview>, String>>);
    let deleting = create_rw_signal(None::<String>);
    let form = create_rw_signal(Form::default());
    let form_error = create_rw_signal(None::<String>);
    // Row count on its own, so typing in a row never rebuilds the rows.
    let kpi_rows = create_memo(move |_| form.with(|f| f.kpis.len()));
    create_effect(move |_| {
        refresh.get();
        spawn_local(async move {
            mandates.set(Some(
                call::<Vec<MandateOverview>>("list_all_mandates", serde_json::json!({})).await,
            ));
        });
    });
    create_effect(move |_| {
        if !form_open.get() {
            mandate_form.set(false);
        }
    });
    let open_form = move |next: Form| {
        form.set(next);
        form_error.set(None);
        mandate_form.set(true);
        form_open.set(true);
    };
    let blank = move || Form {
        project_id: projects
            .get_untracked()
            .first()
            .map(|p| p.id.clone())
            .unwrap_or_default(),
        kpis: vec![MandateKpi::default()],
        review_mutations: true,
        hours: 24,
        report_days: 7,
        min_minutes: 15,
        max_days: 7,
        max_rounds: 6,
        ..Default::default()
    };
    let start = move |template: Option<&'static Template>| {
        let loc = locale.get_untracked();
        let mut next = blank();
        if let Some(t) = template {
            next.name = pick(loc, t.title).into();
            next.goal = pick(loc, t.goal).into();
            next.kpis = t
                .kpis
                .iter()
                .map(|k| MandateKpi {
                    name: pick(loc, k[0]).into(),
                    definition: pick(loc, k[1]).into(),
                    target: pick(loc, k[2]).into(),
                    period: pick(loc, k[3]).into(),
                    current: None,
                })
                .collect();
            next.autonomous = pick(loc, t.autonomous).into();
            next.review_first = pick(loc, t.review_first).into();
            next.ask_for_help = pick(loc, t.ask_for_help).into();
            next.review_mutations = t.review_mutations;
            next.hours = t.hours;
            next.report_days = t.report_days;
            next.end_date = day_key(now() + t.days * DAY);
        }
        open_form(next);
    };
    let act = move |command: &'static str, args: serde_json::Value| {
        spawn_local(async move {
            match call::<serde_json::Value>(command, args).await {
                Ok(_) => refresh.update(|n| *n += 1),
                Err(e) => notice.set(Some(e)),
            }
        });
    };
    let save = move |_| {
        let f = form.get_untracked();
        let loc = locale.get_untracked();
        if f.project_id.is_empty() || f.goal.trim().is_empty() {
            form_error.set(Some(
                crate::i18n::tr(loc, "mandates.choose_a_project_and_write_a_goal").into(),
            ));
            return;
        }
        if !f.end_date.is_empty() && end_of_day(&f.end_date).is_none_or(|end| end <= now()) {
            form_error.set(Some(
                crate::i18n::tr(
                    loc,
                    "mandates.choose_an_end_date_in_the_future_or_leave_it_empty",
                )
                .into(),
            ));
            return;
        }
        let draft = serde_json::to_value(draft_from(&f)).unwrap_or_default();
        let (command, args) = match &f.id {
            Some(id) => (
                "update_mandate",
                serde_json::json!({"id": id, "draft": draft}),
            ),
            None => ("create_mandate", serde_json::json!({"draft": draft})),
        };
        spawn_local(async move {
            match call::<MandateRecord>(command, args).await {
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
    // One KPI cell: reads and writes field `get`/`set` of row `i`.
    let kpi_input = move |i: usize,
                          label: &'static str,
                          class: &'static str,
                          get: fn(&MandateKpi) -> String,
                          set: fn(&mut MandateKpi, String)| {
        view! {<input type="text" class=class aria-label=label placeholder=label
        prop:value=move || form.with(|f| f.kpis.get(i).map(get).unwrap_or_default())
        on:input=move |ev| form.update(|f| if let Some(k) = f.kpis.get_mut(i) { set(k, event_target_value(&ev)) })/>}
    };
    view! {
        {move || (form_open.get() && mandate_form.get()).then(|| {
            let loc = locale.get();
            let editing = form.with_untracked(|f| f.id.is_some());
            let title = if editing { crate::i18n::tr(loc, "mandates.edit_mandate") } else { crate::i18n::tr(loc, "mandates.new_mandate") };
            view! {<section class="automation-card automation-form mandate-form" data-testid="mandate-form" aria-label=title>
                <h3>{title}</h3>
                <div class="automation-form-grid">
                    <label>{crate::i18n::tr(loc, "permissions.scope.project")}<select prop:disabled=editing on:change=move |ev| form.update(|f| f.project_id = event_target_value(&ev))>
                        {projects.get().into_iter().map(|p| { let id = p.id.clone(); view! {<option value=p.id prop:selected=move || form.with(|f| f.project_id == id)>{p.name}</option>} }).collect_view()}
                    </select></label>
                    <label>{crate::i18n::tr(loc, "proj_settings.name")}<input type="text" prop:value=move || form.with(|f| f.name.clone()) on:input=move |ev| form.update(|f| f.name = event_target_value(&ev))/></label>
                    <label>{crate::i18n::tr(loc, "mandates.until")}<input type="date" prop:value=move || form.with(|f| f.end_date.clone()) on:input=move |ev| form.update(|f| f.end_date = event_target_value(&ev))/></label>
                    <label class="automation-span">{crate::i18n::tr(loc, "mandates.goal")}<textarea rows="4" placeholder=crate::i18n::tr(loc, "mandates.who_is_this_for_what_should_be_achieved_and_for_how_long") prop:value=move || form.with(|f| f.goal.clone()) on:input=move |ev| form.update(|f| f.goal = event_target_value(&ev))></textarea></label>
                </div>
                <fieldset class="mandate-fieldset"><legend>{crate::i18n::tr(loc, "mandates.kpis")}</legend>
                    <p class="automation-meta">{crate::i18n::tr(loc, "mandates.how_progress_is_measured_say_what_is_counted_the_target_and_the_p")}</p>
                    {move || (0..kpi_rows.get()).map(|i| view! {<div class="mandate-kpi-row" data-testid="mandate-kpi-row">
                        {kpi_input(i, crate::i18n::tr(loc, "mandates.kpi_name"), "mandate-kpi-name", |k| k.name.clone(), |k, v| k.name = v)}
                        {kpi_input(i, crate::i18n::tr(loc, "mandates.how_it_is_counted"), "mandate-kpi-definition", |k| k.definition.clone(), |k, v| k.definition = v)}
                        {kpi_input(i, crate::i18n::tr(loc, "mandates.target"), "mandate-kpi-target", |k| k.target.clone(), |k, v| k.target = v)}
                        {kpi_input(i, crate::i18n::tr(loc, "mandates.period"), "mandate-kpi-period", |k| k.period.clone(), |k, v| k.period = v)}
                        <button type="button" class="calendar-icon" title=crate::i18n::tr(loc, "mandates.remove_kpi") aria-label=crate::i18n::tr(loc, "mandates.remove_kpi") on:click=move |_| form.update(|f| if i < f.kpis.len() { f.kpis.remove(i); })>{compose_icon("minus")}</button>
                    </div>}).collect_view()}
                    <button type="button" class="btn-ghost mandate-add-kpi" data-testid="mandate-add-kpi" on:click=move |_| form.update(|f| f.kpis.push(MandateKpi::default()))>{compose_icon("plus")}{crate::i18n::tr(loc, "mandates.add_kpi")}</button>
                </fieldset>
                <fieldset class="mandate-fieldset"><legend>{crate::i18n::tr(loc, "mandates.constraints")}</legend>
                    <div class="automation-form-grid">
                        <label class="automation-span">{crate::i18n::tr(loc, "mandates.may_do_on_its_own")}<textarea rows="2" prop:value=move || form.with(|f| f.autonomous.clone()) on:input=move |ev| form.update(|f| f.autonomous = event_target_value(&ev))></textarea></label>
                        <label class="automation-span">{crate::i18n::tr(loc, "mandates.needs_your_review_first")}<textarea rows="2" prop:value=move || form.with(|f| f.review_first.clone()) on:input=move |ev| form.update(|f| f.review_first = event_target_value(&ev))></textarea></label>
                        <label class="automation-span">{crate::i18n::tr(loc, "mandates.ask_for_help_when")}<textarea rows="2" prop:value=move || form.with(|f| f.ask_for_help.clone()) on:input=move |ev| form.update(|f| f.ask_for_help = event_target_value(&ev))></textarea></label>
                        <label class="automation-span">{crate::i18n::tr(loc, "mandates.standing_instructions_one_per_line")}<textarea rows="3" data-testid="mandate-notes" placeholder=crate::i18n::tr(loc, "mandates.how_the_work_should_go_from_now_on_feedback_you_confirm_in_the_ma") prop:value=move || form.with(|f| f.notes.clone()) on:input=move |ev| form.update(|f| f.notes = event_target_value(&ev))></textarea></label>
                        <label class="mandate-check automation-span"><input type="checkbox" data-testid="mandate-review-mutations" prop:checked=move || form.with(|f| f.review_mutations) on:change=move |ev| form.update(|f| f.review_mutations = event_target_checked(&ev))/>{crate::i18n::tr(loc, "mandates.ask_me_before_anything_that_changes_files_runs_commands_or_submit")}</label>
                    </div>
                </fieldset>
                <fieldset class="mandate-fieldset"><legend>{crate::i18n::tr(loc, "mandates.rhythm")}</legend>
                    <div class="automation-form-grid">
                        <label>{crate::i18n::tr(loc, "mandates.check_in_every_hours")}<input type="number" min="1" max="720" prop:value=move || form.with(|f| f.hours.to_string()) on:input=move |ev| form.update(|f| f.hours = event_target_value(&ev).parse().unwrap_or(24))/></label>
                        <label>{crate::i18n::tr(loc, "mandates.report_every_days")}<input type="number" min="1" max="90" prop:value=move || form.with(|f| f.report_days.to_string()) on:input=move |ev| form.update(|f| f.report_days = event_target_value(&ev).parse().unwrap_or(7))/></label>
                        <label>{crate::i18n::tr(loc, "mandates.soonest_next_round_minutes")}<input type="number" min="5" max="43200" prop:value=move || form.with(|f| f.min_minutes.to_string()) on:input=move |ev| form.update(|f| f.min_minutes = event_target_value(&ev).parse().unwrap_or(15))/></label>
                        <label>{crate::i18n::tr(loc, "mandates.latest_next_round_days")}<input type="number" min="1" max="30" prop:value=move || form.with(|f| f.max_days.to_string()) on:input=move |ev| form.update(|f| f.max_days = event_target_value(&ev).parse().unwrap_or(7))/></label>
                        <label>{crate::i18n::tr(loc, "mandates.rounds_per_day_at_most")}<input type="number" min="1" max="48" prop:value=move || form.with(|f| f.max_rounds.to_string()) on:input=move |ev| form.update(|f| f.max_rounds = event_target_value(&ev).parse().unwrap_or(6))/></label>
                    </div>
                </fieldset>
                <p class="automation-meta">{crate::i18n::tr(loc, "mandates.the_mandate_works_in_its_own_conversation_in_the_project_and_star")}</p>
                {move || form_error.get().map(|e| view! {<p class="automation-error" role="alert">{e}</p>})}
                <div class="automation-actions automation-form-actions">
                    <button type="button" class="btn-ghost" on:click=move |_| form_open.set(false)>{crate::i18n::tr(loc, "projects.cancel")}</button>
                    <button type="button" class="btn-primary" data-testid="mandate-save" on:click=save>{if editing {crate::i18n::tr(loc, "settings.save")} else {crate::i18n::tr(loc, "projects.create")}}</button>
                </div>
            </section>}
        })}
        <section class="automation-section" data-testid="mandate-section">
            <div class="mandate-section-head"><h3>{move || crate::i18n::tr(locale.get(), "mandates.research_mandates")}</h3>
                <button type="button" class="btn-ghost" data-testid="mandate-create" on:click=move |_| start(None)>{compose_icon("plus")}{move || crate::i18n::tr(locale.get(), "mandates.new_mandate")}</button>
            </div>
            <p class="automation-meta mandate-intro">{move || crate::i18n::tr(locale.get(), "mandates.hand_over_a_responsibility_instead_of_a_prompt_a_goal_how_it_is_m")}</p>
            {move || {
                let loc = locale.get();
                match mandates.get() {
                    None => view! {<p class="automation-meta" role="status">{crate::i18n::tr(loc, "privacy.loading")}</p>}.into_view(),
                    Some(Err(e)) => view! {<p class="automation-error" role="alert">{e}</p>}.into_view(),
                    Some(Ok(rows)) => {
                        // Privacy mode hides a project's mandates with the project.
                        let rows: Vec<_> = rows.into_iter().filter_map(|o| project_name(&o.mandate.project_id).map(|name| (o, name))).collect();
                        if rows.is_empty() {
                            return view! {<div class="automation-empty" data-testid="mandate-empty">{crate::i18n::tr(loc, "mandates.no_mandates_yet_start_from_a_template_below_or_define_your_own")}</div>}.into_view();
                        }
                        rows.into_iter().map(|(o, project)| {
                            let last_round = o.last_round;
                            let request = o.request;
                            let reports = o.reports;
                            let m = o.mandate;
                            let status = m.status.clone();
                            let running = matches!(status.as_str(), "active" | "waiting");
                            let when = match status.as_str() {
                                "active" => format!("{} {} {}", crate::i18n::tr(loc, "mandates.next_round"), day_key(m.next_run_at), clock(m.next_run_at)),
                                "waiting" => crate::i18n::tr(loc, "mandates.waiting_for_you").into(),
                                "paused" => crate::i18n::tr(loc, "runs.status.paused").into(),
                                _ => crate::i18n::tr(loc, "mandates.closed").into(),
                            };
                            let until = m.ends_at.map(|end| format!(" · {} {}", crate::i18n::tr(loc, "mandates.until_2"), day_key(end))).unwrap_or_default();
                            let (toggle_id, run_id, delete_id, confirm_id, remove_id) = (m.id.clone(), m.id.clone(), m.id.clone(), m.id.clone(), m.id.clone());
                            let report_id = m.id.clone();
                            let close_id = m.id.clone();
                            let notes = m.constraints.notes.clone();
                            let edit = m.clone();
                            let closed = status == "done";
                            view! {<article class="mandate-card" data-mandate-id=m.id.clone() data-status=status.clone() class:paused=!running>
                                <div class="mandate-card-head">{compose_icon("research-trail")}<strong>{m.name.clone()}</strong>
                                    <span class="mandate-status">{status_label(loc, &status)}</span>
                                    <div class="automation-row-actions">
                                        <label class="automation-toggle"><input type="checkbox" role="switch" aria-label=format!("{} {}", crate::i18n::tr(loc, "automation.enable"), m.name) prop:checked=running on:change=move |ev| act("set_mandate_status", serde_json::json!({"id":toggle_id,"status": if event_target_checked(&ev) {"active"} else {"paused"}}))/></label>
                                        <button type="button" class="calendar-icon" prop:disabled=closed title=crate::i18n::tr(loc, "mandates.run_a_round_now") aria-label=format!("{} {}", crate::i18n::tr(loc, "mandates.run_a_round_now"), m.name) on:click=move |_| { act("run_mandate_now", serde_json::json!({"id":run_id})); notice.set(Some(crate::i18n::tr(locale.get_untracked(), "mandates.started_a_round_it_runs_in_the_mandate_s_conversation_in_that_pro").into())); }>{compose_icon("play")}</button>
                                        <button type="button" class="calendar-icon" prop:disabled=closed title=crate::i18n::tr(loc, "mandates.write_a_report_now") aria-label=format!("{} {}", crate::i18n::tr(loc, "mandates.write_a_report_now"), m.name) on:click=move |_| { act("report_mandate_now", serde_json::json!({"id":report_id})); notice.set(Some(crate::i18n::tr(locale.get_untracked(), "mandates.writing_the_report_it_appears_on_the_card_when_it_is_ready").into())); }>{compose_icon("clipboard")}</button>
                                        <button type="button" class="calendar-icon" prop:disabled=closed title=crate::i18n::tr(loc, "mandates.close_mandate") aria-label=format!("{} {}", crate::i18n::tr(loc, "mandates.close_mandate"), m.name) on:click=move |_| { act("set_mandate_status", serde_json::json!({"id":close_id,"status":"done"})); notice.set(Some(crate::i18n::tr(locale.get_untracked(), "mandates.closed_a_final_report_is_being_written_switch_it_back_on_to_reope").into())); }>{compose_icon("circle-check")}</button>
                                        <button type="button" class="calendar-icon" title=crate::i18n::tr(loc, "menu.edit") aria-label=format!("{} {}", crate::i18n::tr(loc, "menu.edit"), m.name) on:click=move |_| open_form(form_from(&edit))>{compose_icon("edit")}</button>
                                        <button type="button" class="calendar-icon automation-delete" class:confirming=move || deleting.get().as_ref() == Some(&confirm_id)
                                            title=move || if deleting.get().as_ref() == Some(&delete_id) {crate::i18n::tr(locale.get(), "automation.click_again_to_delete")} else {crate::i18n::tr(locale.get(), "projects.delete")}
                                            aria-label=format!("{} {}", crate::i18n::tr(loc, "projects.delete"), m.name)
                                            on:click=move |_| if deleting.get_untracked().as_ref() == Some(&remove_id) { deleting.set(None); act("delete_mandate", serde_json::json!({"id":remove_id.clone()})); } else { deleting.set(Some(remove_id.clone())); }>{compose_icon("trash")}</button>
                                    </div>
                                </div>
                                <small>{format!("{project} · {when}{until}")}</small>
                                <p class="automation-prompt">{m.goal.clone()}</p>
                                {(!notes.is_empty()).then(|| view! {<ul class="mandate-notes" data-testid="mandate-notes-list" aria-label=crate::i18n::tr(loc, "mandates.standing_instructions")>
                                    {notes.iter().map(|note| view! {<li>{note.clone()}</li>}).collect_view()}
                                </ul>})}
                                {(!m.kpis.is_empty()).then(|| view! {<ul class="mandate-kpis">
                                    {m.kpis.iter().map(|k| {
                                        let current = k.current.clone().unwrap_or_else(|| "–".into());
                                        let value = if k.target.is_empty() { current } else { format!("{current} / {}", k.target) };
                                        view! {<li title=k.definition.clone()><span>{k.name.clone()}</span><strong>{value}</strong><small>{k.period.clone()}</small></li>}
                                    }).collect_view()}
                                </ul>})}
                                {last_round.map(|r| {
                                    let at = format!("{} {}", day_key(r.created_at), clock(r.created_at));
                                    let title = crate::i18n::tf(loc, "mandates.round_title", &[("seq", &r.seq.to_string()), ("at", &at)]);
                                    view! {<div class="mandate-round" data-testid="mandate-last-round">
                                        <small>{title}{(r.source == "host").then(|| crate::i18n::tr(loc, "mandates.ended_without_a_report"))}</small>
                                        <p><b>{crate::i18n::tr(loc, "mandates.done")}</b>{r.done}</p>
                                        {(!r.blockers.is_empty()).then(|| view! {<p class="mandate-round-blocked"><b>{crate::i18n::tr(loc, "mandates.blocked")}</b>{r.blockers}</p>})}
                                        {(!r.next_step.is_empty()).then(|| view! {<p><b>{crate::i18n::tr(loc, "onboard.next")}</b>{r.next_step}</p>})}
                                    </div>}
                                })}
                                {request.map(|r| {
                                    let reply = create_rw_signal(String::new());
                                    let (reply_id, name) = (m.id.clone(), m.name.clone());
                                    let asked = format!("{} {}", day_key(r.created_at), clock(r.created_at));
                                    let send = move |_| {
                                        let text = reply.get_untracked();
                                        if text.trim().is_empty() { return; }
                                        act("reply_to_mandate", serde_json::json!({"id": reply_id.clone(), "text": text}));
                                        notice.set(Some(crate::i18n::tr(locale.get_untracked(), "mandates.sent_the_mandate_continues_in_its_conversation").into()));
                                    };
                                    view! {<div class="mandate-request" data-testid="mandate-request">
                                        <small>{format!("{} · {} {asked}", request_kind_label(loc, &r.kind), crate::i18n::tr(loc, "mandates.asked"))}</small>
                                        <p><b>{crate::i18n::tr(loc, "mandates.needs")}</b>{r.what}</p>
                                        {(!r.why.is_empty()).then(|| view! {<p><b>{crate::i18n::tr(loc, "mandates.why")}</b>{r.why}</p>})}
                                        {(!r.then_what.is_empty()).then(|| view! {<p><b>{crate::i18n::tr(loc, "mandates.then")}</b>{r.then_what}</p>})}
                                        <div class="mandate-reply">
                                            <textarea rows="2" data-testid="mandate-reply-text" aria-label=format!("{} {name}", crate::i18n::tr(loc, "mandates.reply_to")) placeholder=crate::i18n::tr(loc, "mandates.what_you_did_decided_or_provided_your_reply_starts_the_next_round") prop:value=move || reply.get() on:input=move |ev| reply.set(event_target_value(&ev))></textarea>
                                            <button type="button" class="btn-primary" data-testid="mandate-reply-send" prop:disabled=move || reply.with(|text| text.trim().is_empty()) on:click=send>{crate::i18n::tr(loc, "trajectory.head.assistant")}</button>
                                        </div>
                                    </div>}
                                })}
                                {(!reports.is_empty()).then(|| {
                                    let latest = reports.first().map(|r| day_key(r.period_until)).unwrap_or_default();
                                    let count = reports.len();
                                    let title = crate::i18n::tf(loc, "mandates.reports_title", &[("count", &count.to_string()), ("latest", &latest)]);
                                    view! {<details class="mandate-reports" data-testid="mandate-reports">
                                        <summary>{title}</summary>
                                        {reports.into_iter().map(|r| report_view(loc, r)).collect_view()}
                                    </details>}
                                })}
                            </article>}
                        }).collect_view()
                    }
                }
            }}
            <p class="automation-meta mandate-intro">{move || crate::i18n::tr(locale.get(), "mandates.start_from_a_mandate_template")}</p>
            <div class="automation-templates">
                {TEMPLATES.iter().map(|t| view! {
                    <button type="button" class="mandate-template" on:click=move |_| start(Some(t))>
                        <span class="automation-template-head">{compose_icon(t.icon)}<strong>{move || pick(locale.get(), t.title)}</strong></span>
                        <span class="automation-template-desc">{move || pick(locale.get(), t.goal)}</span>
                        <small>{move || { let loc = locale.get(); crate::i18n::tf(loc, "mandates.template_cadence", &[("hours", &t.hours.to_string()), ("days", &t.report_days.to_string())]) }}</small>
                    </button>
                }).collect_view()}
            </div>
        </section>
    }
}
