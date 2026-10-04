//! Automation page, opened from the research assistant because it spans
//! projects: the built-in daily recap plus every project's scheduled prompts,
//! created from a template or from scratch. Schedules fire only while Wisp
//! runs; a missed slot runs once on the next launch.
use crate::app_support::compose_icon;
use crate::dto::{DailyRecapAutomation, ProjectSummary, ScheduleRecord};
use crate::i18n::Locale;
use crate::research_journey::{call, clock, date, day_key, j, now};
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
    title: (&'static str, &'static str),
    prompt: (&'static str, &'static str),
    cadence: Cadence,
    time: &'static str,
    weekday: u32,
}

static TEMPLATES: [Template; 3] = [
    Template {
        icon: "book",
        title: ("Literature watch", "文献追踪"),
        prompt: (
            "Search for papers published in the past 7 days that bear on this project's research question. List at most 10, most relevant first, each with one sentence on how it relates to this project, and state the search queries you used. Read-only: do not change project files.",
            "检索过去 7 天内与本项目研究问题相关的新文献，按相关性列出最多 10 篇，每篇用一句话说明与本项目的关系，并注明所用检索式。只读，不修改项目文件。",
        ),
        cadence: Cadence::Weekly,
        time: "09:00",
        weekday: 1,
    },
    Template {
        icon: "gauge",
        title: ("Run check", "运行巡检"),
        prompt: (
            "Check this project's runs. List runs that are still running, failed, lost their connection, or finished without their outputs harvested, with the likely reason and a suggested next step for each. Read-only: do not resubmit or cancel runs.",
            "检查本项目的所有运行：列出仍在运行、失败、失去连接或已完成但尚未收取产出的任务，说明可能原因和建议的下一步。只读，不重新提交或取消任务。",
        ),
        cadence: Cadence::Daily,
        time: "18:00",
        weekday: 1,
    },
    Template {
        icon: "doc",
        title: ("Weekly report", "研究周报"),
        prompt: (
            "Summarize this week's research journey for this project, including its daily recaps, into a short weekly report: completed work, key findings and decisions, problems met, and next week's plan. Cite runs and outputs by name. Read-only.",
            "汇总本项目本周的研究历程（含每日回顾），整理成一份简短周报：本周完成、关键发现与决定、遇到的问题、下周计划，并注明相关运行和产出。只读。",
        ),
        cadence: Cadence::Weekly,
        time: "17:00",
        weekday: 5,
    },
];

fn weekday_name(loc: Locale, day: u32) -> &'static str {
    let en = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let zh = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
    let day = (day % 7) as usize;
    if loc == Locale::Zh {
        zh[day]
    } else {
        en[day]
    }
}

