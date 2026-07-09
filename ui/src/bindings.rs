//! The single JS boundary for the UI.
//!
//! Every `wasm_bindgen` extern (Tauri `invoke`/`listen`, preview mounting,
//! uploads, highlight.js, chat autoscroll) lives here so the rest of the crate
//! never touches raw `JsValue` FFI. Callers depend on these thin wrappers, not
//! on the JS module layout — keep new JS-facing calls in this file.

use leptos::spawn_local;
use wasm_bindgen::prelude::*;

/// DOM id of the chat scroll container (owned by the chat view markup).
pub(crate) const CHAT_SCROLLER_ID: &str = "chat-scroller";
/// DOM id of the chat thread content inside the scroller.
pub(crate) const CHAT_THREAD_ID: &str = "chat-thread";

#[wasm_bindgen(module = "/src/highlight.js")]
extern "C" {
    async fn highlight_by_id(id: &str) -> JsValue;
}

#[wasm_bindgen(module = "/src/api.js")]
extern "C" {
    pub(crate) async fn invoke(cmd: &str, args: JsValue) -> JsValue;
    #[wasm_bindgen(catch, js_name = invoke_strict)]
    pub(crate) async fn invoke_checked(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = invoke_timeout)]
    pub(crate) async fn invoke_timeout(cmd: &str, args: JsValue, timeout_ms: u32) -> Result<JsValue, JsValue>;
    pub(crate) async fn listen(event: &str, cb: &js_sys::Function) -> JsValue;
    pub(crate) async fn mount_preview(kind: &str, el_id: &str, payload: &str) -> JsValue;
    #[wasm_bindgen(js_name = pasted_image_count)]
    pub(crate) fn pasted_image_count(event: JsValue) -> usize;
    #[wasm_bindgen(js_name = drag_has_files)]
    pub(crate) fn drag_has_files(event: JsValue) -> bool;
    #[wasm_bindgen(js_name = set_drag_copy)]
    pub(crate) fn set_drag_copy(event: JsValue);
    pub(crate) async fn upload_files(files: JsValue) -> JsValue;
    #[wasm_bindgen(js_name = upload_pasted_images)]
    pub(crate) async fn upload_pasted_images(event: JsValue) -> JsValue;
    #[wasm_bindgen(js_name = native_drop_in_composer)]
    pub(crate) fn native_drop_in_composer(payload: JsValue) -> bool;
    #[wasm_bindgen(js_name = upload_input_files)]
    pub(crate) async fn upload_input_files(input_id: &str) -> JsValue;
}

#[wasm_bindgen(module = "/src/scroll.js")]
extern "C" {
    fn attach_chat_scroll(scroller_id: &str, content_id: &str);
    fn notify_chat_scroll(scroller_id: &str);
    fn force_chat_scroll_bottom(scroller_id: &str);
}

/// Bind the chat scroller so it keeps pinned to the bottom as content grows.
pub(crate) fn attach_chat_autoscroll() {
    attach_chat_scroll(CHAT_SCROLLER_ID, CHAT_THREAD_ID);
}

/// Nudge the chat view to follow new content (respects the user's scroll-up).
pub(crate) fn schedule_chat_follow() {
    notify_chat_scroll(CHAT_SCROLLER_ID);
}

/// Force the chat view to jump to the bottom (e.g. after switching sessions).
pub(crate) fn force_chat_bottom() {
    force_chat_scroll_bottom(CHAT_SCROLLER_ID);
}

/// Syntax-highlight the code block with the given DOM id, once it is mounted.
pub(crate) fn schedule_highlight(id: String) {
    spawn_local(async move {
        let _ = highlight_by_id(&id).await;
    });
}

/// Open an http(s)/mailto/tel link in the OS default handler (not the app webview).
pub(crate) fn open_external_url(url: String) {
    spawn_local(async move {
        let args = serde_wasm_bindgen::to_value(&serde_json::json!({ "url": url }))
            .unwrap_or(JsValue::UNDEFINED);
        let _ = invoke("open_external_url", args).await;
    });
}
