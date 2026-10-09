//! Research mandates on the Automation page: a long-running responsibility
//! defined by a goal, KPIs and constraints, carried round by round in its own
//! project conversation. Rounds run only while Wisp is open.
use crate::app_support::compose_icon;
use crate::dto::{
    MandateConstraints, MandateDraft, MandateKpi, MandateOverview, MandateRecord, MandateReport,
    ProjectSummary, ResearchRecapItem,
};
use crate::i18n::Locale;
use crate::research_journey::{call, clock, day_key, j, now};
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
    /// Confirmed standing instructions ride along untouched.
    notes: Vec<String>,
}

struct Template {
    icon: &'static str,
    title: (&'static str, &'static str),
    goal: (&'static str, &'static str),
    /// name, definition, target, period — each (en, zh).
    kpis: &'static [[(&'static str, &'static str); 4]],
    autonomous: (&'static str, &'static str),
    review_first: (&'static str, &'static str),
    ask_for_help: (&'static str, &'static str),
    review_mutations: bool,
    hours: i64,
    report_days: i64,
    days: i64,
}

static TEMPLATES: [Template; 4] = [
    Template {
        icon: "book",
        title: ("Literature watch", "文献追踪"),
        goal: (
            "For the next 12 months, keep this project's literature current: find newly published work that bears on its research question, screen it, file what matters, and tell me what changes our plans. Report weekly.",
            "未来 12 个月，持续跟进本项目相关文献：发现与研究问题相关的新工作，筛选并入库，告诉我哪些会影响我们的计划。每周向我汇报。",
        ),
        kpis: &[[
            ("Papers screened and filed", "筛选并入库的文献"),
            ("Read at abstract level or deeper, judged relevant, and saved to the project with a one-line reason", "至少读过摘要、判断相关、保存到项目并附一句理由的文献数"),
            ("5", "5"),
            ("per week", "每周"),
        ]],
        autonomous: ("Search literature databases, read papers, and save references and notes to the project.", "检索文献数据库、阅读论文、把文献和笔记保存到项目。"),
        review_first: ("Changing the project's analysis plan or conclusions because of a paper.", "因某篇文献而修改项目的分析计划或结论。"),
        ask_for_help: ("A paper needs an institutional login, or a finding contradicts our approach and needs your judgement.", "文献需要机构登录才能获取，或有发现与我们的方案冲突、需要你判断。"),
        review_mutations: false,
        hours: 24,
        report_days: 7,
        days: 365,
    },
    Template {
        icon: "doc",
        title: ("Submission follow-up", "投稿与返修跟进"),
        goal: (
            "Follow this manuscript from submission to acceptance: track the journal's status, prepare point-by-point responses and a revision checklist when reviews arrive, and keep every deadline in view. Report weekly and whenever the status changes.",
            "跟进这篇稿件从投稿到接收：跟踪期刊状态，审稿意见返回后准备逐条回复和返修清单，盯住每一个截止日期。每周汇报，状态变化时立即汇报。",
        ),
        kpis: &[
            [
                ("Open reviewer points", "未回应的审稿意见"),
                ("Reviewer comments that have no drafted response yet", "尚未起草回复的审稿意见条数"),
                ("0", "0"),
                ("before the revision deadline", "返修截止前"),
            ],
            [
                ("Revision checklist done (%)", "返修清单完成度（%）"),
                ("Checklist items finished and checked against the manuscript", "已完成并对照稿件核对过的清单条目占比"),
                ("100", "100"),
                ("before the revision deadline", "返修截止前"),
            ],
        ],
        autonomous: ("Read the manuscript and reviews, draft responses and checklists, and check the journal's status page.", "阅读稿件和审稿意见，起草回复与清单，查看期刊状态页。"),
        review_first: ("Any text sent to editors or reviewers, and any change to the manuscript.", "发给编辑或审稿人的任何文字，以及对稿件的任何修改。"),
        ask_for_help: ("Logging in to the submission system, confirming authorship or payment, or deciding how to answer a contested point.", "登录投稿系统、确认作者身份或付款，或决定如何回应有争议的意见。"),
        review_mutations: true,
        hours: 72,
        report_days: 7,
        days: 180,
    },
    Template {
        icon: "server",
        title: ("Long-running compute", "长期计算任务"),
        goal: (
            "See this computation through to verified results: submit and monitor the runs, diagnose and resubmit failures, harvest the outputs, and confirm they pass the acceptance checks. Report daily while runs are active.",
            "把这批计算跑到得到经过核验的结果：提交并监控运行，诊断并重提失败任务，取回产出，确认通过验收检查。运行期间每天汇报。",
        ),
        kpis: &[[
            ("Samples completed", "完成的样本数"),
            ("Samples whose run succeeded and whose outputs were harvested and checked", "运行成功、产出已取回并核验的样本数"),
            ("12", "12"),
            ("by the end date", "截止日期前"),
        ]],
        autonomous: ("Submit, monitor, cancel and resubmit runs on the configured servers; harvest outputs.", "在已配置的服务器上提交、监控、取消和重提运行；取回产出。"),
        review_first: ("Changing analysis parameters, deleting data, or using more compute than agreed.", "修改分析参数、删除数据，或使用超出约定的算力。"),
        ask_for_help: ("A server needs credentials or quota, or a failure repeats after two different fixes.", "服务器需要凭据或配额，或同一失败在两种修复后仍然重复。"),
        review_mutations: false,
        hours: 6,
        report_days: 1,
        days: 30,
    },
    Template {
        icon: "gauge",
        title: ("Method metric iteration", "方法指标迭代"),
        goal: (
            "Over the next 3 months, improve this method against its benchmark: measure the baseline, find what limits it, try one change at a time, and keep only changes that hold up on held-out data. Report weekly with the current metric.",
            "未来 3 个月，围绕基准持续改进这个方法：测定基线，找出限制因素，每次只改一处，只保留在留出数据上站得住的改动。每周汇报当前指标。",
        ),
        kpis: &[[
            ("Benchmark metric", "基准指标"),
            ("The primary metric on the held-out benchmark, measured with the fixed evaluation script", "用固定评测脚本在留出基准上测得的主要指标"),
            ("0.90", "0.90"),
            ("by the end date", "截止日期前"),
        ]],
        autonomous: ("Run evaluations and experiments, analyze results, and prepare code changes for review.", "运行评测和实验，分析结果，准备可供审阅的代码改动。"),
        review_first: ("Merging code, or changing the benchmark or the evaluation script.", "合并代码，或修改基准与评测脚本。"),
        ask_for_help: ("The metric stalls for three rounds, or a result looks too good to explain.", "指标连续三轮停滞，或结果好得无法解释。"),
        review_mutations: true,
        hours: 24,
        report_days: 7,
        days: 90,
    },
];