/// Human cadence from the interval and the next slot's local wall clock.
fn cadence_label(loc: Locale, schedule: &ScheduleRecord) -> String {
    let at = clock(schedule.next_run_at);
    match schedule.interval_secs {
        DAY => format!("{} {at}", j(loc, "Daily", "每天")),
        WEEK => format!(
            "{} {at}",
            weekday_name(loc, date(schedule.next_run_at).get_day())
        ),
        n if n % 3600 == 0 => {
            let h = n / 3600;
            if loc == Locale::Zh {
                format!("每 {h} 小时")
            } else {
                format!("Every {h} h")
            }
        }
        n => {
            let m = n / 60;
            if loc == Locale::Zh {
                format!("每 {m} 分钟")
            } else {
                format!("Every {m} min")
            }
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
        });
    });
    // While a recap run is in flight, look again shortly.
    create_effect(move |_| {
        if daily.get().is_some_and(|d| d.running) {
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
            next.name = j(loc, t.title.0, t.title.1).into();
            next.prompt = j(loc, t.prompt.0, t.prompt.1).into();
            next.cadence = t.cadence;
            next.time = t.time.into();
            next.weekday = t.weekday;
        }
        draft.set(next);
        form_error.set(None);
        form_open.set(true);
    };
    let create = move |_| {
        let d = draft.get_untracked();
        let loc = locale.get_untracked();
        if d.project_id.is_empty() || d.prompt.trim().is_empty() {
            form_error.set(Some(
                j(
                    loc,
                    "Choose a project and write a prompt.",
                    "请选择项目并填写提示词。",
                )
                .into(),
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
                j(
                    loc,
                    "Use a 24-hour HH:MM time.",
                    "请输入 24 小时制时间 HH:MM。",
                )
                .into(),
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
        <section class="home-calendar home-automation" data-testid="home-automation" aria-label=move || j(locale.get(),"Automation","自动化")>
            <button type="button" class="calendar-back" on:click=move |_| on_close.call(())>{compose_icon("arrow-left")}{move || j(locale.get(),"Back to research assistant","返回科研助理")}</button>
            <header class="home-calendar-heading"><div><h2>{move || j(locale.get(),"Automation","自动化")}</h2><p>{move || j(locale.get(),"Run tasks on a schedule while Wisp is open, or any time you need them.","按计划运行任务，或在需要时随时执行。")}</p></div>
                <div class="automation-heading-actions">
                    <button type="button" class="calendar-icon" aria-label=move || j(locale.get(),"Refresh automation","刷新自动化") on:click=move |_| refresh.update(|n| *n += 1)>{compose_icon("refresh")}</button>
                    <button type="button" class="btn-primary" data-testid="automation-create" on:click=move |_| start(None)>{compose_icon("plus")}{move || j(locale.get(),"New scheduled task","创建定时任务")}</button>
                </div>
            </header>
            {move || notice.get().map(|e| view! {<p class="automation-notice" role="alert">{e}</p>})}
            <section class="automation-card automation-builtin" data-testid="automation-daily-recap">
                {move || {
                    let loc = locale.get();
                    let Some(d) = daily.get() else { return view! {<p class="automation-meta" role="status">{j(loc,"Loading…","正在读取…")}</p>}.into_view(); };
                    let (enabled, time) = (d.enabled, d.time.clone());
                    let toggle_time = time.clone();
                    let status = if d.running {
                        j(loc,"Running now…","正在运行…").to_string()
                    } else if let Some(at) = d.last_run_at {
                        let when = format!("{} {}", day_key(at), clock(at));
                        if loc == Locale::Zh { format!("上次运行：{when} · 起草 {} 份", d.drafted) } else { format!("Last run: {when} · {} drafted", d.drafted) }
                    } else {
                        j(loc,"Not run yet","尚未运行").to_string()
                    };
                    view! {
                        <div class="automation-card-head">{compose_icon("sparkles")}<h3>{j(loc,"Daily research recap","每日研究回顾")}</h3><span class="automation-tag">{j(loc,"Built-in","内置")}</span>
                            <label class="automation-toggle"><input type="checkbox" role="switch" data-testid="daily-recap-enabled" prop:checked=enabled on:change=move |ev| save_daily(event_target_checked(&ev), toggle_time.clone())/><span>{if enabled {j(loc,"On","已开启")} else {j(loc,"Off","已关闭")}}</span></label>
                        </div>
                        <p class="automation-desc">{j(loc,"Every day after","每天")}" "
                            <input type="time" class="automation-time" data-testid="daily-recap-time" aria-label=j(loc,"Daily recap time","每日回顾时间") prop:value=time on:change=move |ev| save_daily(enabled, event_target_value(&ev))/>
                            " "{j(loc,"it drafts a recap for each project's recent days with recorded activity (catching up to 3 days). Drafts become records only after you confirm them in the research journey.","起，为各项目最近有记录的日子起草研究回顾（最多补齐 3 天）。草稿在研究历程中确认后才算正式记录。")}</p>
                        <p class="automation-meta" data-testid="daily-recap-status">{status}</p>
                        {d.error.map(|e| view! {<p class="automation-error" role="alert">{e}</p>})}
                        <div class="automation-actions">
                            <button type="button" class="btn-ghost" data-testid="daily-recap-run" prop:disabled=d.running on:click=move |_| spawn_local(async move {
                                if let Ok(value) = call::<DailyRecapAutomation>("run_daily_recap_now", serde_json::json!({})).await { daily.set(Some(value)); }
                            })>{compose_icon("play")}{j(loc,"Run now","立即运行")}</button>
                            <button type="button" class="journey-link" on:click=move |_| on_open_specialists.call(())>{compose_icon("gear")}{j(loc,"Change model in Settings → Specialists → Recap","在 设置 → 专家 → Recap 中更换模型")}</button>
                        </div>
                    }.into_view()
                }}
            </section>
            {move || form_open.get().then(|| {
                let loc = locale.get();
                view! {<section class="automation-card automation-form" data-testid="automation-form" aria-label=j(loc,"New scheduled task","创建定时任务")>
                    <h3>{j(loc,"New scheduled task","创建定时任务")}</h3>
                    <div class="automation-form-grid">
                        <label>{j(loc,"Project","项目")}<select on:change=move |ev| draft.update(|d| d.project_id = event_target_value(&ev))>
                            {projects.get().into_iter().map(|p| { let id = p.id.clone(); view! {<option value=p.id prop:selected=move || draft.with(|d| d.project_id == id)>{p.name}</option>} }).collect_view()}
                        </select></label>
                        <label>{j(loc,"Name","名称")}<input type="text" prop:value=move || draft.with(|d| d.name.clone()) on:input=move |ev| draft.update(|d| d.name = event_target_value(&ev))/></label>
                        <label>{j(loc,"Repeat","频率")}<select on:change=move |ev| draft.update(|d| d.cadence = match event_target_value(&ev).as_str() { "weekly" => Cadence::Weekly, "hourly" => Cadence::Hourly, _ => Cadence::Daily })>
                            <option value="daily" prop:selected=move || draft.with(|d| d.cadence == Cadence::Daily)>{j(loc,"Daily","每天")}</option>
                            <option value="weekly" prop:selected=move || draft.with(|d| d.cadence == Cadence::Weekly)>{j(loc,"Weekly","每周")}</option>
                            <option value="hourly" prop:selected=move || draft.with(|d| d.cadence == Cadence::Hourly)>{j(loc,"Every few hours","每隔几小时")}</option>
                        </select></label>
                        {move || match draft.with(|d| d.cadence) {
                            Cadence::Hourly => view! {<label>{j(loc,"Every (hours)","间隔（小时）")}<input type="number" min="1" max="168" prop:value=move || draft.with(|d| d.hours.to_string()) on:input=move |ev| draft.update(|d| d.hours = event_target_value(&ev).parse().unwrap_or(1))/></label>}.into_view(),
                            cadence => view! {
                                {(cadence == Cadence::Weekly).then(|| view! {<label>{j(loc,"Day","星期")}<select on:change=move |ev| draft.update(|d| d.weekday = event_target_value(&ev).parse().unwrap_or(1))>
                                    {[1u32, 2, 3, 4, 5, 6, 0].into_iter().map(|w| view! {<option value=w.to_string() prop:selected=move || draft.with(|d| d.weekday == w)>{weekday_name(loc, w)}</option>}).collect_view()}
                                </select></label>})}
                                <label>{j(loc,"Time","时间")}<input type="time" prop:value=move || draft.with(|d| d.time.clone()) on:input=move |ev| draft.update(|d| d.time = event_target_value(&ev))/></label>
                            }.into_view(),
                        }}
                        <label class="automation-span">{j(loc,"Prompt","提示词")}<textarea rows="6" prop:value=move || draft.with(|d| d.prompt.clone()) on:input=move |ev| draft.update(|d| d.prompt = event_target_value(&ev))></textarea></label>
                    </div>
                    <p class="automation-meta">{j(loc,"When due, Wisp starts a new session in the project and sends this prompt. Tasks run only while Wisp is open; a missed time runs once on the next launch.","到点后会在所选项目中新建一个会话并发送这段提示词。仅在 Wisp 运行时触发，错过的时间会在下次启动时补跑一次。")}</p>
                    {move || form_error.get().map(|e| view! {<p class="automation-error" role="alert">{e}</p>})}
                    <div class="automation-actions automation-form-actions">
                        <button type="button" class="btn-ghost" on:click=move |_| form_open.set(false)>{j(loc,"Cancel","取消")}</button>
                        <button type="button" class="btn-primary" data-testid="automation-save" on:click=create>{j(loc,"Create","创建")}</button>
                    </div>
                </section>}
            })}
            <section class="automation-section">
                <h3>{move || j(locale.get(),"Scheduled tasks","定时任务")}</h3>
                {move || {
                    let loc = locale.get();
                    match schedules.get() {
                        None => view! {<p class="automation-meta" role="status">{j(loc,"Loading…","正在读取…")}</p>}.into_view(),
                        Some(Err(e)) => view! {<p class="automation-error" role="alert">{e}</p>}.into_view(),
                        Some(Ok(rows)) => {
                            // Privacy mode hides a project's tasks with the project.
                            let rows: Vec<_> = rows.into_iter().filter_map(|s| project_name(&s.project_id).map(|name| (s, name))).collect();
                            if rows.is_empty() {
                                return view! {<div class="automation-empty" data-testid="automation-empty">{j(loc,"No scheduled tasks yet. Start from a template below or write your own prompt.","还没有定时任务。可以从下方模板开始，或自己写一段提示词。")}</div>}.into_view();
                            }
                            rows.into_iter().map(|(s, name)| {
                                let (id, toggle_id, run_id, delete_id, confirm_id) = (s.id.clone(), s.id.clone(), s.id.clone(), s.id.clone(), s.id.clone());
                                let enabled = s.enabled;
                                let timer = s.replace_previous_turn;
                                let next = if enabled { format!("{} {} {}", j(loc,"Next","下次"), day_key(s.next_run_at), clock(s.next_run_at)) } else { j(loc,"Paused","已暂停").into() };
                                view! {<article class="automation-row" data-schedule-id=id class:paused=!enabled>
                                    <div class="automation-row-main"><strong>{s.name.clone()}</strong>
                                        <small>{format!("{name} · {} · {next}", cadence_label(loc, &s))}</small>
                                        <p class="automation-prompt">{s.prompt.clone()}</p></div>
                                    <div class="automation-row-actions">
                                        <label class="automation-toggle"><input type="checkbox" role="switch" aria-label=format!("{} {}", j(loc,"Enable","启用"), s.name) prop:checked=enabled on:change=move |ev| act("set_schedule_enabled", serde_json::json!({"id":toggle_id,"enabled":event_target_checked(&ev)}))/></label>
                                        <button type="button" class="calendar-icon" title=j(loc,"Run now","立即运行") aria-label=format!("{} {}", j(loc,"Run now","立即运行"), s.name) on:click=move |_| { act("run_schedule_now", serde_json::json!({"id":run_id})); notice.set(Some(if timer { j(locale.get_untracked(),"Requested a check in the bound conversation.","已请求在绑定会话中检查。") } else { j(locale.get_untracked(),"Started. The result appears in a new session of that project.","已触发，结果会出现在该项目的新会话中。") }.into())); }>{compose_icon("play")}</button>
                                        <button type="button" class="calendar-icon automation-delete" class:confirming=move || deleting.get().as_ref() == Some(&confirm_id)
                                            title=move || if deleting.get().as_ref() == Some(&delete_id) {j(locale.get(),"Click again to delete","再次点击确认删除")} else {j(locale.get(),"Delete","删除")}
                                            aria-label=format!("{} {}", j(loc,"Delete","删除"), s.name)
                                            on:click=move |_| if deleting.get_untracked().as_ref() == Some(&s.id) { deleting.set(None); act("delete_schedule", serde_json::json!({"id":s.id.clone()})); } else { deleting.set(Some(s.id.clone())); }>{compose_icon("trash")}</button>
                                    </div>
                                </article>}
                            }).collect_view()
                        }
                    }
                }}
            </section>
            <section class="automation-section">
                <h3>{move || j(locale.get(),"Templates","定时任务模板")}</h3>
                <div class="automation-templates">
                    {TEMPLATES.iter().map(|t| view! {
                        <button type="button" class="automation-template" on:click=move |_| start(Some(t))>
                            <span class="automation-template-head">{compose_icon(t.icon)}<strong>{move || j(locale.get(), t.title.0, t.title.1)}</strong></span>
                            <span class="automation-template-desc">{move || j(locale.get(), t.prompt.0, t.prompt.1)}</span>
                            <small>{move || { let loc = locale.get(); let when = if t.cadence == Cadence::Weekly { weekday_name(loc, t.weekday).to_string() } else { j(loc,"Daily","每天").to_string() }; format!("{when} {}", t.time) }}</small>
                        </button>
                    }).collect_view()}
                </div>
            </section>
        </section>
    }
}
