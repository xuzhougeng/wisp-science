use crate::{compose_icon, i18n::Locale};
use leptos::*;
use wasm_bindgen::{prelude::*, JsCast};

#[wasm_bindgen(module = "/src/chat_find.js")]
extern "C" {
    pub(crate) fn can_find_chat() -> bool;
    pub(crate) fn focus_chat_find();
    fn start_chat_find(report: &js_sys::Function);
    fn query_chat_find(query: &str);
    fn step_chat_find(direction: i32);
    fn stop_chat_find();
}

#[component]
pub(crate) fn ChatFindBar(open: RwSignal<bool>, locale: RwSignal<Locale>) -> impl IntoView {
    let count = create_rw_signal((0_u32, 0_u32));
    let report =
        Closure::<dyn Fn(u32, u32)>::new(move |current, total| count.set((current, total)));
    start_chat_find(report.as_ref().unchecked_ref());
    on_cleanup(move || {
        stop_chat_find();
        drop(report);
    });
    let input = create_node_ref::<html::Input>();
    create_effect(move |_| {
        if let Some(input) = input.get() {
            let _ = input.focus();
        }
    });
    let label = move |en: &'static str, zh: &'static str| {
        if locale.get() == Locale::Zh {
            zh
        } else {
            en
        }
    };
    view! {
        <div class="chat-find" role="search" aria-label=move || label("Find in displayed conversation", "查找当前显示的会话")>
            <input id="chat-find-input" node_ref=input type="search"
                aria-label=move || label("Find in displayed conversation", "查找当前显示的会话")
                placeholder=move || label("Find in conversation", "在会话中查找")
                on:input=move |ev| query_chat_find(&event_target_value(&ev))
                on:keydown=move |ev| {
                    if ev.key() == "Enter" && !crate::ime_composing(&ev) {
                        ev.prevent_default();
                        ev.stop_propagation();
                        step_chat_find(if ev.shift_key() { -1 } else { 1 });
                    }
                } />
            <span class="chat-find-count" role="status" aria-live="polite">{move || {
                let (current, total) = count.get();
                format!("{current} / {total}")
            }}</span>
            <button type="button" aria-label=move || label("Previous match", "上一个匹配")
                disabled=move || count.get().1 == 0 on:click=move |_| step_chat_find(-1)>{compose_icon("chevron-up")}</button>
            <button type="button" aria-label=move || label("Next match", "下一个匹配")
                disabled=move || count.get().1 == 0 on:click=move |_| step_chat_find(1)>{compose_icon("chevron-down")}</button>
            <button type="button" aria-label=move || label("Close find", "关闭查找")
                on:click=move |_| open.set(false)>{compose_icon("close")}</button>
        </div>
    }
}
