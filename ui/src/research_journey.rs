//! Daily, project-scoped research history. The journal is a view over recorded
//! work, not an LLM-generated claim about what the researcher discovered.
use crate::app_support::{compose_icon, load_file_content};
use crate::bindings::{invoke_checked, media_thumbnail_url};
use crate::dto::{
    ResearchEdge, ResearchGraph, ResearchJournalInput, ResearchJourney, ResearchJourneyEntry,
    ResearchJourneySource, ResearchRecap, ResearchRecapEdit, ResearchRecapItem, RunRecord,
};
use crate::i18n::{t, Locale};
use crate::text::{file_kind, parse_csv_line, unique_dom_id};
use crate::window_capture_escape;
use leptos::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use wasm_bindgen::JsValue;

pub(super) fn j(loc: Locale, en: &'static str, zh: &'static str) -> &'static str {
    if loc == Locale::Zh {
        zh
    } else {
        en
    }
}
pub(super) fn date(ts: i64) -> js_sys::Date {
    js_sys::Date::new(&JsValue::from_f64(ts as f64 * 1000.0))
}
pub(super) fn day_key(ts: i64) -> String {
    let d = date(ts);
    format!(
        "{:04}-{:02}-{:02}",
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date()
    )
}
pub(super) fn now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}
pub(super) fn month_of(ts: i64) -> (i32, u32) {
    let d = date(ts);
    (d.get_full_year() as i32, d.get_month())
}
pub(super) fn month_start(month: (i32, u32)) -> i64 {
    (js_sys::Date::new_with_year_month_day(month.0 as u32, month.1 as i32, 1).get_time() / 1000.0)
        as i64
}
pub(super) fn shift_month(month: (i32, u32), delta: i32) -> (i32, u32) {
    let n = month.0 * 12 + month.1 as i32 + delta;
    (n.div_euclid(12), n.rem_euclid(12) as u32)
}
pub(super) fn clock(ts: i64) -> String {
    let d = date(ts);
    format!("{:02}:{:02}", d.get_hours(), d.get_minutes())
}
pub(super) fn category(loc: Locale, kind: &str) -> &'static str {
    match kind {
        "progress" => j(loc, "Progress", "进展"),
        "finding" => j(loc, "Finding", "发现"),
        "decision" => j(loc, "Decision", "决策"),
        "next" => j(loc, "Next step", "待继续"),
        "run" => j(loc, "Experiment", "实验"),
        "session" => j(loc, "Conversation", "会话"),
        "archive" => j(loc, "Research archive", "研究归档"),
        "paper" => j(loc, "Paper", "文献"),
        "data_asset" => j(loc, "Data", "数据"),
        _ => j(loc, "Output", "产出"),
    }
}
fn icon(kind: &str) -> &'static str {
    match kind {
        "finding" => "lightbulb",
        "decision" => "fork",
        "next" => "play",
        "run" => "flask",
        "session" => "chat",
        "archive" => "archive",
        "data_asset" => "database",
        "paper" => "book",
        "artifact" => "image",
        _ => "plan",
    }
}
pub(super) fn status(loc: Locale, value: &str) -> String {
    match value {
        "submitted" => j(loc, "Submitted", "已提交"),
        "started" => j(loc, "Started", "开始"),
        "succeeded" => j(loc, "Completed", "已完成"),
        "failed" => j(loc, "Failed", "失败"),
        "cancelled" => j(loc, "Cancelled", "已取消"),
        "lost" => j(loc, "Connection lost", "失去连接"),
        "running" => j(loc, "Running", "运行中"),
        "queued" => j(loc, "Queued", "排队中"),
        _ => value,
    }
    .into()
}
pub(super) async fn call<T: serde::de::DeserializeOwned>(
    command: &str,
    args: serde_json::Value,
) -> Result<T, String> {
    let value = invoke_checked(
        command,
        serde_wasm_bindgen::to_value(&args).map_err(|e| e.to_string())?,
    )
    .await
    .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(value).map_err(|e| e.to_string())
}
pub(super) fn days(
    entries: &[ResearchJourneyEntry],
    query: &str,
) -> Vec<(String, Vec<ResearchJourneyEntry>)> {
    let query = query.trim().to_lowercase();
    let mut groups = BTreeMap::<String, Vec<ResearchJourneyEntry>>::new();
    for e in entries {
        if !query.is_empty()
            && !format!("{} {} {}", e.title, e.summary, day_key(e.occurred_at))
                .to_lowercase()
                .contains(&query)
        {
            continue;
        }
        groups
            .entry(day_key(e.occurred_at))
            .or_default()
            .push(e.clone());
    }
    for group in groups.values_mut() {
        // A session appears once per local day, even when many turns were sent.
        let mut sessions = HashSet::new();
        group.retain(|e| e.kind != "session" || sessions.insert(e.source_id.clone()));
        // A run that finished the same day shows once, as its outcome.
        let ended = group
            .iter()
            .filter(|e| e.id.starts_with("run-end:"))
            .map(|e| e.source_id.clone())
            .collect::<HashSet<_>>();
        group.retain(|e| !(e.id.starts_with("run-start:") && ended.contains(&e.source_id)));
    }
    groups.into_iter().rev().collect()
}

