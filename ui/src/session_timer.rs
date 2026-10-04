use crate::app_support::compose_icon;
use crate::dto::{ChatItem, ScheduleRecord, TimerTurnRemoval};
use crate::i18n::Locale;
use crate::research_journey::{call, clock, j};
use leptos::*;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};

pub(crate) fn listen<T: serde::de::DeserializeOwned + 'static>(
    name: &'static str,
    callback: Callback<T>,
) {
    let closed = store_value(false);
    let unlisten = store_value(None::<js_sys::Function>);
    let cb = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        if let Ok(value) = serde_wasm_bindgen::from_value::<T>(event) {
            callback.call(value);
        }
    });
    let function = cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    spawn_local(async move {
        let result = crate::bindings::listen(name, &function).await;
        if let Some(f) = result.dyn_ref::<js_sys::Function>() {
            if closed.try_get_value().unwrap_or(true) {
                let _ = f.call0(&JsValue::NULL);
            } else {
                unlisten.set_value(Some(f.clone()));
            }
        }
    });
    on_cleanup(move || {
        closed.set_value(true);
        if let Some(f) = unlisten.get_value() {
            let _ = f.call0(&JsValue::NULL);
        }
        drop(cb);
    });
}

pub(crate) fn duration(seconds: i64) -> String {
    if seconds % 86400 == 0 {
        format!("{}d", seconds / 86400)
    } else if seconds % 3600 == 0 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}m", seconds / 60)
    }
}

pub(crate) fn label(timer: &ScheduleRecord, loc: Locale) -> String {
    if !timer.enabled {
        return j(loc, "Timer paused", "定时任务已暂停").into();
    }
    format!(
        "{} {} · {} {}",
        j(loc, "Every", "每"),
        duration(timer.interval_secs),
        j(loc, "Next", "下次"),
        clock(timer.next_run_at)
    )
}

/// User indexes are absolute, even when only the last transcript page is loaded.
pub(crate) fn remove_turn(items: &mut Vec<ChatItem>, offset: usize, removal: &TimerTurnRemoval) {
    let mut index = offset;
    let mut remove =
        offset > removal.first_user_index && offset < removal.first_user_index + removal.user_count;
    items.retain(|item| {
        if matches!(item, ChatItem::User(_)) {
            remove = index >= removal.first_user_index && index < removal.first_user_index + removal.user_count;
            index += 1;
        }
        if matches!(item, ChatItem::QueuedUser { .. }) { return true }
        if matches!(item, ChatItem::Compaction { epoch: Some(epoch), .. } if *epoch > removal.base_epoch as u64) { return false }
        !remove
    });
}

