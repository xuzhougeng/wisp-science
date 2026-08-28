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
    async fn highlight_set_code(id: &str, text: &str) -> JsValue;
}

#[wasm_bindgen(module = "/src/pet.js")]
extern "C" {
    #[wasm_bindgen(js_name = sync_pet)]
    pub(crate) fn sync_pet(element_id: &str, config_json: &str);
}

#[wasm_bindgen(module = "/src/api.js")]
extern "C" {
    pub(crate) async fn invoke(cmd: &str, args: JsValue) -> JsValue;
    #[wasm_bindgen(catch, js_name = invoke_strict)]
    pub(crate) async fn invoke_checked(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = invoke_timeout)]
    pub(crate) async fn invoke_timeout(
        cmd: &str,
        args: JsValue,
        timeout_ms: u32,
    ) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = download_app_update)]
    pub(crate) async fn download_app_update(
        callback: &js_sys::Function,
    ) -> Result<JsValue, JsValue>;
    pub(crate) async fn listen(event: &str, cb: &js_sys::Function) -> JsValue;
    #[wasm_bindgen(js_name = listen_current_window)]
    pub(crate) async fn listen_current_window(event: &str, cb: &js_sys::Function) -> JsValue;
    #[wasm_bindgen(js_name = listen_native_file_drop)]
    pub(crate) async fn listen_native_file_drop(cb: &js_sys::Function) -> JsValue;
    pub(crate) async fn mount_preview(kind: &str, el_id: &str, payload: &str) -> JsValue;
    #[wasm_bindgen(js_name = mount_mcp_app)]
    pub(crate) fn mount_mcp_app(instance_id: &str, el_id: &str, payload_json: &str) -> bool;
    #[wasm_bindgen(js_name = park_mcp_app)]
    pub(crate) fn park_mcp_app(instance_id: &str);
    #[wasm_bindgen(catch, js_name = import_motif_dna_file)]
    pub(crate) async fn import_motif_dna_file(instance_id: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = add_workspace_file_to_motif)]
    pub(crate) async fn add_workspace_file_to_motif(
        instance_id: &str,
        path: &str,
    ) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = request_motif_selection)]
    pub(crate) async fn request_motif_selection(instance_id: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = close_mcp_app)]
    pub(crate) fn close_mcp_app(instance_id: &str);
    #[wasm_bindgen(js_name = pasted_image_count)]
    pub(crate) fn pasted_image_count(event: JsValue) -> usize;
    /// Chat media as a cached blob object URL (never a base64 data URL) —
    /// `null` when the file cannot be read, so callers paint their fallback.
    #[wasm_bindgen(js_name = media_url)]
    pub(crate) async fn media_url(path: &str) -> JsValue;
    /// Small canvas-downscaled variant of [`media_url`] for thumbnail-sized
    /// cards, so long histories do not keep full-size decoded bitmaps alive.
    #[wasm_bindgen(js_name = media_thumbnail_url)]
    pub(crate) async fn media_thumbnail_url(path: &str) -> JsValue;
    #[wasm_bindgen(js_name = drag_has_files)]
    pub(crate) fn drag_has_files(event: JsValue) -> bool;
    #[wasm_bindgen(js_name = set_drag_copy)]
    pub(crate) fn set_drag_copy(event: JsValue);
    pub(crate) async fn upload_files(files: JsValue) -> JsValue;
    pub(crate) fn is_windows() -> bool;
    pub(crate) fn is_mac() -> bool;
    pub(crate) async fn window_control(action: &str);
    #[wasm_bindgen(js_name = set_window_title)]
    pub(crate) async fn set_window_title(title: &str);
    pub(crate) async fn start_window_move();
    #[wasm_bindgen(js_name = arm_caption_drag)]
    pub(crate) async fn arm_caption_drag(start_x: f64, start_y: f64);
    #[wasm_bindgen(js_name = upload_pasted_images)]
    pub(crate) async fn upload_pasted_images(event: JsValue) -> JsValue;
    #[wasm_bindgen(js_name = native_drop_in_composer)]
    pub(crate) fn native_drop_in_composer(payload: JsValue) -> bool;
    #[wasm_bindgen(js_name = native_drop_remote_target)]
    pub(crate) fn native_drop_remote_target(payload: JsValue) -> JsValue;
    #[wasm_bindgen(js_name = upload_input_files)]
    pub(crate) async fn upload_input_files(input_id: &str) -> JsValue;
    #[wasm_bindgen(js_name = preview_selection)]
    pub(crate) fn preview_selection(fallback_x: i32, fallback_y: i32) -> String;
    #[wasm_bindgen(js_name = clear_selection)]
    pub(crate) fn clear_selection();
    #[wasm_bindgen(catch, js_name = render_share_png)]
    pub(crate) async fn render_share_png(payload_json: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = snapshot_share_theme)]
    pub(crate) fn snapshot_share_theme() -> String;
    #[wasm_bindgen(catch, js_name = copy_share_pack)]
    pub(crate) async fn copy_share_pack(text: &str, png_base64: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = crop_region_to_upload)]
    pub(crate) async fn crop_region_to_upload(
        host_id: &str,
        left: f64,
        top: f64,
        width: f64,
        height: f64,
    ) -> JsValue;
    #[wasm_bindgen(js_name = attach_cropped_region)]
    pub(crate) fn attach_cropped_region(path: &str, jump_to_chat: bool);
}