#[component]
pub(super) fn ResearchJourneyView(
    locale: RwSignal<Locale>,
    project_name: String,
    #[prop(optional_no_strip)] initial_day: Option<i64>,
    left: Signal<f64>,
    graph: ReadSignal<ResearchGraph>,
    artifact_open: Signal<bool>,
    on_close: Callback<()>,
    on_artifact: Callback<(String, String, String)>,
    on_session: Callback<String>,
    on_archive: Callback<String>,
) -> impl IntoView {
    let month = create_rw_signal(month_of(initial_day.unwrap_or_else(now)));
    let refresh = create_rw_signal(0u32);
    let focus_day = create_rw_signal(initial_day);
    let query = create_rw_signal(String::new());
    let graph_tab = create_rw_signal(false);
    let graph_canvas = create_rw_signal(false);
    let selected_edge = create_rw_signal::<Option<ResearchEdge>>(None);
    let selected = create_rw_signal::<Option<ResearchJourneyEntry>>(None);
    let selected_date = create_rw_signal(day_key(initial_day.unwrap_or_else(now)));
    let note_open = create_rw_signal(false);
    let run_open = create_rw_signal::<Option<String>>(None);
    // The day whose recap is being edited inline; Escape cancels the edit.
    let recap_editing = create_rw_signal::<Option<String>>(None);
    let history = create_local_resource(
        move || (month.get(), refresh.get(), focus_day.get()),
        move |(m, _, day)| async move {
            let (from, until) = match day {
                None => (month_start(m), month_start(shift_month(m, 1))),
                Some(ts) => {
                    let d = date(ts);
                    (
                        ts,
                        (js_sys::Date::new_with_year_month_day(
                            d.get_full_year(),
                            d.get_month() as i32,
                            d.get_date() as i32 + 1,
                        )
                        .get_time()
                            / 1000.0) as i64,
                    )
                }
            };
            call::<ResearchJourney>(
                "get_research_journey",
                serde_json::json!({"from":from,"until":until}),
            )
            .await
        },
    );
    let source = create_local_resource(
        move || {
            selected
                .get()
                .filter(|e| e.kind == "artifact")
                .map(|e| e.source_id)
        },
        move |id| async move {
            match id {
                Some(id) => call::<ResearchJourneySource>(
                    "get_research_journey_source",
                    serde_json::json!({"versionId":id}),
                )
                .await
                .map(Some),
                None => Ok(None),
            }
        },
    );
    create_effect(move |_| {
        if !history.loading().get() {
            if let Some(Ok(data)) = history.get() {
                let current = selected.get_untracked();
                if current.is_none()
                    || !data
                        .entries
                        .iter()
                        .any(|e| Some(&e.id) == current.as_ref().map(|e| &e.id))
                {
                    selected.set(data.entries.into_iter().find(|e| e.kind == "artifact"));
                }
            }
        }
    });
    window_capture_escape(move || {
        // The app's artifact viewer is visually above this page and its details.
        if artifact_open.get_untracked() {
            return false;
        }
        if note_open.get_untracked() {
            note_open.set(false);
            return true;
        }
        if run_open.get_untracked().is_some() {
            run_open.set(None);
            return true;
        }
        if recap_editing.get_untracked().is_some() {
            recap_editing.set(None);
            return true;
        }
        if selected_edge.get_untracked().is_some() {
            selected_edge.set(None);
            return true;
        }
        false
    });
    let choose = Callback::new(move |entry: ResearchJourneyEntry| {
        selected.set(Some(entry));
        if window()
            .inner_width()
            .ok()
            .and_then(|v| v.as_f64())
            .is_some_and(|w| w <= 960.0)
        {
            if let Some(el) = document().get_element_by_id("journey-source") {
                el.scroll_into_view();
            }
        }
    });
    view! {
        <section class="research-journey" data-testid="research-journey" aria-label=move || j(locale.get(),"Research journey","研究历程")
            style=move || format!("--journey-left:{}px",left.get())>
            <header class="journey-header">
                <div class="journey-breadcrumb">{project_name}<span>" / "</span>{move || j(locale.get(),"Research journey","研究历程")}</div>
                <div class="journey-title-row"><div><h1>{move || j(locale.get(),"Research journey","研究历程")}</h1>
                    <p>{move || j(locale.get(),"Your daily exploration, outputs and decisions — with their sources.","每天的探索、产出与关键决定，都有迹可循。")}</p></div>
                    <div class="journey-actions"><button type="button" class="btn-primary" on:click=move |_| note_open.set(true)>{compose_icon("note-plus")}{move || j(locale.get(),"Add entry","补充记录")}</button>
                    <button type="button" class="journey-icon" aria-label=move || j(locale.get(),"Close research journey","关闭研究历程") on:click=move |_| on_close.call(())>{compose_icon("close")}</button></div>
                </div>
                <div class="journey-toolbar"><div role="tablist" aria-label=move || j(locale.get(),"Research view","研究视图")>
                    <button role="tab" aria-selected=move || (!graph_tab.get()).to_string() class:active=move || !graph_tab.get() on:click=move |_| graph_tab.set(false)>{move || j(locale.get(),"By day","按日")}</button>
                    <button role="tab" aria-selected=move || graph_tab.get().to_string() class:active=move || graph_tab.get() on:click=move |_| graph_tab.set(true)>{move || j(locale.get(),"Relationships","关系图")}</button></div>
                    <div class="journey-search">{compose_icon("search")}<input type="search" aria-label=move || j(locale.get(),"Search research records","搜索研究记录") placeholder=move || j(locale.get(),"Search this month's records","搜索本月研究记录") prop:value=move || query.get() on:input=move |ev| query.set(event_target_value(&ev)) /></div>
                    <button type="button" class="journey-icon" title=move || j(locale.get(),"Refresh","刷新") aria-label=move || j(locale.get(),"Refresh research records","刷新研究记录") on:click=move |_| refresh.update(|v|*v+=1)>{compose_icon("refresh")}</button>
                </div>
            </header>
            {move || if graph_tab.get() {
                let current=graph.get(); let loc=locale.get();
                view! { <div class="journey-graph" data-testid="journey-relationships">
                    <div class="journey-graph-tools"><span>{format!("{} {} · {} {}",current.nodes.len(),j(loc,"nodes","个节点"),current.edges.len(),j(loc,"relationships","条关系"))}</span>
                        <button class="btn-ghost" on:click=move |_| graph_canvas.update(|v|*v=!*v)>{move || if graph_canvas.get(){j(locale.get(),"List","列表")}else{j(locale.get(),"Graph","图谱")}}</button></div>
                    {if graph_canvas.get(){crate::research::research_graph_canvas(loc,&current,selected_edge)}else{crate::research::research_graph_list(loc,&current,selected_edge)}}
                    {selected_edge.get().map(|e|crate::research::research_edge_detail(loc,&current,e,selected_edge))}
                </div> }.into_view()
            } else {
                view! { <div class="journey-body">
                    <main class="journey-feed" data-testid="journey-feed">
                        {move || match if history.loading().get(){None}else{history.get()} {
                            None => view!{<div class="journey-empty" role="status">{j(locale.get(),"Loading research records…","正在读取研究记录…")}</div>}.into_view(),
                            Some(Err(error)) => view!{<div class="journey-empty" role="alert"><h2>{j(locale.get(),"Could not load research records","研究记录加载失败")}</h2><p>{error}</p><button class="btn-ghost" on:click=move |_| refresh.update(|n|*n+=1)>{j(locale.get(),"Try again","重试")}</button></div>}.into_view(),
                            Some(Ok(data)) => {
                                let groups=days(&data.entries,&query.get());
                                if groups.is_empty() { return view!{<div class="journey-empty"><h2>{j(locale.get(),"No records in this view","当前范围没有研究记录")}</h2><p>{j(locale.get(),"Choose another month, clear the search, or add a research note. Conversations, runs and registered outputs appear here automatically.","可以切换月份、清空搜索或补充记录。会话、实验和已登记的产出会自动出现在这里。")}</p></div>}.into_view(); }
                                let first=groups[0].0.clone();
                                let recaps=data.recaps.into_iter().map(|r|(day_key(r.day_start),r)).collect::<HashMap<_,_>>();
                                view!{
                                    {data.truncated.then(||view!{<p class="journey-notice" role="status">{j(locale.get(),"Showing the latest 2,000 events. Use the date picker to load one day.","当前展示最近 2,000 条活动，请按日查看以缩小范围。")}</p>})}
                                    {groups.into_iter().map(|(day,entries)|view!{<JourneyDay locale=locale day=day.clone() recap=recaps.get(&day).cloned() recap_editing=recap_editing entries=entries initially_open={day==first} selected_date=selected_date selected=selected on_select=choose on_session=on_session on_artifact=on_artifact run_open=run_open/>}).collect_view()}
                                }.into_view()
                            }
                        }}
                    </main>
                    <aside class="journey-aside">
                        <JourneyCalendar locale=locale month=month history=history selected_date=selected_date query=query focus_day=focus_day/>
                        <div class="journey-source" id="journey-source" data-testid="journey-source">
                            <h2>{move || if selected.get().is_some_and(|e|e.kind!="artifact"){j(locale.get(),"Record details","记录详情")}else{j(locale.get(),"Source details","产物来源")}}</h2>
                            {move || match selected.get() {
                                None=>view!{<p class="journey-muted">{j(locale.get(),"Select an output or record to trace its source.","选择一份产出或研究记录，查看它的来源。")}</p>}.into_view(),
                                Some(e)=>{
                                    let loc=locale.get();let artifact=e.kind=="artifact";let open=e.clone();
                                    view!{
                                        <h3>{e.title.clone()}</h3><p class="journey-muted">{format!("{} {} · {}",day_key(e.occurred_at),if e.manual{String::new()}else{clock(e.occurred_at)},if artifact{j(loc,"Registered","登记")}else if e.manual{j(loc,"Manually recorded","手动记录")}else{j(loc,"Recorded","记录")})}</p>
                                        {(!e.summary.is_empty()).then(||view!{<p class="journey-source-body">{e.summary.clone()}</p>})}
                                        {(e.manual && day_key(e.recorded_at)!=day_key(e.occurred_at)).then(||view!{<p class="journey-muted">{format!("{} {}",j(loc,"Added on","补记于"),day_key(e.recorded_at))}</p>})}
                                        {if artifact {view!{
                                            <p class="journey-muted">{format!("{} {}",j(loc,"Version","版本"),e.version_number.unwrap_or(1))}</p>
                                            {move || match if source.loading().get(){None}else{source.get()} {
                                                None=>view!{<p role="status">{j(locale.get(),"Loading source…","正在读取来源…")}</p>}.into_view(),
                                                Some(Err(err))=>view!{<p role="alert">{err}</p>}.into_view(),
                                                Some(Ok(Some(src)))=>{
                                                    let loc=locale.get();let rid=src.run_id.clone();
                                                    view!{
                                                        <div class="journey-lineage"><div>{compose_icon("database")}<strong>{j(loc,"Input data","输入数据")}</strong>
                                                            {if src.inputs.is_empty(){view!{<p class="journey-muted">{j(loc,"No inputs recorded","尚未记录输入")}</p>}.into_view()}else{src.inputs.into_iter().map(|input|{
                                                                let title=input.title.clone();
                                                                view!{<p>{match input.version_id { Some(id)=>view!{<button class="journey-link" on:click=move |_| on_artifact.call((format!("artifact-version:{id}"),title.clone(),file_kind(&title).unwrap_or("text").into()))>{input.title}</button>}.into_view(),None=>view!{<span>{input.title}</span>}.into_view()}}<small>{format!("{} · {}",input.role,input.confidence)}</small></p>}
                                                            }).collect_view()}}
                                                        </div><div>{compose_icon("flask")}<strong>{j(loc,"Experiment","实验运行")}</strong>
                                                            <p>{if src.run_title.is_empty(){j(loc,"No producing run recorded","尚未记录生成它的运行").to_string()}else{src.run_title}}</p>
                                                            <small>{format!("{} {}",src.context_id,status(loc,&src.run_status))}</small>
                                                            {src.generated_at.map(|ts|view!{<small>{format!("{} {} {}",day_key(ts),clock(ts),j(loc,"completed","完成"))}</small>})}
                                                        </div></div>
                                                        {rid.map(|id|view!{<button class="journey-link" on:click=move |_| run_open.set(Some(id.clone()))>{compose_icon("terminal")}{j(loc,"View run record","查看运行记录")}</button>})}
                                                    }.into_view()
                                                },_=>().into_view()
                                            }}
                                            <button class="journey-link journey-open-output" disabled=e.source_discarded on:click=move |_| on_artifact.call((format!("artifact-version:{}",open.source_id),open.title.clone(),file_kind(&open.title).unwrap_or("text").into()))>{compose_icon("external-link")}{if e.source_discarded{j(loc,"Source unavailable","源文件已不可用")}else{j(loc,"Open output","打开产物")}}</button>
                                        }.into_view()}else{view!{<p class="journey-muted">{if e.manual{j(loc,"This entry was added by the researcher.","这条记录由研究者手动补充。")}else{j(loc,"Recorded project object. Related evidence is available in Relationships.","已记录的项目对象，可在关系图中查看关联依据。")}}</p>}.into_view()}}
                                        {(e.kind=="archive").then(||e.frame_id.clone()).flatten().map(|id|view!{<button class="journey-link" data-testid="journey-open-archive" on:click=move |_|on_archive.call(id.clone())>{compose_icon("archive")}{j(loc,"View archive / Continue research","查看归档 / 继续研究")}</button>})}
                                        {e.frame_id.map(|id|view!{<button class="journey-link" on:click=move |_| on_session.call(id.clone())>{compose_icon("chat")}{j(loc,"Open conversation","打开会话")}</button>})}
                                    }.into_view()
                                }
                            }}
                        </div>
                    </aside>
                </div> }.into_view()
            }}
            {move || note_open.get().then(||view!{<JournalEditor locale=locale initial_date=selected_date.get_untracked() on_close=Callback::new(move |_|note_open.set(false)) on_saved=Callback::new(move |ts|{note_open.set(false);focus_day.set(None);month.set(month_of(ts));selected_date.set(day_key(ts));refresh.update(|n|*n+=1);})/>})}
            {move || run_open.get().map(|id|view!{<JourneyRun locale=locale run_id=id on_close=Callback::new(move |_|run_open.set(None))/>})}
        </section>
    }
}