fn pick(loc: Locale, text: (&'static str, &'static str)) -> &'static str {
    j(loc, text.0, text.1)
}

fn request_kind_label(loc: Locale, kind: &str) -> &'static str {
    match kind {
        "login" => j(loc, "Sign-in", "登录"),
        "materials" => j(loc, "Materials", "补充资料"),
        "payment" => j(loc, "Payment", "付款授权"),
        "release" => j(loc, "Release", "发布"),
        _ => j(loc, "Your judgement", "需要你判断"),
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
    let rounds = if loc == Locale::Zh {
        format!("{} 轮", report.rounds)
    } else {
        format!("{} round(s)", report.rounds)
    };
    // A period with rounds but no model was assembled from the ledger.
    let ledger = (report.rounds > 0 && report.body.model.is_empty())
        .then(|| j(loc, " · from the ledger", " · 由账本整理"));
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
        {section.clone()(j(loc,"Done","已完成"), report.body.done)}
        {section.clone()(j(loc,"Progress","进展"), report.body.findings)}
        {section.clone()(j(loc,"Blocked","阻塞"), report.body.issues)}
        {section(j(loc,"Next","下一步"), report.body.next)}
    </article>}
}

fn status_label(loc: Locale, status: &str) -> &'static str {
    match status {
        "paused" => j(loc, "Paused", "已暂停"),
        "waiting" => j(loc, "Needs you", "等你处理"),
        "done" => j(loc, "Closed", "已结束"),
        _ => j(loc, "Active", "进行中"),
    }
}