#[wasm_bindgen(module = "/src/scroll.js")]
extern "C" {
    fn attach_chat_scroll(scroller_id: &str, content_id: &str);
    fn notify_chat_scroll(scroller_id: &str);
    fn force_chat_scroll_bottom(scroller_id: &str);
    fn switch_chat_scroll(scroller_id: &str, session_id: &str);
    fn preserve_chat_scroll_on_prepend(scroller_id: &str, content_id: &str);
    fn jump_chat_scroll(scroller_id: &str, selector: &str);
    fn follow_run_outputs();
}

#[wasm_bindgen(module = "/src/marks.js")]
extern "C" {
    /// Underline the given saved excerpts (JSON string array) in the transcript.
    pub(crate) fn set_saved_marks(json: &str);
    /// Cancel a pending underline pass scheduled by `set_saved_marks`.
    pub(crate) fn cancel_saved_marks_apply();
    /// Scroll the first occurrence of a saved excerpt into view.
    pub(crate) fn reveal_saved_mark(text: &str);
}

#[wasm_bindgen(module = "/src/terminal.js")]
extern "C" {
    #[wasm_bindgen(js_name = mount_terminal)]
    pub(crate) fn mount_terminal(element_id: &str, session_id: &str);
    #[wasm_bindgen(js_name = set_terminal_active)]
    pub(crate) fn set_terminal_active(element_id: &str, active: bool);
    #[wasm_bindgen(js_name = unmount_terminal)]
    pub(crate) fn unmount_terminal(element_id: &str);
}

/// Bind the chat scroller so it keeps pinned to the bottom as content grows.
pub(crate) fn attach_chat_autoscroll() {
    attach_chat_scroll(CHAT_SCROLLER_ID, CHAT_THREAD_ID);
}

/// Nudge the chat view after a non-transcript layout change (respects scroll-up).
pub(crate) fn schedule_chat_follow() {
    notify_chat_scroll(CHAT_SCROLLER_ID);
}

/// Force the chat view to jump to the bottom (e.g. after switching sessions).
pub(crate) fn force_chat_bottom() {
    force_chat_scroll_bottom(CHAT_SCROLLER_ID);
}

/// Save the previous conversation's position and restore this conversation.
pub(crate) fn restore_chat_session_scroll(session_id: &str) {
    switch_chat_scroll(CHAT_SCROLLER_ID, session_id);
}

/// Re-pin every run output panel to the bottom after a run-list refresh
/// rebuilt the cards (unless the user scrolled that panel up).
pub(crate) fn schedule_run_output_follow() {
    follow_run_outputs();
}

/// Keep the first previously visible transcript row in place after older rows
/// are prepended.
pub(crate) fn preserve_chat_prepend_position() {
    preserve_chat_scroll_on_prepend(CHAT_SCROLLER_ID, CHAT_THREAD_ID);
}

/// Jump to a user turn and pause bottom-follow until the user returns there.
pub(crate) fn jump_chat_to_user(index: usize) {
    jump_chat_scroll(CHAT_SCROLLER_ID, &format!("[data-user-index=\"{index}\"]"));
}

/// Jump to one rendered transcript item by its UI row index.
pub(crate) fn jump_chat_to_item(index: usize) {
    jump_chat_scroll(
        CHAT_SCROLLER_ID,
        &format!("[data-ui-index=\"{index}\"], [data-ui-indices~=\"{index}\"]"),
    );
}

/// Syntax-highlight the code block with the given DOM id, once it is mounted.
pub(crate) fn schedule_highlight(id: String) {
    spawn_local(async move {
        let _ = highlight_by_id(&id).await;
    });
}

/// Replace and re-highlight the editable code node with the given DOM id.
/// The editor owns that node's children, outside the reactive renderer.
pub(crate) fn set_highlighted_code(id: String, text: String) {
    spawn_local(async move {
        let _ = highlight_set_code(&id, &text).await;
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

/// Ask the backend to open the first available browser on its extension
/// manager page; resolves to the `BrowserExtensionSetup` reply.
pub(crate) async fn open_browser_extension_page() -> JsValue {
    invoke("open_browser_extension_page", JsValue::UNDEFINED).await
}