#[component]
fn JourneyDay(
    locale: RwSignal<Locale>,
    day: String,
    recap: Option<ResearchRecap>,
    recap_editing: RwSignal<Option<String>>,
    entries: Vec<ResearchJourneyEntry>,
    initially_open: bool,
    selected_date: RwSignal<String>,
    selected: RwSignal<Option<ResearchJourneyEntry>>,
    on_select: Callback<ResearchJourneyEntry>,
    on_session: Callback<String>,
    on_artifact: Callback<(String, String, String)>,
    run_open: RwSignal<Option<String>>,
) -> impl IntoView {
    let expanded = create_rw_signal(initially_open);
    let recap = create_rw_signal(recap);
    let all_entries = store_value(entries.clone());
    let output_limit = create_rw_signal(3usize);
    let this_day = day.clone();
    create_effect(move |_| {
        if selected_date.get() == this_day {
            expanded.set(true);
        }
    });
    let outputs = entries
        .iter()
        .filter(|e| e.kind == "artifact")
        .cloned()
        .collect::<Vec<_>>();
    let sessions = entries
        .iter()
        .filter(|e| e.kind == "session")
        .cloned()
        .collect::<Vec<_>>();
    let activities = entries
        .iter()
        .filter(|e| e.kind == "run")
        .cloned()
        .collect::<Vec<_>>();
    let notes = entries
        .iter()
        .filter(|e| e.kind != "artifact" && e.kind != "session" && e.kind != "run")
        .cloned()
        .collect::<Vec<_>>();
    let headline = notes
        .iter()
        .find(|e| e.kind == "archive")
        .or_else(|| notes.iter().find(|e| e.kind == "progress"))
        .or_else(|| notes.iter().find(|e| e.kind == "decision"))
        .or_else(|| activities.iter().find(|e| e.status != "started"))
        .or_else(|| entries.first())
        .cloned()
        .unwrap_or_default();
    let run_count = activities
        .iter()
        .map(|e| &e.source_id)
        .collect::<HashSet<_>>()
        .len();
    let output_count = outputs.len();
    let session_count = sessions.len();
    let note_count = notes.len();
    let ts = entries.first().map(|e| e.occurred_at).unwrap_or_default();
    let d = date(ts);
    let month = d.get_month() + 1;
    let day_number = d.get_date();
    let weekday = d.get_day() as usize;
    let bounds = (
        (js_sys::Date::new_with_year_month_day(d.get_full_year(), d.get_month() as i32, d.get_date() as i32).get_time() / 1000.0) as i64,
        (js_sys::Date::new_with_year_month_day(d.get_full_year(), d.get_month() as i32, d.get_date() as i32 + 1).get_time() / 1000.0) as i64,
    );
    let recap_day = day.clone();
    let today = day == day_key(now());
    let summary = headline.summary.clone();
    let headline_entry = headline.clone();
    let notes = notes
        .into_iter()
        .filter(|e| !(e.kind == "progress" && e.id == headline.id))
        .collect::<Vec<_>>();
    let outputs = store_value(outputs);
    let activities = store_value(activities);
    let notes = store_value(notes);
    let sessions = store_value(sessions);
    view! {
        <article class="journey-day" class:today=today id=format!("journey-day-{day}") data-day=day>
            <div class="journey-day-date"><strong>{move ||if locale.get()==Locale::Zh{format!("{month} 月 {day_number} 日")}else{format!("{month:02}/{day_number:02}")}}</strong>
                <span>{move ||{let w=if locale.get()==Locale::Zh{["周日","周一","周二","周三","周四","周五","周六"][weekday]}else{["Sun","Mon","Tue","Wed","Thu","Fri","Sat"][weekday]};format!("{w}{}",if today{j(locale.get()," · Today"," · 今天")}else{""})}}</span></div>
            <div class="journey-day-main"><div class="journey-day-heading"><h2>{move ||match recap.get().filter(|r|r.status!="dismissed"){
                    // A kept recap names the day better than its latest record.
                    Some(r)=>view!{<span class="journey-headline" data-testid="journey-recap-headline">{r.headline}</span>}.into_view(),
                    None=>{let entry=headline_entry.clone();view!{<button class="journey-headline" on:click=move |_|on_select.call(entry.clone())>{headline.title.clone()}</button>}.into_view()},
                }}</h2>
                <button class="journey-icon" aria-expanded=move ||expanded.get().to_string() aria-label=move || if expanded.get(){j(locale.get(),"Collapse day","收起记录")}else{j(locale.get(),"Expand day","展开记录")} on:click=move |_|expanded.update(|v|*v=!*v)>{move ||compose_icon(if expanded.get(){"chevron-down"}else{"chevron-right"})}</button></div>
                {move ||(!summary.is_empty()&&recap.with(|r|r.as_ref().is_none_or(|r|r.status=="dismissed"))).then(||view!{<p class="journey-day-summary">{summary.clone()}</p>})}
                <JourneyRecap locale=locale day=recap_day bounds=bounds recap=recap editing=recap_editing entries=all_entries run_open=run_open on_session=on_session on_select=on_select on_artifact=on_artifact/>
                <p class="journey-day-counts">{move ||format!("{run_count} {} · {output_count} {} · {note_count} {} · {session_count} {}",j(locale.get(),"experiments","次实验"),j(locale.get(),"outputs","份产出"),j(locale.get(),"notes","条研究记录"),j(locale.get(),"conversations","个会话"))}</p>
                {move ||if expanded.get(){view!{
                    <div class="journey-activities">{activities.get_value().into_iter().map(|entry|{
                        let failed=matches!(entry.status.as_str(),"failed"|"lost"|"cancelled"); let id=entry.source_id.clone();let st=entry.status.clone();
                        let made=outputs.with_value(|o|o.iter().filter(|e|e.run_id.as_ref()==Some(&entry.source_id)).count());
                        view!{<button class="journey-activity" class:failed=failed on:click=move |_|run_open.set(Some(id.clone()))>{compose_icon(if failed{"circle-alert"}else if entry.status=="succeeded"{"check"}else{"clock"})}<time>{clock(entry.occurred_at)}</time><span>{entry.title}</span><small>{move ||if made>0{format!("{made} {} · {}",j(locale.get(),"outputs","份产出"),status(locale.get(),&st))}else{status(locale.get(),&st)}}</small></button>}
                    }).collect_view()}</div>
                    {(!outputs.get_value().is_empty()).then(||view!{<h3 class="journey-output-heading">{j(locale.get(),"Today's outputs","当日产出")}</h3><div class="journey-outputs">{outputs.get_value().into_iter().take(output_limit.get()).map(|e|view!{<JourneyOutput locale=locale entry=e selected=selected on_select=on_select/>}).collect_view()}</div>})}
                    {(output_count>output_limit.get()).then(||view!{<button class="journey-link journey-more-outputs" on:click=move |_|output_limit.update(|n|*n+=6)>{j(locale.get(),"Show more outputs","显示更多产出")}</button>})}
                    <div class="journey-notes">{notes.get_value().into_iter().map(|e|{
                        let entry=e.clone();let kind=e.kind.clone();
                        view!{<div class="journey-note-row">{compose_icon(icon(&e.kind))}<span>{move ||category(locale.get(),&kind)}</span><button class="journey-note-text" on:click=move |_|on_select.call(entry.clone())>{e.title}{(!e.summary.is_empty()).then(||view!{<small>{e.summary}</small>})}</button>{e.manual.then(||view!{<small class="journey-manual">{j(locale.get(),"Manual","手动")}</small>})}</div>}
                    }).collect_view()}</div>
                    {(!sessions.get_value().is_empty()).then(||view!{<div class="journey-session-links">{sessions.get_value().into_iter().map(|e|{let id=e.source_id;view!{<button class="journey-link" on:click=move |_|on_session.call(id.clone())>{compose_icon("chat")}{e.title}</button>}}).collect_view()}</div>})}
                }.into_view()}else{view!{<div class="journey-collapsed-outputs">{outputs.get_value().into_iter().take(3).map(|e|{let entry=e.clone();view!{<button class="journey-link" on:click=move |_|{expanded.set(true);on_select.call(entry.clone());}>{compose_icon("doc")}{e.title}</button>}}).collect_view()}</div>}.into_view()}}
            </div>
        </article>
    }
}