#[component]
pub(crate) fn SessionTimerPanel(
    locale: RwSignal<Locale>,
    session_id: String,
    timer: RwSignal<Option<ScheduleRecord>>,
    active_session: RwSignal<Option<String>>,
    refresh: RwSignal<u64>,
    on_close: Callback<()>,
) -> impl IntoView {
    let initial = timer.get_untracked();
    let interval = create_rw_signal(
        initial
            .as_ref()
            .map(|s| duration(s.interval_secs))
            .unwrap_or_else(|| "1h".into()),
    );
    let prompt = create_rw_signal(
        initial
            .as_ref()
            .map(|s| s.prompt.clone())
            .unwrap_or_default(),
    );
    let busy = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);
    let last_error = create_rw_signal(None::<String>);
    create_effect(move |_| {
        if let Some(timer) = timer.get() {
            spawn_local(async move {
                if let Ok(runs) = call::<Vec<crate::dto::ScheduleRunRecord>>(
                    "list_schedule_runs",
                    serde_json::json!({"id":timer.id,"limit":1}),
                )
                .await
                {
                    last_error.try_set(runs.first().and_then(|run| run.error.clone()));
                }
            });
        }
    });
    let session_id = store_value(session_id);
    let save = move |_| {
        if busy.get_untracked() {
            return;
        }
        let expression = format!("{} {}", interval.get_untracked(), prompt.get_untracked());
        if crate::dto::parse_timer_expression(&expression).is_err() {
            error.set(Some(
                j(
                    locale.get_untracked(),
                    "Use 1m–365d and a nonempty prompt (for example: 1h).",
                    "请输入 1m–365d 的间隔和非空提示词，例如 1h。",
                )
                .into(),
            ));
            return;
        }
        busy.set(true);
        error.set(None);
        let target = session_id.get_value();
        spawn_local(async move {
            match call::<ScheduleRecord>(
                "set_session_timer",
                serde_json::json!({"sessionId":target,"expression":expression}),
            )
            .await
            {
                Ok(value) => {
                    if active_session.try_get_untracked().flatten().as_deref() == Some(&target) {
                        timer.set(Some(value));
                    }
                    refresh.update(|n| *n = n.wrapping_add(1));
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
            busy.try_set(false);
        });
    };
    let act = Callback::new(move |delete: bool| {
        let Some(current) = timer.get_untracked() else {
            return;
        };
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let result = if delete {
                call::<()>("delete_schedule", serde_json::json!({"id":current.id})).await
            } else {
                call::<()>(
                    "set_schedule_enabled",
                    serde_json::json!({"id":current.id,"enabled":!current.enabled}),
                )
                .await
            };
            let still_current = active_session.try_get_untracked().flatten() == current.frame_id;
            match result {
                Ok(()) if delete => {
                    if still_current {
                        timer.set(None);
                        on_close.call(());
                    }
                    refresh.update(|n| *n = n.wrapping_add(1));
                }
                Ok(()) => {
                    if still_current {
                        let mut current = current;
                        current.enabled = !current.enabled;
                        timer.set(Some(current));
                    }
                    refresh.update(|n| *n = n.wrapping_add(1));
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
            busy.try_set(false);
        });
    });
    view! {
        <div class="session-timer-backdrop" on:click=move |_| on_close.call(())>
            <section class="session-timer-panel" role="dialog" aria-modal="true" aria-label=move || j(locale.get(),"Conversation timer","会话定时任务") data-testid="session-timer-panel" on:click=move |ev| ev.stop_propagation()>
                <header><h3>{compose_icon("clock")}{move || j(locale.get(),"Conversation timer","会话定时任务")}</h3>
                    <button type="button" aria-label=move || j(locale.get(),"Close timer","关闭定时任务") on:click=move |_| on_close.call(())>{compose_icon("close")}</button></header>
                <p>{move || j(locale.get(),"Each run replaces this timer's previous question, tools and answer. Your other messages remain. Runs only while Wisp is open; busy conversations wait until idle.","每次执行替换此定时任务上一轮的提问、工具调用和回答，保留其他聊天。仅在 Wisp 运行时触发，会话忙时等待空闲。")}</p>
                <label>{move || j(locale.get(),"Interval","间隔")}<input data-testid="timer-interval" prop:value=move || interval.get() on:input=move |ev| interval.set(event_target_value(&ev)) placeholder="1h"/></label>
                <label>{move || j(locale.get(),"Prompt","提示词")}<textarea data-testid="timer-prompt" rows="4" prop:value=move || prompt.get() on:input=move |ev| prompt.set(event_target_value(&ev))></textarea></label>
                {move || timer.get().map(|value| view! {<p class="session-timer-status">{label(&value,locale.get())}</p>})}
                {move || error.get().map(|e| view! {<p role="alert">{e}</p>})}
                {move || last_error.get().map(|e| view! {<p role="alert">{j(locale.get(),"Last run failed: ","上次执行失败：")}{e}</p>})}
                <footer>
                    {move || timer.get().map(|current| view! {
                        <button type="button" disabled=move || busy.get() data-testid="timer-toggle" on:click=move |_| act.call(false)>{compose_icon(if current.enabled {"pause"} else {"play"})}{if current.enabled {j(locale.get(),"Pause","暂停")} else {j(locale.get(),"Resume","恢复")}}</button>
                        <button type="button" disabled=move || busy.get() data-testid="timer-delete" on:click=move |_| act.call(true)>{compose_icon("trash")}{move || j(locale.get(),"Cancel timer","取消定时任务")}</button>
                    })}
                    <button type="button" class="btn-primary" data-testid="timer-save" disabled=move || busy.get() on:click=save>{compose_icon("save")}{move || j(locale.get(),"Save","保存")}</button>
                </footer>
            </section>
        </div>
    }
}
