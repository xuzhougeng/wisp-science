use crate::app_support::compose_icon;
use crate::dto::{ArchiveFileChoice, ConfirmResearchArchive, ResearchArchive};
use crate::i18n::{localize_backend, Locale};
use crate::research_journey::{call, j};
use leptos::*;

#[component]
pub(crate) fn ArchiveReview(
    locale: RwSignal<Locale>,
    frame_id: String,
    busy: RwSignal<bool>,
    minimized: RwSignal<bool>,
    on_close: Callback<()>,
    on_frozen: Callback<String>,
    on_continue: Callback<String>,
    on_notebook: Callback<String>,
    on_file: Callback<(String, String, String)>,
) -> impl IntoView {
    let frame = store_value(frame_id);
    let record = create_rw_signal::<Option<ResearchArchive>>(None);
    let edited = create_rw_signal::<Option<ResearchArchive>>(None);
    create_effect(move |_| edited.set(record.get()));
    let error = create_rw_signal(String::new());
    let accepted = create_rw_signal(false);
    let closed = store_value(false);
    on_cleanup(move || {
        closed.set_value(true);
        busy.set(false);
    });
    let prepare = Callback::new(move |_: ()| {
        busy.set(true);
        error.set(String::new());
        accepted.set(false);
        spawn_local(async move {
            let result = call::<ResearchArchive>(
                "prepare_research_archive",
                serde_json::json!({"frameId":frame.get_value()}),
            )
            .await;
            if closed.try_get_value().unwrap_or(true) {
                return;
            }
            busy.set(false);
            match result {
                Ok(a) => record.set(Some(a)),
                Err(e) => error.set(e),
            }
        });
    });
    busy.set(true);
    spawn_local(async move {
        let result = call::<Option<ResearchArchive>>(
            "get_research_archive",
            serde_json::json!({"frameId":frame.get_value()}),
        )
        .await;
        if closed.try_get_value().unwrap_or(true) {
            return;
        }
        busy.set(false);
        match result {
            Ok(Some(a)) => record.set(Some(a)),
            Ok(None) => prepare.call(()),
            Err(e) => error.set(e),
        }
    });
    let frozen =
        create_memo(move |_| record.with(|a| a.as_ref().is_some_and(|a| a.frozen_at.is_some())));
    let confirm = move |_| {
        let Some(a) = edited.get_untracked() else {
            return;
        };
        if busy.get_untracked() || !accepted.get_untracked() || a.frozen_at.is_some() {
            return;
        }
        let input = ConfirmResearchArchive {
            id: a.id,
            title: a.title,
            report: a.report,
            scripts: a.scripts,
            files: a
                .files
                .into_iter()
                .map(|f| ArchiveFileChoice {
                    path: f.path,
                    action: f.action,
                })
                .collect(),
        };
        busy.set(true);
        error.set(String::new());
        spawn_local(async move {
            let result = call::<ResearchArchive>(
                "confirm_research_archive",
                serde_json::json!({"frameId":frame.get_value(),"input":input}),
            )
            .await;
            if closed.try_get_value().unwrap_or(true) {
                return;
            }
            busy.set(false);
            match result {
                Ok(a) => {
                    on_frozen.call(a.frame_id.clone());
                    record.set(Some(a));
                }
                Err(e) => {
                    error.set(e);
                    // A filesystem cleanup failure may follow a successful freeze.
                    if let Ok(Some(a)) = call::<Option<ResearchArchive>>(
                        "get_research_archive",
                        serde_json::json!({"frameId":frame.get_value()}),
                    )
                    .await
                    {
                        if a.frozen_at.is_some() {
                            on_frozen.call(a.frame_id.clone());
                            record.set(Some(a));
                        }
                    }
                }
            }
        });
    };
    let retry = move |_| {
        busy.set(true);
        error.set(String::new());
        spawn_local(async move {
            let result = call::<ResearchArchive>(
                "retry_research_archive_cleanup",
                serde_json::json!({"frameId":frame.get_value()}),
            )
            .await;
            if closed.try_get_value().unwrap_or(true) {
                return;
            }
            busy.set(false);
            match result {
                Ok(a) => record.set(Some(a)),
                Err(e) => error.set(e),
            }
        });
    };
    let continue_research = move |_| {
        busy.set(true);
        error.set(String::new());
        spawn_local(async move {
            let result = call::<String>(
                "continue_research_archive",
                serde_json::json!({"frameId":frame.get_value()}),
            )
            .await;
            if closed.try_get_value().unwrap_or(true) {
                return;
            }
            busy.set(false);
            match result {
                Ok(id) => on_continue.call(id),
                Err(e) => error.set(e),
            }
        });
    };
    let pill_label = create_memo(move |_| {
        if busy.get() {
            j(locale.get(), "Archiving in progress…", "归档处理中…")
        } else if !error.get().is_empty() {
            j(locale.get(), "Archive needs attention", "归档需要处理")
        } else if frozen.get() {
            j(locale.get(), "Archived milestone", "已归档研究节点")
        } else {
            j(locale.get(), "Archive ready for review", "归档材料已就绪")
        }
    });
    view! {
        {move || if minimized.get() { view! {
            <div class="archive-pill" data-testid="archive-pill" role="status" aria-live="polite" class:archive-pill-error=move ||!busy.get()&&!error.get().is_empty()>
                <button class="archive-pill-restore" data-testid="archive-pill-restore" title=move ||pill_label.get() aria-label=move ||pill_label.get() on:click=move |_|minimized.set(false)>
                    {compose_icon("archive")}
                    <span>{move ||pill_label.get()}</span>
                </button>
                {move || (!busy.get()).then(|| view!{
                    <button class="icon-btn" aria-label=j(locale.get(),"Close archive","关闭归档") on:click=move |_|on_close.call(())>{compose_icon("close")}</button>
                })}
            </div>
        }.into_view()} else { view! {
        <div class="overlay archive-overlay" on:click=move |_|{if !busy.get_untracked(){on_close.call(());}}>
        <section class="modal archive-review" role="dialog" aria-modal="true" aria-label=move ||j(locale.get(),"Research archive","研究归档") data-testid="archive-review" on:click=|ev|ev.stop_propagation()>
            <header><div><small>{j(locale.get(),"RESEARCH NOTEBOOK","研究实验记录本")}</small><h2>{move ||if frozen.get(){j(locale.get(),"Archived milestone","已归档研究节点")}else{j(locale.get(),"Review this research archive","确认研究归档")}}</h2></div>
                <button class="icon-btn" data-testid="archive-minimize" title=move ||j(locale.get(),"Minimize archive","最小化归档") aria-label=move ||j(locale.get(),"Minimize archive","最小化归档") on:click=move |_|minimized.set(true)>{compose_icon("minus")}</button>
                <button class="icon-btn" aria-label=move ||j(locale.get(),"Close archive","关闭归档") disabled=move ||busy.get() on:click=move |_|on_close.call(())>{compose_icon("close")}</button></header>
            <div class="archive-body">
                {move ||busy.get().then(||view!{<p role="status">{j(locale.get(),"Preparing and saving research materials…","正在整理或保存研究材料…")}</p>})}
                {move ||(!error.get().is_empty()).then(||view!{<p class="archive-error" role="alert">{localize_backend(locale.get(), &error.get())}</p>})}
                {move ||record.get().map(|a|{
                    let readonly=a.frozen_at.is_some();

                    view!{
                        <div class="archive-layout"><div class="archive-content">
                        <label>{j(locale.get(),"Milestone title","节点标题")}<input data-testid="archive-title" readonly=readonly prop:value=a.title on:input=move |e|{accepted.set(false);edited.update(|a|if let Some(a)=a{a.title=event_target_value(&e);});}/></label>
                        <label>{j(locale.get(),"Question, findings, limitations and decisions","研究问题、结论、局限与选择依据")}<textarea data-testid="archive-report" class="archive-report" readonly=readonly prop:value=a.report on:input=move |e|{accepted.set(false);edited.update(|a|if let Some(a)=a{a.report=event_target_value(&e);});}/></label>
                        <h3>{j(locale.get(),"Recorded operations","整理后的操作代码")}</h3>
                        {a.scripts.into_iter().enumerate().map(|(index,s)|view!{
                            <details class="archive-script"><summary>{s.filename.clone()}</summary>
                                <input aria-label=j(locale.get(),"Script filename","脚本文件名") readonly=readonly prop:value=s.filename on:input=move |e|{accepted.set(false);edited.update(|a|if let Some(a)=a{a.scripts[index].filename=event_target_value(&e);});}/>
                                <textarea aria-label=j(locale.get(),"Script content","脚本内容") readonly=readonly prop:value=s.content on:input=move |e|{accepted.set(false);edited.update(|a|if let Some(a)=a{a.scripts[index].content=event_target_value(&e);});}/>
                            </details>}).collect_view()}
                        </div><aside class="archive-files"><h3>{j(locale.get(),"Local materials","本地材料")}</h3>
                        <p>{j(locale.get(),"Snapshots preserve the reviewed bytes. References keep the original location. Remote files are excluded.","保存副本会固定当前文件内容；原位引用保留现有路径。远程文件不在清理范围内。")}</p>
                        {a.files.into_iter().enumerate().map(|(index,f)|{
                            let snapshot=f.snapshot_path.clone();let name=f.path.clone();
                            view!{<div class="archive-file" data-testid="archive-file"><strong>{f.path}</strong><small>{format!("{} bytes · {}",f.size_bytes,f.reason)}</small>
                                <select aria-label=format!("{} · {}",j(locale.get(),"File action","文件处理"),name) disabled=readonly prop:value=f.action on:change=move |e|{accepted.set(false);edited.update(|a|if let Some(a)=a{a.files[index].action=event_target_value(&e);});}>
                                    <option value="snapshot">{j(locale.get(),"Keep snapshot","保留副本")}</option><option value="reference">{j(locale.get(),"Keep in place","原位保留")}</option><option value="delete" disabled=!f.can_delete>{j(locale.get(),"Permanently delete","永久删除")}</option>
                                </select>
                                {(!f.cleanup_status.is_empty()).then(||view!{<small>{f.cleanup_status}</small>})}
                                {snapshot.map(|path|view!{<button class="btn-ghost" on:click=move |_|on_file.call((path.clone(),name.clone(),crate::text::file_kind(&name).unwrap_or("text").into()))>{j(locale.get(),"Open saved material","查看归档材料")}</button>})}
                            </div>}
                        }).collect_view()}
                        <p class="archive-delete-total" data-testid="archive-delete-total">{move ||format!("{}: {} bytes",j(locale.get(),"Selected for permanent deletion","将永久删除"),edited.with(|a|a.as_ref().map(|a|a.files.iter().filter(|f|f.action=="delete").map(|f|f.size_bytes).sum::<u64>()).unwrap_or(0)))}</p>
                        {a.warnings.into_iter().map(|w|view!{<p class="archive-note">{w}</p>}).collect_view()}
                        </aside></div>
                    }
                })}
            </div>
            <footer>
                {move ||if frozen.get(){view!{
                    <button class="btn-ghost" disabled=move ||busy.get() on:click=move |_|on_notebook.call(frame.get_value())>{j(locale.get(),"Original notebook","查看原始记录")}</button>
                    <button class="btn-ghost" disabled=move ||busy.get() on:click=retry>{j(locale.get(),"Retry incomplete cleanup","重试未完成清理")}</button>
                    <button class="btn-primary" disabled=move ||busy.get() on:click=continue_research>{j(locale.get(),"Continue research","继续研究")}</button>
                }.into_view()}else{view!{
                    <label class="archive-consent"><input type="checkbox" data-testid="archive-consent" prop:checked=move ||accepted.get() disabled=move ||busy.get() on:change=move |e|accepted.set(event_target_checked(&e))/>{j(locale.get(),"I reviewed the materials. This locks the notebook and permanently deletes the selected files.","我已检查归档材料。确认后会话将只读，所选文件立即永久删除，无法撤销。")}</label>
                    <button class="btn-ghost" disabled=move ||busy.get() on:click=move |_|prepare.call(())>{j(locale.get(),"Regenerate draft","重新整理")}</button>
                    <button class="btn-primary" data-testid="archive-confirm" disabled=move ||busy.get() || !accepted.get() ||record.get().is_none() on:click=confirm>{j(locale.get(),"Archive and clean up","确认归档并清理")}</button>
                }.into_view()}}
            </footer>
        </section></div>
        }.into_view()}}
    }
}