/// Local end of day for a `YYYY-MM-DD` date input.
fn end_of_day(value: &str) -> Option<i64> {
    let mut parts = value.split('-').map(|part| part.parse::<i32>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    let ts = js_sys::Date::new_with_year_month_day_hr_min_sec(y as u32, m - 1, d, 23, 59, 59)
        .get_time();
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
        notes: c.notes.clone(),
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
            notes: form.notes.clone(),
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
                j(loc, "Choose a project and write a goal.", "请选择项目并填写目标。").into(),
            ));
            return;
        }
        if !f.end_date.is_empty() && end_of_day(&f.end_date).is_none_or(|end| end <= now()) {
            form_error.set(Some(
                j(loc, "Choose an end date in the future, or leave it empty.", "请选择将来的结束日期，或留空。").into(),
            ));
            return;
        }
        let draft = serde_json::to_value(draft_from(&f)).unwrap_or_default();
        let (command, args) = match &f.id {
            Some(id) => ("update_mandate", serde_json::json!({"id": id, "draft": draft})),
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
    let kpi_input = move |i: usize, label: &'static str, class: &'static str, get: fn(&MandateKpi) -> String, set: fn(&mut MandateKpi, String)| {
        view! {<input type="text" class=class aria-label=label placeholder=label
            prop:value=move || form.with(|f| f.kpis.get(i).map(get).unwrap_or_default())
            on:input=move |ev| form.update(|f| if let Some(k) = f.kpis.get_mut(i) { set(k, event_target_value(&ev)) })/>}
    };
    view! {
        {move || (form_open.get() && mandate_form.get()).then(|| {
            let loc = locale.get();
            let editing = form.with_untracked(|f| f.id.is_some());
            let title = if editing { j(loc,"Edit mandate","编辑职责") } else { j(loc,"New mandate","新建职责") };
            view! {<section class="automation-card automation-form mandate-form" data-testid="mandate-form" aria-label=title>
                <h3>{title}</h3>
                <div class="automation-form-grid">
                    <label>{j(loc,"Project","项目")}<select prop:disabled=editing on:change=move |ev| form.update(|f| f.project_id = event_target_value(&ev))>
                        {projects.get().into_iter().map(|p| { let id = p.id.clone(); view! {<option value=p.id prop:selected=move || form.with(|f| f.project_id == id)>{p.name}</option>} }).collect_view()}
                    </select></label>
                    <label>{j(loc,"Name","名称")}<input type="text" prop:value=move || form.with(|f| f.name.clone()) on:input=move |ev| form.update(|f| f.name = event_target_value(&ev))/></label>
                    <label>{j(loc,"Until","截止日期")}<input type="date" prop:value=move || form.with(|f| f.end_date.clone()) on:input=move |ev| form.update(|f| f.end_date = event_target_value(&ev))/></label>
                    <label class="automation-span">{j(loc,"Goal","目标")}<textarea rows="4" placeholder=j(loc,"Who is this for, what should be achieved, and for how long?","为谁工作、希望达成什么结果、持续多久？") prop:value=move || form.with(|f| f.goal.clone()) on:input=move |ev| form.update(|f| f.goal = event_target_value(&ev))></textarea></label>
                </div>
                <fieldset class="mandate-fieldset"><legend>{j(loc,"KPIs","KPI")}</legend>
                    <p class="automation-meta">{j(loc,"How progress is measured: say what is counted, the target, and the period it applies to.","怎样衡量进展：写清统计口径、目标值和统计周期。")}</p>
                    {move || (0..kpi_rows.get()).map(|i| view! {<div class="mandate-kpi-row" data-testid="mandate-kpi-row">
                        {kpi_input(i, j(loc,"KPI name","指标名称"), "mandate-kpi-name", |k| k.name.clone(), |k, v| k.name = v)}
                        {kpi_input(i, j(loc,"How it is counted","统计口径"), "mandate-kpi-definition", |k| k.definition.clone(), |k, v| k.definition = v)}
                        {kpi_input(i, j(loc,"Target","目标值"), "mandate-kpi-target", |k| k.target.clone(), |k, v| k.target = v)}
                        {kpi_input(i, j(loc,"Period","统计周期"), "mandate-kpi-period", |k| k.period.clone(), |k, v| k.period = v)}
                        <button type="button" class="calendar-icon" title=j(loc,"Remove KPI","移除指标") aria-label=j(loc,"Remove KPI","移除指标") on:click=move |_| form.update(|f| if i < f.kpis.len() { f.kpis.remove(i); })>{compose_icon("minus")}</button>
                    </div>}).collect_view()}
                    <button type="button" class="btn-ghost mandate-add-kpi" data-testid="mandate-add-kpi" on:click=move |_| form.update(|f| f.kpis.push(MandateKpi::default()))>{compose_icon("plus")}{j(loc,"Add KPI","添加指标")}</button>
                </fieldset>
                <fieldset class="mandate-fieldset"><legend>{j(loc,"Constraints","工作约束")}</legend>
                    <div class="automation-form-grid">
                        <label class="automation-span">{j(loc,"May do on its own","可自行完成")}<textarea rows="2" prop:value=move || form.with(|f| f.autonomous.clone()) on:input=move |ev| form.update(|f| f.autonomous = event_target_value(&ev))></textarea></label>
                        <label class="automation-span">{j(loc,"Needs your review first","需要先审核")}<textarea rows="2" prop:value=move || form.with(|f| f.review_first.clone()) on:input=move |ev| form.update(|f| f.review_first = event_target_value(&ev))></textarea></label>
                        <label class="automation-span">{j(loc,"Ask for help when","何时请求协助")}<textarea rows="2" prop:value=move || form.with(|f| f.ask_for_help.clone()) on:input=move |ev| form.update(|f| f.ask_for_help = event_target_value(&ev))></textarea></label>
                        <label class="mandate-check automation-span"><input type="checkbox" data-testid="mandate-review-mutations" prop:checked=move || form.with(|f| f.review_mutations) on:change=move |ev| form.update(|f| f.review_mutations = event_target_checked(&ev))/>{j(loc,"Ask me before anything that changes files, runs commands or submits work","修改文件、运行命令或提交任何内容之前先问我")}</label>
                    </div>
                </fieldset>
                <fieldset class="mandate-fieldset"><legend>{j(loc,"Rhythm","工作节奏")}</legend>
                    <div class="automation-form-grid">
                        <label>{j(loc,"Check in every (hours)","默认每隔（小时）")}<input type="number" min="1" max="720" prop:value=move || form.with(|f| f.hours.to_string()) on:input=move |ev| form.update(|f| f.hours = event_target_value(&ev).parse().unwrap_or(24))/></label>
                        <label>{j(loc,"Report every (days)","汇报间隔（天）")}<input type="number" min="1" max="90" prop:value=move || form.with(|f| f.report_days.to_string()) on:input=move |ev| form.update(|f| f.report_days = event_target_value(&ev).parse().unwrap_or(7))/></label>
                        <label>{j(loc,"Soonest next round (minutes)","最短间隔（分钟）")}<input type="number" min="5" max="43200" prop:value=move || form.with(|f| f.min_minutes.to_string()) on:input=move |ev| form.update(|f| f.min_minutes = event_target_value(&ev).parse().unwrap_or(15))/></label>
                        <label>{j(loc,"Latest next round (days)","最长间隔（天）")}<input type="number" min="1" max="30" prop:value=move || form.with(|f| f.max_days.to_string()) on:input=move |ev| form.update(|f| f.max_days = event_target_value(&ev).parse().unwrap_or(7))/></label>
                        <label>{j(loc,"Rounds per day at most","每天最多回合")}<input type="number" min="1" max="48" prop:value=move || form.with(|f| f.max_rounds.to_string()) on:input=move |ev| form.update(|f| f.max_rounds = event_target_value(&ev).parse().unwrap_or(6))/></label>
                    </div>
                </fieldset>
                <p class="automation-meta">{j(loc,"The mandate works in its own conversation in the project and starts its first round right away. Rounds run only while Wisp is open; a missed round runs once on the next launch.","职责在所选项目中使用自己的一条对话，创建后立即开始第一轮。仅在 Wisp 运行时工作，错过的回合会在下次启动时补跑一次。")}</p>
                {move || form_error.get().map(|e| view! {<p class="automation-error" role="alert">{e}</p>})}
                <div class="automation-actions automation-form-actions">
                    <button type="button" class="btn-ghost" on:click=move |_| form_open.set(false)>{j(loc,"Cancel","取消")}</button>
                    <button type="button" class="btn-primary" data-testid="mandate-save" on:click=save>{if editing {j(loc,"Save","保存")} else {j(loc,"Create","创建")}}</button>
                </div>
            </section>}
        })}
        <section class="automation-section" data-testid="mandate-section">
            <div class="mandate-section-head"><h3>{move || j(locale.get(),"Research mandates","研究职责")}</h3>
                <button type="button" class="btn-ghost" data-testid="mandate-create" on:click=move |_| start(None)>{compose_icon("plus")}{move || j(locale.get(),"New mandate","新建职责")}</button>
            </div>
            <p class="automation-meta mandate-intro">{move || j(locale.get(),"Hand over a responsibility instead of a prompt: a goal, how it is measured, and what may be done without asking. The agent carries it round by round and reports back.","交出一份职责而不是一段提示词：目标、衡量方式，以及哪些事可以不问就做。Agent 一轮轮推进，并向你汇报。")}</p>
            {move || {
                let loc = locale.get();
                match mandates.get() {
                    None => view! {<p class="automation-meta" role="status">{j(loc,"Loading…","正在读取…")}</p>}.into_view(),
                    Some(Err(e)) => view! {<p class="automation-error" role="alert">{e}</p>}.into_view(),
                    Some(Ok(rows)) => {
                        // Privacy mode hides a project's mandates with the project.
                        let rows: Vec<_> = rows.into_iter().filter_map(|o| project_name(&o.mandate.project_id).map(|name| (o, name))).collect();
                        if rows.is_empty() {
                            return view! {<div class="automation-empty" data-testid="mandate-empty">{j(loc,"No mandates yet. Start from a template below or define your own.","还没有职责。可以从下方模板开始，或自己定义一份。")}</div>}.into_view();
                        }
                        rows.into_iter().map(|(o, project)| {
                            let last_round = o.last_round;
                            let request = o.request;
                            let reports = o.reports;
                            let m = o.mandate;
                            let status = m.status.clone();
                            let running = matches!(status.as_str(), "active" | "waiting");
                            let when = match status.as_str() {
                                "active" => format!("{} {} {}", j(loc,"Next round","下一轮"), day_key(m.next_run_at), clock(m.next_run_at)),
                                "waiting" => j(loc,"Waiting for you","等待你的处理").into(),
                                "paused" => j(loc,"Paused","已暂停").into(),
                                _ => j(loc,"Closed","已结束").into(),
                            };
                            let until = m.ends_at.map(|end| format!(" · {} {}", j(loc,"until","截至"), day_key(end))).unwrap_or_default();
                            let (toggle_id, run_id, delete_id, confirm_id, remove_id) = (m.id.clone(), m.id.clone(), m.id.clone(), m.id.clone(), m.id.clone());
                            let report_id = m.id.clone();
                            let edit = m.clone();
                            let closed = status == "done";
                            view! {<article class="mandate-card" data-mandate-id=m.id.clone() data-status=status.clone() class:paused=!running>
                                <div class="mandate-card-head">{compose_icon("research-trail")}<strong>{m.name.clone()}</strong>
                                    <span class="mandate-status">{status_label(loc, &status)}</span>
                                    <div class="automation-row-actions">
                                        <label class="automation-toggle"><input type="checkbox" role="switch" aria-label=format!("{} {}", j(loc,"Enable","启用"), m.name) prop:checked=running on:change=move |ev| act("set_mandate_status", serde_json::json!({"id":toggle_id,"status": if event_target_checked(&ev) {"active"} else {"paused"}}))/></label>
                                        <button type="button" class="calendar-icon" prop:disabled=closed title=j(loc,"Run a round now","立即运行一轮") aria-label=format!("{} {}", j(loc,"Run a round now","立即运行一轮"), m.name) on:click=move |_| { act("run_mandate_now", serde_json::json!({"id":run_id})); notice.set(Some(j(locale.get_untracked(),"Started a round. It runs in the mandate's conversation in that project.","已开始一轮，在该项目的职责对话中运行。").into())); }>{compose_icon("play")}</button>
                                        <button type="button" class="calendar-icon" prop:disabled=closed title=j(loc,"Write a report now","立即生成汇报") aria-label=format!("{} {}", j(loc,"Write a report now","立即生成汇报"), m.name) on:click=move |_| { act("report_mandate_now", serde_json::json!({"id":report_id})); notice.set(Some(j(locale.get_untracked(),"Writing the report. It appears on the card when it is ready.","正在生成汇报，完成后显示在卡片上。").into())); }>{compose_icon("clipboard")}</button>
                                        <button type="button" class="calendar-icon" title=j(loc,"Edit","编辑") aria-label=format!("{} {}", j(loc,"Edit","编辑"), m.name) on:click=move |_| open_form(form_from(&edit))>{compose_icon("edit")}</button>
                                        <button type="button" class="calendar-icon automation-delete" class:confirming=move || deleting.get().as_ref() == Some(&confirm_id)
                                            title=move || if deleting.get().as_ref() == Some(&delete_id) {j(locale.get(),"Click again to delete","再次点击确认删除")} else {j(locale.get(),"Delete","删除")}
                                            aria-label=format!("{} {}", j(loc,"Delete","删除"), m.name)
                                            on:click=move |_| if deleting.get_untracked().as_ref() == Some(&remove_id) { deleting.set(None); act("delete_mandate", serde_json::json!({"id":remove_id.clone()})); } else { deleting.set(Some(remove_id.clone())); }>{compose_icon("trash")}</button>
                                    </div>
                                </div>
                                <small>{format!("{project} · {when}{until}")}</small>
                                <p class="automation-prompt">{m.goal.clone()}</p>
                                {(!m.kpis.is_empty()).then(|| view! {<ul class="mandate-kpis">
                                    {m.kpis.iter().map(|k| {
                                        let current = k.current.clone().unwrap_or_else(|| "–".into());
                                        let value = if k.target.is_empty() { current } else { format!("{current} / {}", k.target) };
                                        view! {<li title=k.definition.clone()><span>{k.name.clone()}</span><strong>{value}</strong><small>{k.period.clone()}</small></li>}
                                    }).collect_view()}
                                </ul>})}
                                {last_round.map(|r| {
                                    let at = format!("{} {}", day_key(r.created_at), clock(r.created_at));
                                    let title = if loc == Locale::Zh { format!("第 {} 轮 · {at}", r.seq) } else { format!("Round {} · {at}", r.seq) };
                                    view! {<div class="mandate-round" data-testid="mandate-last-round">
                                        <small>{title}{(r.source == "host").then(|| j(loc," · ended without a report"," · 未提交回合报告"))}</small>
                                        <p><b>{j(loc,"Done","已完成")}</b>{r.done}</p>
                                        {(!r.blockers.is_empty()).then(|| view! {<p class="mandate-round-blocked"><b>{j(loc,"Blocked","阻塞")}</b>{r.blockers}</p>})}
                                        {(!r.next_step.is_empty()).then(|| view! {<p><b>{j(loc,"Next","下一步")}</b>{r.next_step}</p>})}
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
                                        notice.set(Some(j(locale.get_untracked(),"Sent. The mandate continues in its conversation.","已发送，职责在它的对话中继续。").into()));
                                    };
                                    view! {<div class="mandate-request" data-testid="mandate-request">
                                        <small>{format!("{} · {} {asked}", request_kind_label(loc, &r.kind), j(loc,"asked","提出于"))}</small>
                                        <p><b>{j(loc,"Needs","需要")}</b>{r.what}</p>
                                        {(!r.why.is_empty()).then(|| view! {<p><b>{j(loc,"Why","原因")}</b>{r.why}</p>})}
                                        {(!r.then_what.is_empty()).then(|| view! {<p><b>{j(loc,"Then","之后")}</b>{r.then_what}</p>})}
                                        <div class="mandate-reply">
                                            <textarea rows="2" data-testid="mandate-reply-text" aria-label=format!("{} {name}", j(loc,"Reply to","回复")) placeholder=j(loc,"What you did, decided or provided. Your reply starts the next round.","你做了什么、决定了什么或提供了什么。回复后立即开始下一轮。") prop:value=move || reply.get() on:input=move |ev| reply.set(event_target_value(&ev))></textarea>
                                            <button type="button" class="btn-primary" data-testid="mandate-reply-send" prop:disabled=move || reply.with(|text| text.trim().is_empty()) on:click=send>{j(loc,"Reply","回复")}</button>
                                        </div>
                                    </div>}
                                })}
                                {(!reports.is_empty()).then(|| {
                                    let latest = reports.first().map(|r| day_key(r.period_until)).unwrap_or_default();
                                    let count = reports.len();
                                    let title = if loc == Locale::Zh { format!("汇报（{count}）· 最近 {latest}") } else { format!("Reports ({count}) · latest {latest}") };
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
            <p class="automation-meta mandate-intro">{move || j(locale.get(),"Start from a mandate template:","从职责模板开始：")}</p>
            <div class="automation-templates">
                {TEMPLATES.iter().map(|t| view! {
                    <button type="button" class="mandate-template" on:click=move |_| start(Some(t))>
                        <span class="automation-template-head">{compose_icon(t.icon)}<strong>{move || pick(locale.get(), t.title)}</strong></span>
                        <span class="automation-template-desc">{move || pick(locale.get(), t.goal)}</span>
                        <small>{move || { let loc = locale.get(); if loc == Locale::Zh { format!("默认每 {} 小时 · 每 {} 天汇报", t.hours, t.report_days) } else { format!("Every {} h · report every {} d", t.hours, t.report_days) } }}</small>
                    </button>
                }).collect_view()}
            </div>
        </section>
    }
}