const RECAP_SECTIONS: [(&str, &str, &str); 4] = [
    ("done", "Done", "今日完成"),
    ("findings", "Findings & decisions", "发现与决定"),
    ("issues", "Issues", "问题与失败"),
    ("next", "Next", "待继续"),
];

fn recap_section<'a>(recap: &'a ResearchRecap, key: &str) -> &'a [ResearchRecapItem] {
    match key {
        "done" => &recap.done,
        "findings" => &recap.findings,
        "issues" => &recap.issues,
        _ => &recap.next,
    }
}

/// One item per non-empty line. The backend keeps the sources of lines the
/// researcher left unchanged.
fn recap_items(text: &str) -> Vec<ResearchRecapItem> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| ResearchRecapItem {
            text: line.into(),
            refs: vec![],
        })
        .collect()
}

/// The day's AI-drafted recap: generate, confirm, edit, regenerate or
/// dismiss. Each item links back to the records it cites.
#[component]
fn JourneyRecap(
    locale: RwSignal<Locale>,
    day: String,
    bounds: (i64, i64),
    recap: RwSignal<Option<ResearchRecap>>,
    editing: RwSignal<Option<String>>,
    entries: StoredValue<Vec<ResearchJourneyEntry>>,
    run_open: RwSignal<Option<String>>,
    on_session: Callback<String>,
    on_select: Callback<ResearchJourneyEntry>,
    on_artifact: Callback<(String, String, String)>,
) -> impl IntoView {
    let busy = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);
    let quiet = create_rw_signal(false);
    let draft = create_rw_signal((String::new(), vec![String::new(); 4]));
    let day = store_value(day);
    let generate = move |_| {
        busy.set(true);
        error.set(None);
        quiet.set(false);
        spawn_local(async move {
            match call::<Option<ResearchRecap>>("generate_research_recap", serde_json::json!({"from":bounds.0,"until":bounds.1})).await {
                Ok(Some(r)) => recap.set(Some(r)),
                Ok(None) => quiet.set(true),
                Err(e) => error.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let save = move |status: &'static str, edited: bool| {
        let Some(current) = recap.get_untracked() else { return };
        let (headline, mut sections) = if edited {
            let (headline, texts) = draft.get_untracked();
            (headline, texts.iter().map(|t| recap_items(t)).collect::<Vec<_>>())
        } else {
            (current.headline.clone(), RECAP_SECTIONS.iter().map(|(key, _, _)| recap_section(&current, key).to_vec()).collect())
        };
        let edit = ResearchRecapEdit {
            id: current.id,
            status: status.into(),
            headline,
            next: sections.pop().unwrap_or_default(),
            issues: sections.pop().unwrap_or_default(),
            findings: sections.pop().unwrap_or_default(),
            done: sections.pop().unwrap_or_default(),
        };
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match call::<ResearchRecap>("update_research_recap", serde_json::json!({"edit":edit})).await {
                Ok(r) => {
                    recap.set(Some(r));
                    editing.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let open_source = move |kind: String, id: String, title: String| match kind.as_str() {
        "run" => run_open.set(Some(id)),
        "session" => on_session.call(id),
        "artifact" => on_artifact.call((format!("artifact-version:{id}"), title.clone(), file_kind(&title).unwrap_or("text").into())),
        _ => {
            if let Some(entry) = entries.with_value(|rows| rows.iter().find(|e| e.source_id == id).cloned()) {
                on_select.call(entry);
            }
        }
    };
    view! {
        {move || {
            let loc = locale.get();
            let alert = move || error.get().map(|e| view! {<p class="journey-recap-error" role="alert">{e}</p>});
            let Some(r) = recap.get().filter(|r| r.status != "dismissed") else {
                return view! {<div class="journey-recap-empty">
                    <button type="button" class="journey-link" data-testid="journey-recap-generate" prop:disabled=move || busy.get() on:click=generate>{compose_icon("sparkles")}{move || if busy.get() {j(locale.get(),"Drafting recap…","正在起草回顾…")} else {j(locale.get(),"Generate recap","生成回顾")}}</button>
                    {move || quiet.get().then(|| view! {<small class="journey-muted">{j(locale.get(),"Nothing recorded to recap.","当天没有可总结的记录。")}</small>})}
                    {alert}
                </div>}.into_view();
            };
            if day.with_value(|d| editing.get().as_ref() == Some(d)) {
                return view! {<section class="journey-recap editing" data-testid="journey-recap-editor" aria-label=j(loc,"Edit recap","编辑回顾")>
                    <label>{j(loc,"Headline","标题")}<input type="text" prop:value=move || draft.with(|d| d.0.clone()) on:input=move |ev| draft.update(|d| d.0 = event_target_value(&ev))/></label>
                    {RECAP_SECTIONS.iter().enumerate().map(|(index, (_, en, zh))| view! {
                        <label>{j(loc,en,zh)}<textarea rows="3" prop:value=move || draft.with(|d| d.1[index].clone()) on:input=move |ev| draft.update(|d| d.1[index] = event_target_value(&ev))></textarea></label>
                    }).collect_view()}
                    <p class="journey-muted">{j(loc,"One item per line. A line you change no longer cites its sources.","每行一条；改动过的行不再关联原来源。")}</p>
                    {alert}
                    <div class="journey-recap-edit-actions">
                        <button type="button" class="btn-ghost" on:click=move |_| editing.set(None)>{j(loc,"Cancel","取消")}</button>
                        <button type="button" class="btn-primary" prop:disabled=move || busy.get() on:click=move |_| save("confirmed", true)>{compose_icon("save")}{j(loc,"Save and confirm","保存并确认")}</button>
                    </div>
                </section>}.into_view();
            }
            let confirmed = r.status == "confirmed";
            let sources = r.sources.clone();
            let start = r.clone();
            view! {<section class="journey-recap" data-testid="journey-recap" data-status=r.status.clone() aria-label=j(loc,"Daily recap","每日回顾")>
                <header class="journey-recap-head">
                    {compose_icon("sparkles")}<strong>{j(loc,"Daily recap","每日回顾")}</strong>
                    <span class="journey-recap-badge" class:confirmed=confirmed>{if confirmed {j(loc,"AI-drafted · confirmed","AI 起草 · 已确认")} else {j(loc,"AI draft · review before keeping","AI 草稿 · 待确认")}}</span>
                    <div class="journey-recap-actions">
                        {(!confirmed).then(|| view! {<button type="button" class="journey-link" data-testid="journey-recap-confirm" prop:disabled=move || busy.get() on:click=move |_| save("confirmed", false)>{compose_icon("check")}{j(loc,"Confirm","确认")}</button>})}
                        <button type="button" class="journey-icon" title=j(loc,"Edit recap","编辑回顾") aria-label=j(loc,"Edit recap","编辑回顾") on:click=move |_| {
                            draft.set((start.headline.clone(), RECAP_SECTIONS.iter().map(|(key, _, _)| recap_section(&start, key).iter().map(|i| i.text.clone()).collect::<Vec<_>>().join("\n")).collect()));
                            editing.set(Some(day.get_value()));
                        }>{compose_icon("edit")}</button>
                        <button type="button" class="journey-icon" title=j(loc,"Regenerate recap","重新生成回顾") aria-label=j(loc,"Regenerate recap","重新生成回顾") prop:disabled=move || busy.get() on:click=generate>{compose_icon("refresh")}</button>
                        <button type="button" class="journey-icon" title=j(loc,"Dismiss recap","忽略回顾") aria-label=j(loc,"Dismiss recap","忽略回顾") prop:disabled=move || busy.get() on:click=move |_| save("dismissed", false)>{compose_icon("eye-off")}</button>
                    </div>
                </header>
                {RECAP_SECTIONS.iter().filter(|(key, _, _)| !recap_section(&r, key).is_empty()).map(|(key, en, zh)| view! {
                    <div class="journey-recap-section" data-section=*key><h4>{j(loc,en,zh)}</h4><ul>
                        {recap_section(&r, key).iter().map(|item| view! {<li><span>{item.text.clone()}</span>
                            {item.refs.iter().filter_map(|i| sources.get(*i).cloned()).map(|source| {
                                let title = source.title.clone();
                                view! {<button type="button" class="journey-recap-ref" title=title.clone() on:click=move |_| open_source(source.kind.clone(), source.id.clone(), source.title.clone())>{title.chars().take(28).collect::<String>()}</button>}
                            }).collect_view()}
                        </li>}).collect_view()}
                    </ul></div>
                }).collect_view()}
                <p class="journey-recap-foot">{if loc == Locale::Zh {format!("由 {} 起草，可在 设置 → 专家 → Recap 中更换模型。", r.model)} else {format!("Drafted by {}. Change the model in Settings → Specialists → Recap.", r.model)}}</p>
                {alert}
            </section>}.into_view()
        }}
    }
}

#[component]
fn JourneyOutput(
    locale: RwSignal<Locale>,
    entry: ResearchJourneyEntry,
    selected: RwSignal<Option<ResearchJourneyEntry>>,
    on_select: Callback<ResearchJourneyEntry>,
) -> impl IntoView {
    let id = entry.id.clone();
    let output = entry.clone();
    let kind = file_kind(&entry.title).unwrap_or("text");
    let path = format!("artifact-version:{}", entry.source_id);
    let discarded = entry.source_discarded;
    let dom_id = unique_dom_id("journey-output");
    let owner_id = dom_id.clone();
    let preview = create_local_resource(
        || (),
        move |_| {
            let path = path.clone();
            let owner_id = owner_id.clone();
            async move {
                if discarded {
                    return None;
                }
                if kind == "image" {
                    media_thumbnail_url(&path, &owner_id)
                        .await
                        .as_string()
                        .map(|s| (true, s))
                } else if matches!(kind, "csv" | "markdown" | "text" | "json" | "code") {
                    load_file_content(&path, Locale::En, Some(2048))
                        .await
                        .ok()
                        .and_then(|c| c.text)
                        .map(|s| (false, s.chars().take(600).collect()))
                } else {
                    None
                }
            }
        },
    );
    view! {<button type="button" class="journey-output" id=dom_id class:selected=move || selected.get().is_some_and(|e|e.id==id) on:click=move |_|on_select.call(output.clone()) aria-label=entry.title.clone()>
        <div class="journey-output-preview">{move ||match preview.get().flatten(){
            Some((true,url))=>view!{<img src=url alt="" loading="lazy"/>}.into_view(),
            Some((false,text)) if kind=="csv" => {
                let delimiter=if text.lines().next().unwrap_or("").contains('\t') {'\t'} else {','};
                view!{<table class="journey-table-preview"><tbody>{text.lines().take(7).map(|line|view!{<tr>{if delimiter=='\t'{line.split('\t').map(str::to_string).collect::<Vec<_>>()}else{parse_csv_line(line)}.into_iter().take(4).map(|cell|view!{<td>{cell}</td>}).collect_view()}</tr>}).collect_view()}</tbody></table>}.into_view()
            },
            Some((false,text)) if kind=="markdown" => view!{<div class="journey-document-preview">{text.lines().filter(|l|!l.trim().is_empty()).take(7).map(|line|{
                if line.starts_with('#'){view!{<strong>{line.trim_start_matches('#').trim().to_string()}</strong>}.into_view()}else{view!{<p>{line.to_string()}</p>}.into_view()}
            }).collect_view()}</div>}.into_view(),
            Some((false,text))=>view!{<pre>{text}</pre>}.into_view(),
            None=>view!{<div class="journey-output-fallback">{compose_icon(if kind=="image"{"image"}else if kind=="csv"{"table"}else{"doc"})}<span>{kind.to_uppercase()}</span></div>}.into_view()
        }}</div><strong title=entry.title.clone()>{entry.title}</strong><small>{move ||format!("{} · v{}",if discarded{j(locale.get(),"Unavailable","源文件不可用")}else{j(locale.get(),"Registered output","已登记产出")},entry.version_number.unwrap_or(1))}</small>
    </button>}
}

#[component]
fn JourneyCalendar(
    locale: RwSignal<Locale>,
    month: RwSignal<(i32, u32)>,
    history: Resource<((i32, u32), u32, Option<i64>), Result<ResearchJourney, String>>,
    selected_date: RwSignal<String>,
    query: RwSignal<String>,
    focus_day: RwSignal<Option<i64>>,
) -> impl IntoView {
    let marks = create_rw_signal((month.get_untracked(), HashSet::<String>::new()));
    create_effect(move |_| {
        if focus_day.get().is_none() && !history.loading().get() {
            if let Some(Ok(h)) = history.get() {
                marks.set((
                    month.get(),
                    h.entries.iter().map(|e| day_key(e.occurred_at)).collect(),
                ));
            }
        }
    });
    view! {<div class="journey-calendar" data-testid="journey-calendar">
        <div class="journey-calendar-head"><strong>{move ||{let(y,m)=month.get();if locale.get()==Locale::Zh{format!("{y} 年 {} 月",m+1)}else{format!("{y} / {:02}",m+1)}}}</strong>
            <button class="journey-icon" aria-label=move ||j(locale.get(),"Previous month","上个月") on:click=move |_|{focus_day.set(None);month.update(|m|*m=shift_month(*m,-1));query.set(String::new());}>{compose_icon("chevron-left")}</button>
            <button class="journey-icon" aria-label=move ||j(locale.get(),"Next month","下个月") on:click=move |_|{focus_day.set(None);month.update(|m|*m=shift_month(*m,1));query.set(String::new());}>{compose_icon("chevron-right")}</button></div>
        <div class="journey-calendar-grid">{move ||{let labels=if locale.get()==Locale::Zh{["一","二","三","四","五","六","日"]}else{["M","T","W","T","F","S","S"]};labels.into_iter().map(|label|view!{<span class="journey-weekday">{label}</span>}).collect_view()}}
            {move ||{let m=month.get();let first=date(month_start(m));let offset=(first.get_day()+6)%7;
                let count=js_sys::Date::new_with_year_month_day(m.0 as u32,m.1 as i32+1,0).get_date();
                let active=if marks.get().0==m {marks.get().1}else{HashSet::new()};
                let blanks=(0..offset).map(|_|view!{<span></span>}).collect_view();
                let buttons=(1..=count).map(|d|{
                    let key=format!("{:04}-{:02}-{d:02}",m.0,m.1+1);let chosen=key.clone();let click_key=key.clone();let activity=active.contains(&key);let today=key==day_key(now());
                    view!{<button class:has-activity=activity class:today=today class:selected=move ||selected_date.get()==chosen aria-label=key.clone() aria-pressed=move ||selected_date.get()==key on:click=move |_|{
                        selected_date.set(click_key.clone());query.set(String::new());
                        focus_day.set(Some((js_sys::Date::new_with_year_month_day(m.0 as u32,m.1 as i32,d as i32).get_time()/1000.0) as i64));
                        if let Some(el)=document().get_element_by_id(&format!("journey-day-{click_key}")){el.scroll_into_view();}
                    }>{d}</button>}
                }).collect_view();
                view!{<>{blanks}{buttons}</>}
            }}
        </div><button class="journey-link journey-today" on:click=move |_|{focus_day.set(None);month.set(month_of(now()));selected_date.set(day_key(now()));query.set(String::new());if let Some(el)=document().get_element_by_id(&format!("journey-day-{}",day_key(now()))){el.scroll_into_view();}}>{j(locale.get(),"Back to today","回到今天")}</button>
        {move ||focus_day.get().map(|_|view!{<button class="journey-link" on:click=move |_|focus_day.set(None)>{j(locale.get(),"Show full month","显示整月")}</button>})}
        <p class="journey-timezone">{j(locale.get(),"Dates follow this device's timezone","日期按本机时区显示")}</p>
    </div>}
}

#[component]
fn JournalEditor(
    locale: RwSignal<Locale>,
    initial_date: String,
    on_close: Callback<()>,
    on_saved: Callback<i64>,
) -> impl IntoView {
    let title = create_rw_signal(String::new());
    let body = create_rw_signal(String::new());
    let category_value = create_rw_signal("progress".to_string());
    let note_date = create_rw_signal(initial_date);
    let busy = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);
    view! {<div class="overlay journey-detail-overlay" on:click=move |_|on_close.call(())><form class="modal journey-editor" role="dialog" aria-modal="true" aria-label=move ||j(locale.get(),"Add research entry","补充研究记录") on:click=|ev|ev.stop_propagation() on:submit=move |ev|{
        ev.prevent_default();if busy.get_untracked(){return;}
        let timestamp=js_sys::Date::new(&JsValue::from_str(&format!("{}T12:00:00",note_date.get_untracked()))).get_time()/1000.0;
        if !timestamp.is_finite(){error.set(Some(j(locale.get(),"Choose a valid date","请选择有效日期").into()));return;}
        let input=ResearchJournalInput{title:title.get_untracked(),body:body.get_untracked(),category:category_value.get_untracked(),occurred_at:timestamp as i64};
        busy.set(true);error.set(None);
        spawn_local(async move{let result=call::<String>("add_research_journal_entry",serde_json::json!({"input":input})).await;
            // Closing the dialog while saving must never reopen it or change
            // another project's selected month when this response arrives.
            if busy.try_get_untracked().is_none(){return;}
            busy.set(false);match result{Ok(_)=>on_saved.call(timestamp as i64),Err(e)=>error.set(Some(e))}
        });
    }><div class="journey-dialog-head"><h2>{j(locale.get(),"Add research entry","补充研究记录")}</h2><button type="button" class="journey-icon" aria-label=move ||j(locale.get(),"Close entry editor","关闭记录编辑器") on:click=move |_|on_close.call(())>{compose_icon("close")}</button></div>
        <p class="journey-muted">{j(locale.get(),"Record progress, a finding, a decision or a next step. The entry date and the time you added it are kept separately.","记录进展、发现、决策或下一步。研究日期与补记时间会分别保存。")}</p>
        <div class="journey-form-row"><label>{j(locale.get(),"Research date","研究日期")}<input type="date" required prop:value=move ||note_date.get() on:input=move |ev|note_date.set(event_target_value(&ev))/></label>
        <label>{j(locale.get(),"Category","记录类型")}<select aria-label=move ||j(locale.get(),"Category","记录类型") prop:value=move ||category_value.get() on:change=move |ev|category_value.set(event_target_value(&ev))>{["progress","finding","decision","next"].into_iter().map(|kind|view!{<option value=kind>{category(locale.get(),kind)}</option>}).collect_view()}</select></label></div>
        <label>{j(locale.get(),"Title","标题")}<input required maxlength="200" prop:value=move ||title.get() on:input=move |ev|title.set(event_target_value(&ev))/></label>
        <label>{j(locale.get(),"Details and evidence","详情与依据")}<textarea rows="5" maxlength="10000" prop:value=move ||body.get() on:input=move |ev|body.set(event_target_value(&ev))></textarea></label>
        {move ||error.get().map(|e|view!{<p class="journey-error" role="alert">{e}</p>})}
        <footer><button type="button" class="btn-ghost" on:click=move |_|on_close.call(())>{j(locale.get(),"Cancel","取消")}</button><button class="btn-primary" type="submit" disabled=move ||busy.get()>{move ||if busy.get(){j(locale.get(),"Saving…","正在保存…")}else{j(locale.get(),"Save entry","保存记录")}}</button></footer>
    </form></div>}
}

#[component]
fn JourneyRun(locale: RwSignal<Locale>, run_id: String, on_close: Callback<()>) -> impl IntoView {
    let detail = create_local_resource(
        || (),
        move |_| {
            let id = run_id.clone();
            async move { call::<RunRecord>("get_run_detail", serde_json::json!({"runId":id})).await }
        },
    );
    view! {<div class="overlay journey-detail-overlay" on:click=move |_|on_close.call(())><section class="modal journey-run" role="dialog" aria-modal="true" aria-label=move ||j(locale.get(),"Run record","运行记录") on:click=|ev|ev.stop_propagation()>
        <div class="journey-dialog-head"><h2>{j(locale.get(),"Run record","运行记录")}</h2><button class="journey-icon" aria-label=move ||j(locale.get(),"Close run record","关闭运行记录") on:click=move |_|on_close.call(())>{compose_icon("close")}</button></div>
        <div class="journey-run-body">
            {move || match detail.get() {
                None => view!{<p role="status">{t(locale.get(),"loading")}</p>}.into_view(),
                Some(Err(e)) => view!{<p class="journey-error" role="alert">{e}</p>}.into_view(),
                Some(Ok(run)) => {
                    let loc = locale.get();
                    let timestamp = |value: Option<i64>| value.map(|ts|format!("{} {}",day_key(ts),clock(ts))).unwrap_or_else(||j(loc,"Not recorded","未记录").into());
                    let failed = matches!(run.status.as_str(), "failed" | "lost");
                    let succeeded = run.status == "succeeded";
                    view! {
                        <div class="journey-run-summary">
                            <h3>{run.title}</h3>
                            <span class="journey-run-status" class:succeeded=succeeded class:failed=failed>
                                {compose_icon(if succeeded {"check"} else if failed {"circle-alert"} else {"clock"})}
                                {status(loc,&run.status)}
                            </span>
                        </div>
                        <dl class="journey-run-meta">
                            <div><dt>{j(loc,"Execution context","运行环境")}</dt><dd>{run.context_id}</dd></div>
                            <div><dt>{j(loc,"Exit code","退出码")}</dt><dd>{run.exit_code.map(|code|code.to_string()).unwrap_or_else(||j(loc,"Not recorded","未记录").into())}</dd></div>
                            <div><dt>{j(loc,"Started (local time)","开始时间（本地）")}</dt><dd>{timestamp(run.started_at)}</dd></div>
                            <div><dt>{j(loc,"Ended (local time)","结束时间（本地）")}</dt><dd>{timestamp(run.ended_at)}</dd></div>
                        </dl>
                        <JourneyRunText title=j(loc,"Command","执行命令") text=run.command empty=j(loc,"No command recorded","未记录执行命令")/>
                        <JourneyRunText title=j(loc,"Standard output · log tail","标准输出 · 日志尾部") text=run.stdout_tail empty=j(loc,"No standard output recorded","暂无标准输出")/>
                        <JourneyRunText title=j(loc,"Standard error · log tail","标准错误 · 日志尾部") text=run.stderr_tail empty=j(loc,"No standard error recorded","暂无标准错误输出")/>
                    }.into_view()
                }
            }}
        </div>
    </section></div>}
}

#[component]
fn JourneyRunText(title: &'static str, text: Option<String>, empty: &'static str) -> impl IntoView {
    view! {
        <section class="journey-run-section" aria-label=title>
            <h4>{title}</h4>
            {match text.filter(|value| !value.trim().is_empty()) {
                Some(value) => view!{<pre>{value}</pre>}.into_view(),
                None => view!{<p>{empty}</p>}.into_view(),
            }}
        </section>
    }
}
