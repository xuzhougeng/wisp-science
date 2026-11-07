import { renderScientific } from "/vendor-runtime/scientific-preview.mjs";
import { renderDocx as renderOfficeDocx, renderXlsx as renderOfficeXlsx, renderPptx as renderOfficePptx } from "/vendor-runtime/office-preview-runtime.mjs";
// Tauri v2 shim + scientific preview mounts; Office DOM/worker rendering is shared with native.
export { render_share_png, render_share_png_pages, snapshot_share_theme } from "./share-renderer.js";
import { injectMcpAppCsp, injectMotifWispBridge } from "/mcp_app_protocol.js";
import { isolatedApps, mountIsolatedApp, suspendIsolatedApp, closeIsolatedApp, isolatedAction, useIsolatedHost } from "/mcp_app_isolated.js";

function tauriCore() {
  return window.__TAURI__?.core;
}

function tauriEvent() {
  return window.__TAURI__?.event;
}

let uiHealthStarted = false;
let reportUiHealth = () => {};
const uiHealth = { timerLagMs: 0, longTasks: 0, longestTaskMs: 0, recentLongestTaskMs: 0, transcriptSwitchMs: 0, transcriptRebuilds: 0, transcriptRetries: 0, transcriptItems: 0, transcriptRows: 0, scriptErrors: 0, unhandledRejections: 0, appMessages: 0, firstFault: "", firstInvalidArgs: "" };
const boundedHealthCount = (value) => Math.min(1_000_000, Math.max(0, Math.round(value) || 0));
const FAULT_MAX_CHARS = 160;

/** Keep the first renderer fault's first line for the backend log. Only a Rust
 * panic (location and message, from the panic hook) or a WebAssembly trap
 * qualifies: an ordinary script error may quote user content and stays a count.
 * The packaged app has no console, so this is where a panic location survives. */
export function record_ui_fault(text) {
  if (uiHealth.firstFault) return;
  uiHealth.firstFault = String(text ?? "").split("\n")[0].slice(0, FAULT_MAX_CHARS);
}

function recordTrap(error) {
  if (typeof WebAssembly !== "undefined" && error instanceof WebAssembly.RuntimeError) {
    record_ui_fault(`${error.name}: ${error.message}`);
  }
}

/** Tauri rejects a payload its command cannot deserialize with
 * "invalid args `arg` for command `cmd`: <serde detail>". Keep the first such
 * pair of names, never the detail (it can quote a value), so a renderer/backend
 * argument-shape drift reaches the log even where the caller swallows errors. */
const INVALID_ARGS = /^invalid args `[^`]*` for command `[^`]*`/;
function recordInvalidArgs(error) {
  if (uiHealth.firstInvalidArgs) return;
  const match = INVALID_ARGS.exec(typeof error === "string" ? error : String(error?.message ?? ""));
  if (match) uiHealth.firstInvalidArgs = match[0].slice(0, FAULT_MAX_CHARS);
}

let transcriptSwitch = null;
let transcriptSwitchToken = 0;
export function begin_transcript_switch() {
  const token = ++transcriptSwitchToken;
  transcriptSwitch = { token, started: performance.now(), rebuilds: 0 };
  return token;
}
export function record_transcript_projection(rows) {
  uiHealth.transcriptRows = boundedHealthCount(rows);
  if (transcriptSwitch) transcriptSwitch.rebuilds += 1;
}
export function finish_transcript_switch(token, retries, items) {
  // Include the frame that paints the hydrated list, not just IPC latency.
  requestAnimationFrame(() => requestAnimationFrame(() => {
    if (transcriptSwitch?.token !== token) return;
    uiHealth.transcriptSwitchMs = boundedHealthCount(performance.now() - transcriptSwitch.started);
    uiHealth.transcriptRebuilds = boundedHealthCount(transcriptSwitch.rebuilds);
    uiHealth.transcriptRetries = boundedHealthCount(retries);
    uiHealth.transcriptItems = boundedHealthCount(items);
    transcriptSwitch = null;
    void reportUiHealth();
  }));
}

/** Only numeric performance fields may enter the feedback prompt. */
export function ui_performance_summary() {
  const { longTasks, longestTaskMs, recentLongestTaskMs, transcriptSwitchMs,
    transcriptRebuilds, transcriptRetries, transcriptItems, transcriptRows } = uiHealth;
  return JSON.stringify({ longTasks, longestTaskMs, recentLongestTaskMs, transcriptSwitchMs,
    transcriptRebuilds, transcriptRetries, transcriptItems, transcriptRows });
}

export function report_ui_health() { void reportUiHealth(); }

/** Bounded diagnostics: counters plus the first fault line. The heartbeat is
 * driven by the WASM app timer, so a broken WASM callback cannot be masked by
 * an independent healthy JS timer. */
export function start_ui_health() {
  if (uiHealthStarted) return;
  uiHealthStarted = true;
  window.addEventListener("error", (event) => { uiHealth.scriptErrors = boundedHealthCount(uiHealth.scriptErrors + 1); recordTrap(event.error); });
  window.addEventListener("unhandledrejection", (event) => { uiHealth.unhandledRejections = boundedHealthCount(uiHealth.unhandledRejections + 1); recordTrap(event.reason); });
  if (typeof PerformanceObserver !== "undefined" && PerformanceObserver.supportedEntryTypes?.includes("longtask")) {
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        uiHealth.longTasks = boundedHealthCount(uiHealth.longTasks + 1);
        uiHealth.longestTaskMs = Math.max(uiHealth.longestTaskMs, boundedHealthCount(entry.duration));
        uiHealth.recentLongestTaskMs = Math.max(uiHealth.recentLongestTaskMs, boundedHealthCount(entry.duration));
      }
    }).observe({ type: "longtask" });
  }
  let lastTick = performance.now();
  let inFlight = false;
  const beat = async () => {
    const now = performance.now();
    const timerLagMs = document.visibilityState === "visible" ? boundedHealthCount(now - lastTick - 5000) : 0;
    uiHealth.timerLagMs = Math.max(uiHealth.timerLagMs, timerLagMs);
    lastTick = now;
    if (inFlight || !tauriCore()) return;
    inFlight = true;
    let activeApps = 0;
    let parkedApps = 0;
    for (const instance of [...mcpAppInstances.values(), ...isolatedApps.values()]) {
      if (instance.target?.isConnected) activeApps += 1;
      else parkedApps += 1;
    }
    const snapshot = { ...uiHealth, activeApps, parkedApps,
      mediaBlobUrls, mediaBlobBytes, mediaOwners: mediaOwners.size,
      dragOverlays: document.querySelectorAll(".drag-overlay").length };
    uiHealth.recentLongestTaskMs = 0;
    // Counters are cumulative and bounded: a once-per-minute backend log cannot
    // miss a brief error or message burst between its sampled heartbeats.
    try { await invoke_timeout("ui_heartbeat", { snapshot }, 4000); }
    catch { uiHealth.recentLongestTaskMs = Math.max(uiHealth.recentLongestTaskMs, snapshot.recentLongestTaskMs); }
    finally { inFlight = false; }
  };
  reportUiHealth = beat;
  document.addEventListener("visibilitychange", () => { lastTick = performance.now(); });
  window.addEventListener("focus", () => { lastTick = performance.now(); });
  void beat();
}

export function is_windows() {
  return navigator.userAgent.includes("Windows");
}

export function is_mac() {
  const userAgent = navigator.userAgent || "";
  const platform = navigator.platform || "";
  return userAgent.includes("Macintosh")
    || userAgent.includes("Mac OS X")
    || platform.startsWith("Mac");
}

export async function window_control(action) {
  const current = window.__TAURI__?.window?.getCurrentWindow?.();
  if (!current) return;
  if (action === "minimize") return current.minimize();
  if (action === "toggle-maximize") return current.toggleMaximize();
  if (action === "close") return current.close();
}

/** Native window title (taskbar / Alt-Tab / macOS title bar) plus `document.title`. */
export async function set_window_title(title) {
  if (typeof document !== "undefined") {
    document.title = title;
  }
  const current = window.__TAURI__?.window?.getCurrentWindow?.();
  if (!current?.setTitle) return;
  try {
    await current.setTitle(title);
  } catch {
    // Browser mock or missing capability — `document.title` still updated.
  }
}

/** Caption-style move so Windows Aero Snap / edge snap can engage. */
export async function start_window_move() {
  const core = tauriCore();
  if (!core) return;
  try {
    return await core.invoke("start_window_move");
  } catch (err) {
    const current = window.__TAURI__?.window?.getCurrentWindow?.();
    return current?.startDragging?.();
  }
}

/** Typical Windows `SM_CXDRAG` / `SM_CYDRAG`. Keep in sync with `window_titlebar.rs`. */
export const CAPTION_DRAG_THRESHOLD_PX = 4;

/**
 * Wait for the pointer to move past the drag threshold before starting a
 * caption move. A bare mousedown must not send `SC_MOVE`, or Windows eats the
 * second click of a title-bar double-click (maximize / restore).
 */
export async function arm_caption_drag(startX, startY) {
  await new Promise((resolve) => {
    let settled = false;
    const finish = (startMove) => {
      if (settled) return;
      settled = true;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      if (startMove) void start_window_move();
      resolve();
    };
    const onMove = (event) => {
      if (
        Math.abs(event.clientX - startX) >= CAPTION_DRAG_THRESHOLD_PX
        || Math.abs(event.clientY - startY) >= CAPTION_DRAG_THRESHOLD_PX
      ) {
        finish(true);
      }
    };
    const onUp = () => finish(false);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  });
}

function missingBridgeError(cmd) {
  return new Error(`Tauri bridge is not available while calling ${cmd}. Open the app with 'cargo tauri dev', not the raw Trunk URL.`);
}

export async function invoke(cmd, args) {
  const core = tauriCore();
  if (!core) {
    console.error(missingBridgeError(cmd));
    return null;
  }
  try {
    return await invoke_strict(cmd, args);
  } catch (err) {
    console.error(`Tauri command failed: ${cmd}`, err);
    return null;
  }
}

// The optimistic queue card can be clicked before enqueue_turn finishes its
// backend validation. Preserve that action until this exact item exists;
// unrelated sessions/items and ordinary invokes must remain independent.
const pendingEnqueues = new Map();

// Every wasm invoke (`invoke`, `invoke_checked`, `invoke_timeout`) routes here.
export async function invoke_strict(cmd, args) {
  try {
    return await invokeTauri(cmd, args);
  } catch (error) {
    recordInvalidArgs(error);
    throw error;
  }
}

async function invokeTauri(cmd, args) {
  const core = tauriCore();
  if (!core) {
    throw missingBridgeError(cmd);
  }
  if (cmd === "enqueue_turn" || cmd === "queued_turn_action") {
    const arg = (key) => args instanceof Map ? args.get(key) : args?.[key];
    const key = JSON.stringify([arg("sessionId"), arg("id")]);
    if (cmd === "enqueue_turn") {
      const pending = Promise.resolve().then(() => core.invoke(cmd, args ?? {}));
      pendingEnqueues.set(key, pending);
      try {
        return await pending;
      } finally {
        if (pendingEnqueues.get(key) === pending) pendingEnqueues.delete(key);
      }
    }
    const pending = pendingEnqueues.get(key);
    if (pending) await pending;
  }
  return core.invoke(cmd, args ?? {});
}

export async function invoke_timeout(cmd, args, timeoutMs) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`Request timed out after ${Math.round(timeoutMs / 1000)}s`)), timeoutMs);
  });
  try {
    return await Promise.race([invoke_strict(cmd, args), timeout]);
  } finally {
    clearTimeout(timer);
  }
}

export async function download_app_update(callback) {
  const core = tauriCore();
  if (!core?.Channel) {
    throw missingBridgeError("download_update");
  }
  const onEvent = new core.Channel();
  onEvent.onmessage = callback;
  return core.invoke("download_update", { onEvent });
}

function fileToBase64(file) {
  // Keep in sync with MAX_UPLOAD_BYTES in src-tauri/src/artifact_commands.rs.
  const maxBytes = 100 * 1024 * 1024;
  if (file.size > maxBytes) {
    return Promise.reject(new Error(`file exceeds ${maxBytes} byte limit`));
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = reader.result;
      if (typeof dataUrl !== "string") {
        reject(new Error("Failed to read file"));
        return;
      }
      const comma = dataUrl.indexOf(",");
      resolve(comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl);
    };
    reader.onerror = () => reject(reader.error || new Error("Failed to read file"));
    reader.readAsDataURL(file);
  });
}

/** @param {FileList|File[]} files */
export async function upload_files(files) {
  const list = Array.from(files || []);
  const results = [];
  for (const file of list) {
    try {
      const data_base64 = await fileToBase64(file);
      // Tauri v2 expects camelCase arg keys (maps to snake_case `data_base64`).
      const info = await invoke_strict("upload_file", { filename: file.name, dataBase64: data_base64 });
      results.push({ ok: true, info });
    } catch (err) {
      results.push({
        ok: false,
        filename: file.name,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }
  return results;
}

/**
 * Crop a rectangular region (given as fractions 0..1 of the preview host,
 * which the crop layer exactly covers) from the image inside `#hostId` and
 * upload it as a PNG. Content-relative fractions map to the image's natural
 * pixels via getBoundingClientRect, so the crop matches what the rubber-band
 * covered regardless of zoom, scroll, or a transformed modal ancestor.
 * Returns the saved workspace path, or "" on failure.
 * @param {string} hostId @param {number} left @param {number} top @param {number} width @param {number} height
 */
export async function crop_region_to_upload(hostId, left, top, width, height) {
  const host = document.getElementById(hostId);
  const img = host?.querySelector("img.rp-img");
  if (!img || !img.naturalWidth) return "";
  const hostRect = host.getBoundingClientRect();
  const rect = img.getBoundingClientRect();
  if (rect.width < 1 || rect.height < 1) return "";
  // The browser emits a click after the crop's pointerup. Do not return the
  // path (which mounts action buttons) until that click has fully dispatched.
  const scaleX = img.naturalWidth / rect.width;
  const scaleY = img.naturalHeight / rect.height;
  let sx = (hostRect.left + left * hostRect.width - rect.left) * scaleX;
  let sy = (hostRect.top + top * hostRect.height - rect.top) * scaleY;
  let sw = width * hostRect.width * scaleX;
  let sh = height * hostRect.height * scaleY;
  // Clamp the source rect to the image bounds.
  sx = Math.max(0, Math.min(sx, img.naturalWidth));
  sy = Math.max(0, Math.min(sy, img.naturalHeight));
  sw = Math.max(1, Math.min(sw, img.naturalWidth - sx));
  sh = Math.max(1, Math.min(sh, img.naturalHeight - sy));
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(sw);
  canvas.height = Math.round(sh);
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  ctx.drawImage(img, sx, sy, sw, sh, 0, 0, canvas.width, canvas.height);
  const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) return "";
  const stamp = new Date().toISOString().replace(/\D/g, "").slice(0, 14);
  const file = new File([blob], `region_${stamp}.png`, { type: "image/png" });
  const results = await upload_files([file]);
  const ok = results.find((r) => r.ok && r.info);
  const path = ok?.info?.path;
  if (!path) return "";
  return String(path);
}

/**
 * Copy a social-share pack to the clipboard. `text` becomes text/plain and
 * `pngBase64` (no data-URL prefix) becomes image/png. Either side may be
 * empty. Some host apps still paste only one of the two flavors.
 * @param {string} text
 * @param {string} pngBase64
 */
export async function copy_share_pack(text, pngBase64) {
  const record = {};
  if (typeof text === "string" && text.length > 0) {
    record["text/plain"] = new Blob([text], { type: "text/plain" });
  }
  if (typeof pngBase64 === "string" && pngBase64.length > 0) {
    const binary = atob(pngBase64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) {
      bytes[i] = binary.charCodeAt(i);
    }
    record["image/png"] = new Blob([bytes], { type: "image/png" });
  }
  if (Object.keys(record).length === 0) {
    throw new Error("nothing to copy");
  }
  await navigator.clipboard.write([new ClipboardItem(record)]);
}

/** Attach an uploaded crop, optionally returning from its preview to chat. */
export function attach_cropped_region(path, jumpToChat) {
  window.dispatchEvent(new CustomEvent("wisp:region-attach", {
    detail: { path: String(path), jumpToChat: Boolean(jumpToChat) },
  }));
}

function pastedImageName(file, index) {
  const ext = {
    "image/jpeg": "jpg",
    "image/png": "png",
    "image/gif": "gif",
    "image/webp": "webp",
    "image/svg+xml": "svg",
  }[file.type] || "png";
  const stamp = new Date().toISOString().replace(/\D/g, "").slice(0, 14);
  return `pasted_image_${stamp}_${index + 1}.${ext}`;
}

export function paste_has_files(event) {
  if (event?.pasteSnapshot) return event.hasFiles;
  const data = event?.clipboardData;
  return Array.from(data?.types || []).includes("Files")
    || /^file:\/\//m.test(data?.getData("text/uri-list") || "");
}

// Snapshot while the event's data store is readable, before waiting on IPC.
export function clipboard_paste_snapshot(event) {
  return {
    pasteSnapshot: true,
    hasFiles: paste_has_files(event),
    fileUris: event?.clipboardData?.getData("text/uri-list") || "",
    images: pastedImageFiles(event),
  };
}

// Share the native read between keydown (WebView2 may emit no paste event for
// CF_HDROP) and paste. Never retain clipboard paths beyond the current gesture.
let clipboardRead = null;
export async function clipboard_file_paths(event) {
  const uris = event?.fileUris ?? event?.clipboardData?.getData("text/uri-list") ?? "";
  if (!event || !clipboardRead) {
    const read = tauriCore()?.invoke("read_clipboard_file_paths", {}) ?? Promise.resolve([]);
    clipboardRead = read;
    setTimeout(() => { if (clipboardRead === read) clipboardRead = null; }, 100);
  }
  const nativePaths = await clipboardRead;
  if (Array.isArray(nativePaths) && nativePaths.length) return nativePaths;
  return uris.split(/\r?\n/).filter(line => line.startsWith("file://")).flatMap(line => {
    try {
      const url = new URL(line);
      let path = decodeURIComponent(url.pathname);
      if (url.hostname && url.hostname !== "localhost") path = `//${url.hostname}${path}`;
      else if (/^\/[a-z]:\//i.test(path)) path = path.slice(1);
      return [path];
    } catch { return []; }
  });
}

function pastedImageFiles(event) {
  if (event?.pasteSnapshot) return event.images;
  const data = event?.clipboardData;
  if (!data) return [];
  const items = Array.from(data.items || []);
  const files = items.length
    ? items.filter((item) => item.kind === "file" && item.type?.startsWith("image/")).map((item) => item.getAsFile()).filter(Boolean)
    : Array.from(data.files || []).filter((file) => file.type?.startsWith("image/"));
  return files.map((file, i) => new File([file], pastedImageName(file, i), { type: file.type || "image/png" }));
}

export function pasted_image_count(event) {
  return pastedImageFiles(event).length;
}

export async function upload_pasted_images(event) {
  const files = pastedImageFiles(event);
  if (!files.length) return [];
  return upload_files(files);
}

function dragDataHasFiles(event) {
  const dt = event?.dataTransfer;
  if (!dt) return false;
  const types = Array.from(dt.types || []);
  if (types.includes("Files")) return true;
  if (dt.items && Array.from(dt.items).some((item) => item.kind === "file")) return true;
  return !!dt.files?.length;
}

export function drag_has_files(event) {
  return dragDataHasFiles(event);
}

export function set_drag_copy(event) {
  const dt = event?.dataTransfer;
  if (!dt) return;
  try {
    dt.dropEffect = "copy";
  } catch (_) {
    // Synthetic events may expose a read-only dataTransfer.
  }
}

function nativeDropPointInside(payload, el) {
  if (!el || !payload) return false;
  const rect = el.getBoundingClientRect();
  const scale = window.devicePixelRatio || 1;
  // Tauri reports physical pixels; DOM rectangles use CSS pixels. Testing
  // the unscaled point too can route a composer drop into Files at high DPI.
  const x = Number(payload.x || 0) / scale;
  const y = Number(payload.y || 0) / scale;
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
}

export function native_drop_in_composer(payload) {
  return nativeDropPointInside(payload, document.querySelector(".composer-inner"));
}

/** @returns {{ contextId: string, destinationDir: string } | null} */
export function native_drop_files_target(payload) {
  const panel = document.querySelector(".rp-files");
  if (!nativeDropPointInside(payload, panel)) return null;
  const select = panel.querySelector(".fb-source");
  const pathInput = panel.querySelector(".fb-path-input");
  if (!select) return null;
  const contextId = String(select.value || "");
  if (contextId === "local") {
    return { contextId, destinationDir: panel.querySelector(".fb-path")?.textContent || "." };
  }
  if (!contextId || !pathInput) return null;
  return { contextId, destinationDir: String(pathInput.value || "~") };
}

/** @param {string} inputId */
export async function upload_input_files(inputId) {
  const input = document.getElementById(inputId);
  if (!input?.files?.length) return [];
  const results = await upload_files(input.files);
  input.value = "";
  return results;
}

export async function listen(event, cb) {
  const bus = tauriEvent();
  if (!bus) {
    console.error(new Error(`Tauri event bridge is not available while listening for ${event}.`));
    return () => {};
  }
  return bus.listen(event, (e) => cb(e.payload));
}

// Register navigation events against this exact native window. The top-level
// event.listen API uses an application-wide target, so every open project
// window would otherwise receive an emit_to("proj-*", "open-session", ...)
// event and jump to the same completed conversation.
export async function listen_current_window(event, cb) {
  const current = window.__TAURI__?.window?.getCurrentWindow?.();
  if (!current?.listen) {
    console.error(new Error(`Tauri window event bridge is not available while listening for ${event}.`));
    return () => {};
  }
  return current.listen(event, (e) => cb(e.payload));
}

function normalizeNativeDropPayload(event) {
  const payload = event?.payload ?? event ?? {};
  const position = payload.position ?? {};
  return {
    kind: payload.kind ?? payload.type ?? "",
    paths: Array.isArray(payload.paths) ? payload.paths : [],
    x: Number(payload.x ?? position.x ?? 0),
    y: Number(payload.y ?? position.y ?? 0),
  };
}

const pathBaseName = (path) => String(path).split(/[\\/]/).filter(Boolean).pop() || "";

/**
 * Keep drag-pasteboard paths only when they account for every dropped `File`
 * name, so a pasteboard left over from an earlier drag can never be copied in
 * place of what the user actually dropped. Folders may be missing from
 * `files`, hence "every name is covered" rather than an exact match.
 * @param {string[]} paths @param {string[]} names
 */
export function drag_paths_matching_files(paths, names) {
  if (!Array.isArray(paths) || !paths.length || !names.length) return [];
  const available = new Map();
  for (const path of paths) {
    const key = pathBaseName(path).normalize("NFC");
    available.set(key, (available.get(key) || 0) + 1);
  }
  for (const name of names) {
    const key = String(name).normalize("NFC");
    const left = available.get(key) || 0;
    if (!left) return [];
    available.set(key, left - 1);
  }
  return paths.map(String);
}

// The main window keeps Tauri's native drag handler off (it would swallow
// in-page HTML5 drags such as tab and sidebar reordering), so Finder/Explorer
// drops reach the page only as DOM events whose `File`s carry no path. Over
// the Files panel, translate them into the native payload shape; the drop's
// paths come from the OS drag pasteboard (macOS) and are empty elsewhere.
function listenFilesPanelDomDrop(handle) {
  const hasFiles = (event) => Array.from(event.dataTransfer?.types || []).includes("Files");
  const panelAt = (event) => (event.target instanceof Element ? event.target.closest(".rp-files") : null);
  const point = (event) => {
    const scale = window.devicePixelRatio || 1;
    return { x: event.clientX * scale, y: event.clientY * scale };
  };
  let hovering = false;
  const leave = () => {
    if (!hovering) return;
    hovering = false;
    handle({ kind: "leave", paths: [] });
  };
  const onOver = (event) => {
    if (!hasFiles(event)) return;
    if (!panelAt(event)) { leave(); return; }
    event.preventDefault();
    try { event.dataTransfer.dropEffect = "copy"; } catch (_) { /* read-only in synthetic events */ }
    if (!hovering) {
      hovering = true;
      handle({ kind: "over", paths: [], ...point(event) });
    }
  };
  const onLeave = (event) => {
    if (!hovering || !hasFiles(event)) return;
    // WebKit reports dragleave between children with a null relatedTarget;
    // only a pointer that is no longer over the panel ends the hover.
    const panel = document.querySelector(".rp-files");
    const rect = panel?.getBoundingClientRect();
    if (rect && event.clientX > rect.left && event.clientX < rect.right
      && event.clientY > rect.top && event.clientY < rect.bottom) return;
    leave();
  };
  const onDrop = async (event) => {
    if (!hasFiles(event) || !panelAt(event)) return;
    event.preventDefault();
    hovering = false;
    const at = point(event);
    const names = Array.from(event.dataTransfer?.files || [], (file) => file.name);
    let paths = [];
    try {
      paths = (await tauriCore()?.invoke("read_drag_file_paths", {})) ?? [];
    } catch (err) {
      console.warn("drag pasteboard unavailable", err);
    }
    handle({ kind: "drop", paths: drag_paths_matching_files(paths, names), ...at });
  };
  window.addEventListener("dragenter", onOver);
  window.addEventListener("dragover", onOver);
  window.addEventListener("dragleave", onLeave);
  window.addEventListener("drop", onDrop);
  return () => {
    window.removeEventListener("dragenter", onOver);
    window.removeEventListener("dragover", onOver);
    window.removeEventListener("dragleave", onLeave);
    window.removeEventListener("drop", onDrop);
  };
}

export async function listen_native_file_drop(cb) {
  const unlisten = [];
  const push = (fn) => { if (typeof fn === "function") unlisten.push(fn); };
  const handle = (event) => cb(normalizeNativeDropPayload(event));
  push(listenFilesPanelDomDrop(handle));
  try {
    const current =
      window.__TAURI__?.webviewWindow?.getCurrentWebviewWindow?.() ||
      window.__TAURI__?.window?.getCurrentWindow?.() ||
      window.__TAURI__?.webview?.getCurrentWebview?.();
    if (current?.onDragDropEvent) push(await current.onDragDropEvent(handle));
  } catch (err) {
    console.warn("Tauri native drag/drop listener unavailable", err);
  }
  const bus = tauriEvent();
  if (bus?.listen) {
    try { push(await bus.listen("native-file-drop", handle)); }
    catch (err) { console.warn("custom native-file-drop listener unavailable", err); }
  }
  return () => {
    for (const fn of unlisten) {
      try { fn(); } catch (_) { /* ignore cleanup failures */ }
    }
  };
}

const css = new Set();
function linkCss(href) {
  if (css.has(href)) return;
  const l = document.createElement("link");
  l.rel = "stylesheet";
  l.href = href;
  document.head.appendChild(l);
  css.add(href);
}

let katexMod;
async function katex() {
  if (!katexMod) {
    katexMod = (await import("/vendor-runtime/katex-Dn761jRB.js")).k;
    linkCss("/vendor-runtime/katex-DwwF5kvc.css");
  }
  return katexMod;
}

let pdfjsLib;
async function pdfjs() {
  if (!pdfjsLib) {
    pdfjsLib = import("/vendor-runtime/pdf.min.mjs").then((mod) => {
      // WebView2 does not ship a browser PDF plugin, so PDFs are rendered to
      // canvas with the worker bundled alongside the application.
      mod.GlobalWorkerOptions.workerSrc = "/vendor-runtime/pdf.worker.min.mjs";
      return mod;
    });
  }
  return pdfjsLib;
}

function normalizeRawBytes(value) {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  if (Array.isArray(value)) return Uint8Array.from(value);
  throw new Error("Binary preview command returned an unsupported payload");
}

// Cache slots, in-flight conversions and mounted DOM owners each hold a
// reference. Eviction releases its reference; only the last release revokes
// the URL, so a visible video or a small thumbnail sharing its source survives.
const mediaUrlCache = { entries: new Map(), bytes: 0, limit: 64, maxBytes: 64 * 1024 * 1024 };
const thumbnailCache = { entries: new Map(), bytes: 0, limit: 128, maxBytes: 16 * 1024 * 1024 };
const mediaJobs = new Map();
const thumbnailJobs = new Map();
const mediaOwners = new Map(); // Element -> Map<slot, entry>; actual DOM lifetime, not reactive owner lifetime
let mediaOwnerObserver;
let mediaSweepTimer;
let mediaBlobUrls = 0;
let mediaBlobBytes = 0;

function createMediaEntry(blob) {
  const entry = { url: URL.createObjectURL(blob), bytes: blob.size, refs: 1 };
  mediaBlobUrls += 1;
  mediaBlobBytes += entry.bytes;
  return entry;
}

function retainMedia(entry) {
  entry.refs += 1;
  return entry;
}

function releaseMedia(entry) {
  if (--entry.refs !== 0) return;
  URL.revokeObjectURL(entry.url);
  mediaBlobUrls -= 1;
  mediaBlobBytes -= entry.bytes;
}

function cacheMedia(cache, key, entry) {
  if (cache.entries.has(key)) cache.entries.delete(key);
  else {
    retainMedia(entry);
    cache.bytes += entry.bytes;
  }
  cache.entries.set(key, entry);
  while (cache.entries.size > cache.limit || cache.bytes > cache.maxBytes) {
    const oldest = cache.entries.keys().next().value;
    const removed = cache.entries.get(oldest);
    cache.entries.delete(oldest);
    cache.bytes -= removed.bytes;
    releaseMedia(removed);
  }
}

// Each caller receives one reference, including concurrent callers. The job
// keeps its own reference until every waiter has acquired its result, even if
// other loads evict that result from the cache in the meantime.
async function acquireMedia(cache, jobs, key, produce) {
  const cached = cache.entries.get(key);
  if (cached) {
    retainMedia(cached);
    cacheMedia(cache, key, cached);
    return cached;
  }
  let job = jobs.get(key);
  if (!job) {
    job = { promise: produce(), waiters: 0 };
    jobs.set(key, job);
  }
  job.waiters += 1;
  let entry;
  try {
    entry = await job.promise;
    if (!entry) return null; // failures are retryable and occupy no cache slot
    retainMedia(entry);
    cacheMedia(cache, key, entry);
    return entry;
  } finally {
    if (--job.waiters === 0) {
      jobs.delete(key);
      if (entry) releaseMedia(entry);
    }
  }
}

function sweepMediaOwners() {
  mediaSweepTimer = undefined;
  for (const [owner, entries] of mediaOwners) {
    if (owner.isConnected) continue;
    mediaOwners.delete(owner);
    for (const entry of entries.values()) releaseMedia(entry);
  }
  if (!mediaOwners.size) {
    mediaOwnerObserver?.disconnect();
    mediaOwnerObserver = undefined;
  }
}

async function ownedMediaUrl(path, ownerId, kind, acquire) {
  // A CSR resource may start just before its view is inserted. Yield once;
  // never keep loading for a card which has already left the document.
  let owner = document.getElementById(ownerId);
  if (!owner) {
    await new Promise((resolve) => setTimeout(resolve, 0));
    owner = document.getElementById(ownerId);
  }
  if (!owner) return null;
  const slot = `${kind}:${path}`;
  const existing = mediaOwners.get(owner)?.get(slot);
  if (existing) return existing.url;
  let entry;
  try { entry = await acquire(String(path || "")); }
  catch { return null; }
  if (!entry) return null;
  if (!owner.isConnected) {
    releaseMedia(entry);
    return null;
  }
  let entries = mediaOwners.get(owner);
  if (!entries) mediaOwners.set(owner, entries = new Map());
  const previous = entries.get(slot);
  if (previous) {
    releaseMedia(entry);
    return previous.url;
  }
  entries.set(slot, entry); // transfer the caller's reference to the DOM owner
  if (!mediaOwnerObserver) {
    mediaOwnerObserver = new MutationObserver((records) => {
      if (mediaSweepTimer === undefined && records.some((record) => record.removedNodes.length)) {
        // Coalesce streaming mutations; a node moved within the document keeps
        // its lease. No full-document media scan or permanent polling timer.
        mediaSweepTimer = setTimeout(sweepMediaOwners, 0);
      }
    });
    mediaOwnerObserver.observe(document.documentElement, { childList: true, subtree: true });
  }
  return entry.url;
}

function mediaBytesCommand(path) {
  // Mirrors `previewBytes`'s command selection for the four path spellings.
  if (path.startsWith("artifact-version:")) {
    return { command: "read_artifact_version_bytes", args: { versionId: path.slice("artifact-version:".length) } };
  }
  if (path.startsWith("artifact:")) {
    return { command: "read_artifact_bytes", args: { id: path.slice("artifact:".length) } };
  }
  if (path.startsWith("remote:ssh:")) {
    const withoutPrefix = path.slice("remote:ssh:".length);
    const separator = withoutPrefix.indexOf(":");
    if (separator <= 0 || separator === withoutPrefix.length - 1) {
      throw new Error("Remote media path is invalid");
    }
    return {
      command: "read_remote_file_bytes",
      args: {
        contextId: `ssh:${withoutPrefix.slice(0, separator)}`,
        path: withoutPrefix.slice(separator + 1),
      },
    };
  }
  return { command: "read_file_bytes", args: { path } };
}

function acquireFullMedia(key) {
  return acquireMedia(mediaUrlCache, mediaJobs, key, async () => {
    if (!key || !tauriCore()) return null;
    const { command, args } = mediaBytesCommand(key);
    const bytes = normalizeRawBytes(await tauriCore().invoke(command, args));
    return createMediaEntry(new Blob([bytes], { type: blobMime(bytes) }));
  });
}

/** The unique owner element must remain mounted for the returned URL's use. */
export function media_url(path, ownerId) {
  return ownedMediaUrl(path, ownerId, "full", acquireFullMedia);
}

/** Load unbound historical Markdown images through the project-scoped reader.
 * Keep the exact DOM node across awaits: a late read must never overwrite a
 * replacement message, project, or a newly arrived immutable resource binding.
 */
export async function hydrate_workspace_images(ownerId, unavailable) {
  let owner = document.getElementById(ownerId);
  if (!owner) {
    await new Promise((resolve) => setTimeout(resolve, 0));
    owner = document.getElementById(ownerId);
  }
  if (!owner) return;
  await Promise.all(Array.from(owner.querySelectorAll("img[data-workspace-image-path]"), async (img) => {
    if (img.hasAttribute("data-resource-id") || img.dataset.imageLoading) return;
    img.dataset.imageLoading = "true";
    const path = img.getAttribute("data-workspace-image-path");
    const current = () => owner.isConnected && owner.contains(img);
    try {
      const url = await media_url(path, ownerId);
      if (!current()) return;
      if (!url) throw new Error("Image bytes unavailable");
      img.src = url;
      img.classList.add("resource-inline-image");
      await img.decode();
    } catch {
      if (!current()) return;
      const fallback = document.createElement("span");
      fallback.className = "resource-unresolved";
      fallback.title = path;
      fallback.textContent = `${img.alt || path.split("/").at(-1)} — ${unavailable}`;
      img.replaceWith(fallback);
    }
  }));
}

const THUMB_MAX_EDGE = 384;

export function media_thumbnail_url(path, ownerId) {
  return ownedMediaUrl(path, ownerId, "thumb", (key) => acquireMedia(thumbnailCache, thumbnailJobs, key, async () => {
    const source = await acquireFullMedia(key);
    if (!source) return null;
    try {
      const blob = await downscaleToPngBlob(source.url, THUMB_MAX_EDGE);
      if (blob) return createMediaEntry(blob);
      return retainMedia(source); // small images and decode failures share the source
    } finally {
      releaseMedia(source);
    }
  }));
}

function downscaleToPngBlob(url, maxEdge) {
  return new Promise((resolve) => {
    const img = new Image();
    const finish = (blob) => {
      img.onload = null;
      img.onerror = null;
      img.removeAttribute("src");
      resolve(blob);
    };
    img.onload = () => {
      try {
        const scale = Math.min(1, maxEdge / Math.max(img.naturalWidth, img.naturalHeight));
        if (scale >= 1) { finish(null); return; }
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
        canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
        canvas.getContext("2d").drawImage(img, 0, 0, canvas.width, canvas.height);
        canvas.toBlob((blob) => {
          canvas.width = 0;
          canvas.height = 0;
          finish(blob);
        }, "image/png");
      } catch { finish(null); }
    };
    img.onerror = () => finish(null);
    img.src = url;
  });
}

function blobMime(bytes) {
  // The byte-returning commands omit the MIME; sniff the signatures that
  // matter for chat media. Defaults to application/octet-stream, which still
  // plays in <video> via extension-less blob sniffing in Chromium.
  if (bytes.length >= 12 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return "image/png";
  if (bytes.length >= 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return "image/jpeg";
  if (bytes.length >= 4 && bytes[0] === 0x47 && bytes[1] === 0x49 && bytes[2] === 0x46) return "image/gif";
  if (bytes.length >= 12 && bytes[4] === 0x66 && bytes[5] === 0x74 && bytes[6] === 0x79 && bytes[7] === 0x70) return "video/mp4";
  // "<?xml" or "<svg" plaintext: the SVG fixtures (and SVG artifacts) decode
  // as images only when served with an image/* MIME.
  const head = new TextDecoder().decode(bytes.slice(0, 256)).trimStart().toLowerCase();
  if (head.startsWith("<?xml") || head.startsWith("<svg")) return "image/svg+xml";
  return "application/octet-stream";
}


async function previewBytes(payload) {
  if (payload.bytes) return normalizeRawBytes(payload.bytes);
  if (payload.b64) return base64Bytes(payload.b64);
  const path = String(payload.path || "");
  if (!path) throw new Error("Preview path is empty");
  // 100 MB matches the backend's local preview ceiling (and the upload cap);
  // the backend clamps remote reads to 32 MB on its own.
  const maxBytes = Math.min(Number(payload.maxBytes) || 32 * 1024 * 1024, 100 * 1024 * 1024);

  let command = "read_file_bytes";
  let args = { path, maxBytes };
  if (path.startsWith("artifact-version:")) {
    command = "read_artifact_version_bytes";
    args = { versionId: path.slice("artifact-version:".length), maxBytes };
  } else if (path.startsWith("artifact:")) {
    command = "read_artifact_bytes";
    args = { id: path.slice("artifact:".length), maxBytes };
  } else if (path.startsWith("remote:ssh:")) {
    const withoutPrefix = path.slice("remote:ssh:".length);
    const separator = withoutPrefix.indexOf(":");
    if (separator <= 0 || separator === withoutPrefix.length - 1) {
      throw new Error("Remote preview path is invalid");
    }
    command = "read_remote_file_bytes";
    args = {
      contextId: `ssh:${withoutPrefix.slice(0, separator)}`,
      path: withoutPrefix.slice(separator + 1),
      maxBytes,
    };
  }
  return normalizeRawBytes(await invoke_strict(command, args));
}

function renderDocx(el, payload) { return renderOfficeDocx(el, payload, previewBytes); }
function renderXlsx(el, payload) { return renderOfficeXlsx(el, payload, previewBytes); }
function renderPptx(el, payload) { return renderOfficePptx(el, payload, previewBytes); }

function base64Bytes(value) {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

function pdfPageLabel(template, page, total) {
  return String(template || `Page ${page} of ${total}`)
    .replace("{page}", String(page))
    .replace("{total}", String(total));
}

function cleanupPreview(el) {
  if (typeof el?.__wispPreviewCleanup === "function") {
    try {
      el.__wispPreviewCleanup();
    } catch (error) {
      console.warn("Failed to clean up preview", error);
    }
  }
  delete el.__wispPreviewCleanup;
  delete el.__wispPreviewToken;
}

function pdfNavIcon(direction) {
  const path = direction < 0 ? "m15 18-6-6 6-6" : "m9 18 6-6-6-6";
  return `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${path}"></path></svg>`;
}

/**
 * Read the current text selection if it falls inside a file preview surface
 * (anything tagged `data-file-path`). Returns a JSON string {text, path, x, y}
 * positioned at the selection for a floating quote/annotate toolbar, or "" when
 * there is no usable selection. Kept in JS because it walks the DOM + Selection
 * API, which is far terser here than through web-sys.
 */
export function preview_selection(fallbackX = 0, fallbackY = 0) {
  // The editable source view is a textarea: its selection never appears in
  // window.getSelection(), so read it directly. Coordinates come from the
  // mouseup that raised the popup — a textarea selection has no client rects.
  // A focused textarea without its own file selection (e.g. the composer)
  // falls through to the ordinary DOM-selection path below.
  const active = document.activeElement;
  if (active && active.tagName?.toLowerCase?.() === "textarea") {
    const container = active.closest?.("[data-file-path]");
    const start = active.selectionStart ?? 0;
    const end = active.selectionEnd ?? 0;
    if (container && end > start) {
      const text = active.value.slice(start, end).trim();
      if (text) {
        return JSON.stringify({
          text,
          path: container.getAttribute("data-file-path") || "",
          x: Math.round(fallbackX),
          y: Math.round(fallbackY),
        });
      }
    }
  }
  const sel = window.getSelection?.();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return "";
  const text = sel.toString().trim();
  if (!text) return "";
  const range = sel.getRangeAt(0);
  let node = range.commonAncestorContainer;
  if (node && node.nodeType === 3) node = node.parentElement;
  const container = node && node.closest ? node.closest("[data-file-path]") : null;
  if (!container) return "";
  // A Range's bounding box spans the whole selection and its bottom is the
  // same regardless of drag direction. Anchor to the topmost rendered text
  // fragment instead so an upward selection does not leave the toolbar at the
  // opposite end of a long passage. Empty rects can be emitted around line
  // breaks; fall back to the bounding box for unusual Selection API clients.
  const rect = Array.from(range.getClientRects())
    .filter((candidate) => candidate.width > 0 && candidate.height > 0)
    .reduce((topmost, candidate) => {
      if (!topmost || candidate.top < topmost.top) return candidate;
      if (candidate.top === topmost.top && candidate.left < topmost.left) return candidate;
      return topmost;
    }, null) || range.getBoundingClientRect();
  return JSON.stringify({
    text,
    path: container.getAttribute("data-file-path") || "",
    x: Math.round(rect.left + rect.width / 2),
    y: Math.round(rect.top),
  });
}

/** Drop the active selection once its text has been quoted/annotated. */
export function clear_selection() {
  window.getSelection?.().removeAllRanges();
}

function eventTargetsEditable(target) {
  for (let node = target; node; node = node.parentElement) {
    const tag = node.tagName?.toLowerCase?.();
    if (
      tag === "input" ||
      tag === "textarea" ||
      tag === "select" ||
      node.hasAttribute?.("contenteditable")
    ) {
      return true;
    }
  }
  return false;
}

async function renderPdf(el, payload) {
  cleanupPreview(el);
  const renderToken = Symbol("pdf-preview");
  el.__wispPreviewToken = renderToken;

  const loading = document.createElement("div");
  loading.className = "rp-pdf-loading";
  loading.textContent = payload.loading || "Loading…";
  el.replaceChildren(loading);

  let task;
  let pdf;
  let renderTask;
  let disconnectObserver;
  let resizeObserver;
  let refitTimer;
  let currentPage = 1;
  let rendering = false;
  let disposed = false;
  // Width the current canvas was rasterised for; a resize past it means the page
  // is being upscaled/downscaled by the browser and needs a re-render.
  let renderedFitWidth = null;
  try {
    const bytes = payload.path || payload.b64 || payload.bytes
      ? await previewBytes(payload)
      : null;
    const lib = await pdfjs();
    const source = bytes ? { data: bytes } : payload.url ? { url: payload.url } : null;
    if (!source) throw new Error("PDF data is empty");
    // PDF.js 5.x decodes JPEG2000 (JPXDecode) figures and ICC colors via WASM
    // fetched from wasmUrl; without it the images silently drop while text still
    // renders. The decoders ship in ui/vendor-runtime next to the worker.
    source.wasmUrl = "/vendor-runtime/";

    task = lib.getDocument(source);
    pdf = await task.promise;
    if (!el.isConnected || el.__wispPreviewToken !== renderToken) {
      return;
    }

    const root = document.createElement("div");
    root.className = "rp-pdf";
    root.setAttribute("data-page-count", String(pdf.numPages));

    const toolbar = document.createElement("div");
    toolbar.className = "rp-pdf-toolbar";

    const nav = document.createElement("div");
    nav.className = "rp-pdf-nav";

    const prevButton = document.createElement("button");
    prevButton.type = "button";
    prevButton.className = "rp-pdf-nav-btn";
    prevButton.setAttribute("aria-label", payload.prevPage || "Previous page");
    prevButton.setAttribute(
      "title",
      `${payload.prevPage || "Previous page"} (Page Up)`,
    );
    prevButton.innerHTML = pdfNavIcon(-1);

    const pageIndicator = document.createElement("div");
    pageIndicator.className = "rp-pdf-page-indicator";
    pageIndicator.setAttribute("role", "status");
    pageIndicator.setAttribute("aria-live", "polite");

    const nextButton = document.createElement("button");
    nextButton.type = "button";
    nextButton.className = "rp-pdf-nav-btn";
    nextButton.setAttribute("aria-label", payload.nextPage || "Next page");
    nextButton.setAttribute(
      "title",
      `${payload.nextPage || "Next page"} (Page Down)`,
    );
    nextButton.innerHTML = pdfNavIcon(1);

    // The zoom viewport owns pointer drags for panning; do not let it swallow
    // clicks on the page navigation controls once the preview is zoomed in.
    nav.addEventListener("pointerdown", (event) => event.stopPropagation());
    nav.append(prevButton, pageIndicator, nextButton);

    // Page nav and zoom are one control set, so they share one bar. The zoom bar
    // is Leptos-owned and sits outside .file-preview-zoom-content, which also
    // keeps the nav from scaling with the page. Previews mounted without the
    // zoom wrapper (right pane, plain modal) keep the toolbar inside .rp-pdf.
    const zoomBar = el
      .closest(".file-preview-zoom")
      ?.querySelector(".file-preview-zoom-bar");
    if (zoomBar) {
      zoomBar.prepend(nav);
    } else {
      toolbar.appendChild(nav);
    }

    const viewer = document.createElement("div");
    viewer.className = "rp-pdf-viewer";

    root.append(...(zoomBar ? [viewer] : [toolbar, viewer]));
    el.replaceChildren(root);

    const syncControls = () => {
      root.setAttribute("data-current-page", String(currentPage));
      pageIndicator.textContent = pdfPageLabel(payload.pageLabel, currentPage, pdf.numPages);
      prevButton.disabled = rendering || currentPage <= 1;
      nextButton.disabled = rendering || currentPage >= pdf.numPages;
    };

    const showPageError = (error) => {
      if (error?.name === "RenderingCancelledException" || disposed) {
        return;
      }
      console.error("Failed to render PDF page", error);
      const message = document.createElement("div");
      message.className = "rp-error rp-pdf-error";
      message.textContent = payload.error || "Unable to preview this PDF.";
      el.replaceChildren(message);
      el.__wispPreviewCleanup?.();
    };

    // Fit-to-width base for the page, independent of --preview-zoom: the zoom is
    // a pure CSS multiple of this. Read off the viewer, whose width tracks the
    // pane and not the (possibly zoomed) page inside it.
    const fitWidth = () =>
      Math.max(240, Math.min(viewer.clientWidth || el.clientWidth || 800, 1000));

    const renderPage = async (pageNumber) => {
      rendering = true;
      syncControls();

      const page = await pdf.getPage(pageNumber);
      try {
        // Render at up to 2x the displayed width so text remains crisp on HiDPI
        // screens without making the single-page preview consume unbounded canvas memory.
        const availableWidth = fitWidth();
        renderedFitWidth = availableWidth;
        const pixelRatio = Math.min(window.devicePixelRatio || 1, 2);
        const natural = page.getViewport({ scale: 1 });
        const cssScale = availableWidth / natural.width;
        const viewport = page.getViewport({ scale: cssScale * pixelRatio });
        const wrapper = document.createElement("div");
        wrapper.className = "rp-pdf-page";
        wrapper.dataset.page = String(pageNumber);
        // The page itself is the only thing the zoom scales: availableWidth is
        // the fit-to-width base and --preview-zoom multiplies it. This must be
        // `width`, not `max-width` — .rp-pdf-page is width:100% of the viewer,
        // so a max-width above 100% would never win and zoom-in would no-op.
        wrapper.style.width =
          `calc(${Math.floor(availableWidth)}px * var(--preview-zoom, 1))`;
        wrapper.setAttribute(
          "aria-label",
          pdfPageLabel(payload.pageLabel, pageNumber, pdf.numPages),
        );
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.floor(viewport.width));
        canvas.height = Math.max(1, Math.floor(viewport.height));
        canvas.setAttribute("role", "img");
        wrapper.appendChild(canvas);

        const context = canvas.getContext("2d", { alpha: false });
        if (!context) throw new Error("Canvas is not available");

        renderTask = page.render({ canvasContext: context, viewport });
        await renderTask.promise;
        if (!el.isConnected || el.__wispPreviewToken !== renderToken || disposed) {
          return;
        }

        // Transparent selectable text layer over the canvas, at CSS scale (no
        // pixelRatio) so glyphs align to the displayed page. This is what makes
        // PDF text selectable → "add to chat" (the preview's data-file-path
        // ancestor drives the shared selection popup). Fail-soft: a text-layer
        // error must not blank the rendered page.
        try {
          const cssViewport = page.getViewport({ scale: cssScale });
          const textLayerDiv = document.createElement("div");
          textLayerDiv.className = "rp-pdf-textlayer textLayer";
          textLayerDiv.style.setProperty("--scale-factor", String(cssScale));
          textLayerDiv.style.setProperty(
            "--total-scale-factor",
            `calc(${cssScale} * var(--preview-zoom, 1))`,
          );
          const textLayer = new lib.TextLayer({
            textContentSource: page.streamTextContent(),
            container: textLayerDiv,
            viewport: cssViewport,
          });
          await textLayer.render();
          if (!el.isConnected || el.__wispPreviewToken !== renderToken || disposed) {
            return;
          }
          wrapper.appendChild(textLayerDiv);
        } catch (error) {
          if (error?.name !== "RenderingCancelledException") {
            console.warn("PDF text layer failed", error);
          }
        }

        wrapper.dataset.rendered = "true";
        viewer.replaceChildren(wrapper);
      } finally {
        renderTask = undefined;
        rendering = false;
        syncControls();
        page.cleanup();
      }
    };

    const setPage = (pageNumber) => {
      if (
        rendering ||
        pageNumber < 1 ||
        pageNumber > pdf.numPages ||
        pageNumber === currentPage
      ) {
        return;
      }
      currentPage = pageNumber;
      void renderPage(pageNumber).catch(showPageError);
    };

    const stepPage = (delta) => setPage(currentPage + delta);
    prevButton.addEventListener("click", () => stepPage(-1));
    nextButton.addEventListener("click", () => stepPage(1));

    // Page navigation by keyboard: Page Up/Down and the arrow keys step pages.
    // (Zoom is the wheel gesture, handled by the ZoomableFilePreview wrapper.)
    const onKeyDown = (event) => {
      if (
        event.defaultPrevented ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey ||
        eventTargetsEditable(event.target)
      ) {
        return;
      }
      if (event.key === "PageUp" || event.key === "ArrowUp" || event.key === "ArrowLeft") {
        event.preventDefault();
        stepPage(-1);
      } else if (event.key === "PageDown" || event.key === "ArrowDown" || event.key === "ArrowRight") {
        event.preventDefault();
        stepPage(1);
      }
    };

    if (el.closest(".artifact-modal")) {
      document.addEventListener("keydown", onKeyDown);
    }

    const cleanup = () => {
      if (disposed) return;
      disposed = true;
      // When portalled into the zoom bar the nav lives outside el, so it
      // survives the el.replaceChildren() on the error paths — take it down here.
      nav.remove();
      document.removeEventListener("keydown", onKeyDown);
      clearTimeout(refitTimer);
      resizeObserver?.disconnect();
      disconnectObserver?.disconnect();
      if (renderTask) {
        try {
          renderTask.cancel();
        } catch {
          /* ignore cancellation races */
        }
      }
      if (pdf) {
        const currentPdf = pdf;
        pdf = undefined;
        void currentPdf.destroy().catch((error) => {
          console.warn("Failed to release PDF preview resources", error);
        });
      } else if (task) {
        const currentTask = task;
        task = undefined;
        void currentTask.destroy().catch((error) => {
          console.warn("Failed to release PDF loading task", error);
        });
      }
    };
    el.__wispPreviewCleanup = cleanup;

    const observerTarget = document.body || document.documentElement;
    if (observerTarget) {
      disconnectObserver = new MutationObserver(() => {
        if (!el.isConnected) cleanup();
      });
      disconnectObserver.observe(observerTarget, { childList: true, subtree: true });
    }

    // The canvas is rasterised for one fitWidth, so a pane resize (split toggled,
    // window resized, right pane dragged) otherwise leaves the page frozen at the
    // width it was first rendered at. Debounced; re-queues rather than running
    // while a render is in flight, so two renderTasks never race for the viewer.
    // The initial observation fires harmlessly — by then renderedFitWidth matches.
    const scheduleRefit = () => {
      clearTimeout(refitTimer);
      refitTimer = setTimeout(() => {
        if (disposed || !el.isConnected) return;
        if (rendering) {
          scheduleRefit();
          return;
        }
        if (fitWidth() === renderedFitWidth) return;
        void renderPage(currentPage).catch(showPageError);
      }, 150);
    };
    resizeObserver = new ResizeObserver(scheduleRefit);
    resizeObserver.observe(viewer);

    syncControls();
    await renderPage(currentPage);
  } catch (error) {
    if (error?.name === "RenderingCancelledException") {
      return;
    }
    console.error("Failed to render PDF preview", error);
    el.__wispPreviewCleanup?.();
    if (el.isConnected && el.__wispPreviewToken === renderToken) {
      const message = document.createElement("div");
      message.className = "rp-error rp-pdf-error";
      message.textContent = payload.error || "Unable to preview this PDF.";
      el.replaceChildren(message);
    }
  }
}

function escHtml(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function escAttr(s) {
  return String(s).replace(/&/g, "&amp;").replace(/"/g, "&quot;");
}

function htmlBaseHref(path) {
  if (typeof path !== "string" || !path) return "";
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(path)) {
    return path.replace(/[^/]*$/, "");
  }
  if (path.startsWith("/")) {
    return `file://${path.replace(/[^/]*$/, "")}`;
  }
  return "";
}

function injectBaseHref(html, baseHref) {
  if (!baseHref || /<base\s/i.test(html)) return html;
  const baseTag = `<base href="${escAttr(baseHref)}">`;
  if (/<head(\s[^>]*)?>/i.test(html)) {
    return html.replace(/<head(\s[^>]*)?>/i, (m) => `${m}${baseTag}`);
  }
  return `<!doctype html><html><head>${baseTag}</head><body>${html}</body></html>`;
}

function injectResponsiveHtmlPreview(html, baseHref) {
  const withBase = injectBaseHref(html, baseHref);
  const viewportTag = '<meta name="viewport" content="width=device-width, initial-scale=1">';
  const previewStyle = `<style>
html, body { max-width: 100%; overflow-x: hidden; }
body { margin-left: auto !important; margin-right: auto !important; }
img, svg, canvas, video, iframe, embed, object { max-width: 100% !important; height: auto !important; }
table { max-width: 100%; }
</style>`;
  const resizeScript = `<script>
(() => {
  const setHeight = () => {
    const doc = document.documentElement;
    const body = document.body;
    const height = Math.max(
      doc ? doc.scrollHeight : 0,
      doc ? doc.offsetHeight : 0,
      body ? body.scrollHeight : 0,
      body ? body.offsetHeight : 0
    );
    if (window.frameElement) {
      window.frameElement.style.height = Math.max(height, 320) + "px";
    }
  };
  addEventListener("load", () => {
    setHeight();
    requestAnimationFrame(setHeight);
    setTimeout(setHeight, 60);
  });
  addEventListener("resize", setHeight);
  if (window.ResizeObserver && document.body) {
    const ro = new ResizeObserver(setHeight);
    ro.observe(document.documentElement);
    ro.observe(document.body);
  }
})();
</script>`;
  let out = withBase;
  if (!/<meta\s+name=["']viewport["']/i.test(out)) {
    if (/<head(\s[^>]*)?>/i.test(out)) {
      out = out.replace(/<head(\s[^>]*)?>/i, (m) => `${m}${viewportTag}`);
    } else {
      out = `${viewportTag}${out}`;
    }
  }
  if (/<head(\s[^>]*)?>/i.test(out)) {
    out = out.replace(/<head(\s[^>]*)?>/i, (m) => `${m}${previewStyle}`);
  } else {
    out = `${previewStyle}${out}`;
  }
  if (/<body(\s[^>]*)?>/i.test(out)) {
    out = out.replace(/<\/body>/i, `${resizeScript}</body>`);
    if (!out.includes(resizeScript)) out += resizeScript;
  } else {
    out += resizeScript;
  }
  return out;
}

const mcpAppInstances = new Map();
let mcpAppParkingRoot = null;
let wispAppVersion = "0.0.0";
window.__TAURI__?.app?.getVersion?.().then((version) => { wispAppVersion = version; });

function ensureMcpAppParkingRoot() {
  if (mcpAppParkingRoot?.isConnected) return mcpAppParkingRoot;
  const root = document.createElement("div");
  root.id = "wisp-mcp-app-parking";
  root.setAttribute("aria-hidden", "true");
  Object.assign(root.style, {
    position: "fixed", left: "-10000px", top: "-10000px",
    width: "1px", height: "1px", overflow: "hidden", pointerEvents: "none",
  });
  document.body.appendChild(root);
  mcpAppParkingRoot = root;
  return root;
}

function mcpAppTitle(payload) {
  return payload?.tool?.title
    || payload?.tool?.annotations?.title
    || payload?.tool?.name
    || "MCP App";
}

function mcpAppDimensions(instance) {
  const rect = instance.target?.getBoundingClientRect();
  return {
    width: Math.max(Math.round(rect?.width || instance.frame.clientWidth || 0), 320),
    height: Math.max(Math.round(rect?.height || instance.frame.clientHeight || 0), 320),
  };
}

function createMcpAppInstance(instanceId, payloadJson) {
  const payload = typeof payloadJson === "string" ? JSON.parse(payloadJson) : payloadJson;
  const html = payload?.resource?.text;
  if (typeof html !== "string" || !html) return null;

  const frame = document.createElement("iframe");
  frame.title = mcpAppTitle(payload);
  frame.setAttribute("sandbox", "allow-scripts");
  frame.setAttribute("referrerpolicy", "no-referrer");
  Object.assign(frame.style, { width: "100%", height: "100%", border: "0", background: "#fff" });

  const instance = {
    id: instanceId,
    appName: mcpAppTitle(payload),
    payload,
    payloadJson: typeof payloadJson === "string" ? payloadJson : JSON.stringify(payloadJson),
    frame,
    target: null,
    initialized: false,
    teardownId: 1000000,
    teardownRequestId: null,
    teardownTimer: null,
    resizeObserver: null,
    onMessage: null,
    motifRequestId: 0,
    motifRequests: new Map(),
  };
  const post = (message) => frame.contentWindow?.postMessage(message, "*");
  const clearModelContext = () => invoke("update_mcp_app_context", {
    instanceId,
    appName: instance.appName,
    context: {},
  });
  const hostContext = () => ({
    theme: document.documentElement.dataset.theme === "dark" ? "dark" : "light",
    displayMode: "inline",
    availableDisplayModes: ["inline"],
    containerDimensions: mcpAppDimensions(instance),
    locale: document.documentElement.lang || navigator.language || "en",
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    platform: "desktop",
    userAgent: `wisp-science/${wispAppVersion}`,
    toolInfo: { tool: instance.payload.tool || {} },
  });
  const sendHostContext = () => {
    if (!instance.initialized || !instance.target) return;
    post({
      jsonrpc: "2.0",
      method: "ui/notifications/host-context-changed",
      params: hostContext(),
    });
  };
  const sendData = () => {
    if (!instance.initialized) return;
    post({ jsonrpc: "2.0", method: "ui/notifications/tool-input", params: { arguments: instance.payload.arguments || {} } });
    post({ jsonrpc: "2.0", method: "ui/notifications/tool-result", params: instance.payload.result || { content: [] } });
  };
  const cleanup = () => {
    if (instance.teardownTimer != null) window.clearTimeout(instance.teardownTimer);
    instance.resizeObserver?.disconnect();
    window.removeEventListener("message", instance.onMessage);
    frame.remove();
    if (mcpAppInstances.get(instanceId) === instance) {
      mcpAppInstances.delete(instanceId);
      void clearModelContext();
      // Revoke the host-side serverTools bridge so a later request from a
      // stale iframe fails with a stale-instance error (best-effort).
      void invoke("close_mcp_app", { instanceId });
    }
  };
  const requestTeardown = (reason) => {
    if (instance.initialized && instance.teardownRequestId == null) {
      instance.teardownRequestId = ++instance.teardownId;
      post({
        jsonrpc: "2.0",
        id: instance.teardownRequestId,
        method: "ui/resource-teardown",
        params: { reason },
      });
      instance.teardownTimer = window.setTimeout(cleanup, 500);
    } else {
      cleanup();
    }
  };
  instance.onMessage = (event) => {
    if (event.source !== frame.contentWindow || !event.data || event.data.jsonrpc !== "2.0") return;
    uiHealth.appMessages = boundedHealthCount(uiHealth.appMessages + 1);
    const message = event.data;
    if (message.method?.startsWith("wisp/notifications/motif-")) {
      if (message.method === "wisp/notifications/motif-bridge-ready") {
        if (!instance.initialized) {
          instance.initialized = true;
          sendData();
          sendHostContext();
        }
        return;
      }
      const requestId = message.params?.requestId;
      const pending = instance.motifRequests.get(requestId);
      if (pending) {
        instance.motifRequests.delete(requestId);
        if (message.method === "wisp/notifications/motif-bridge-error") {
          pending.reject(new Error(message.params?.message || "Motif bridge failed."));
        } else {
          pending.resolve(message.params || {});
        }
      }
      return;
    }
    if (message.method === "ui/initialize" && message.id != null) {
      const hostCapabilities = {
        sandbox: { csp: instance.payload?.resource?._meta?.ui?.csp || instance.payload?.resource?._meta?.csp || {} },
        updateModelContext: { text: {} },
      };
      // Only advertise serverTools when this instance still has a live host
      // bridge. Restored/parked Apps without the original MCP connection keep
      // using updateModelContext instead of a false capability.
      void invoke_strict("mcp_app_has_server_tools", { instanceId }).then(
        (available) => {
          if (available) hostCapabilities.serverTools = {};
          post({
            jsonrpc: "2.0",
            id: message.id,
            result: {
              protocolVersion: message.params?.protocolVersion || "2026-01-26",
              hostCapabilities,
              hostInfo: { name: "wisp-science", version: wispAppVersion },
              hostContext: hostContext(),
            },
          });
        },
        () => post({
          jsonrpc: "2.0",
          id: message.id,
          result: {
            protocolVersion: message.params?.protocolVersion || "2026-01-26",
            hostCapabilities,
            hostInfo: { name: "wisp-science", version: wispAppVersion },
            hostContext: hostContext(),
          },
        }),
      );
      return;
    }
    if (message.method === "ui/notifications/initialized") {
      instance.initialized = true;
      sendData();
      sendHostContext();
      return;
    }
    if (message.method === "ping" && message.id != null) {
      post({ jsonrpc: "2.0", id: message.id, result: {} });
      return;
    }
    if (message.method === "tools/list" && message.id != null) {
      void invoke_strict("list_mcp_app_tools", { instanceId }).then(
        (result) => post({ jsonrpc: "2.0", id: message.id, result }),
        (error) => post({
          jsonrpc: "2.0",
          id: message.id,
          error: {
            code: -32603,
            message: (error instanceof Error ? error.message : String(error)).slice(0, 512),
          },
        }),
      );
      return;
    }
    if (message.method === "tools/call" && message.id != null) {
      const params = message.params || {};
      const name = params.name;
      if (typeof name !== "string" || !name) {
        post({
          jsonrpc: "2.0",
          id: message.id,
          error: { code: -32602, message: "tools/call requires a valid 'name' string" },
        });
        return;
      }
      const args = params.arguments
        && typeof params.arguments === "object"
        && !Array.isArray(params.arguments)
        ? params.arguments
        : {};
      void invoke_strict("call_mcp_app_tool", { instanceId, name, arguments: args }).then(
        (result) => post({ jsonrpc: "2.0", id: message.id, result }),
        (error) => post({
          jsonrpc: "2.0",
          id: message.id,
          error: {
            code: -32603,
            message: (error instanceof Error ? error.message : String(error)).slice(0, 512),
          },
        }),
      );
      return;
    }
    if (message.method === "ui/update-model-context" && message.id != null) {
      void invoke_strict("update_mcp_app_context", {
        instanceId,
        appName: instance.appName,
        context: message.params || {},
      }).then(
        () => post({ jsonrpc: "2.0", id: message.id, result: {} }),
        (error) => post({
          jsonrpc: "2.0",
          id: message.id,
          error: {
            code: -32602,
            message: (error instanceof Error ? error.message : String(error)).slice(0, 512),
          },
        }),
      );
      return;
    }
    if (instance.teardownRequestId != null && message.id === instance.teardownRequestId) {
      cleanup();
      return;
    }
    if (message.id != null) {
      post({
        jsonrpc: "2.0",
        id: message.id,
        error: { code: -32601, message: "Capability is not granted by Wisp" },
      });
    }
  };
  instance.requestTeardown = requestTeardown;
  instance.sendHostContext = sendHostContext;
  instance.sendData = sendData;
  window.addEventListener("message", instance.onMessage);
  const bridgedHtml = payload?.tool?.name === "motif_open_workbench"
    ? injectMotifWispBridge(html)
    : html;
  frame.srcdoc = injectMcpAppCsp(bridgedHtml, payload?.resource?._meta);
  mcpAppInstances.set(instanceId, instance);
  return instance;
}

function mcpAppDocumentKey(payload) {
  const uri = String(payload?.resource?.uri || "").split(/[?#]/, 1)[0];
  return `${uri}\n${payload?.resource?.text || ""}`;
}

/** Mount one MCP App inside a host-owned center pane. The app keeps an opaque
 * origin and scripts only; filesystem, forms, popups, top navigation,
 * downloads, and same-origin access remain unavailable. */
const legacyMcpMounts = new Map();
export function mount_mcp_app(instanceId, elId, payloadJson) {
  if (useIsolatedHost()) return mountIsolatedApp(instanceId, elId, payloadJson);
  const target = document.getElementById(elId);
  if (!target) return false;
  const incoming = typeof payloadJson === "string" ? JSON.parse(payloadJson) : payloadJson;
  // Legacy records remain displayable without inventing a connector binding.
  if (!incoming?._wispMcpBinding) return mountLegacyMcpApp(instanceId, elId, payloadJson);
  const ticket = {}; legacyMcpMounts.set(instanceId, ticket);
  // A reconnect may only bind a newly mounted frame, never an old guest callback.
  const old = mcpAppInstances.get(instanceId);
  if (old) {
    mcpAppInstances.delete(instanceId);
    window.removeEventListener("message", old.onMessage);
    old.requestTeardown("primary workspace remount");
  }
  void invoke("prepare_mcp_app", {instanceId}).then((fresh) => {
    if (legacyMcpMounts.get(instanceId) !== ticket || !target.isConnected) return;
    mountLegacyMcpApp(instanceId, elId, fresh ? JSON.stringify(fresh) : payloadJson);
  }).catch((error) => {
    if (legacyMcpMounts.get(instanceId) === ticket && target.isConnected) target.textContent = String(error);
  });
  return true;
}
function mountLegacyMcpApp(instanceId, elId, payloadJson) {
  const target = document.getElementById(elId);
  if (!target) return false;
  let instance = mcpAppInstances.get(instanceId);
  if (instance && instance.payloadJson !== payloadJson) {
    const nextPayload = typeof payloadJson === "string" ? JSON.parse(payloadJson) : payloadJson;
    if (mcpAppDocumentKey(instance.payload) === mcpAppDocumentKey(nextPayload)) {
      instance.payload = nextPayload;
      instance.payloadJson = typeof payloadJson === "string" ? payloadJson : JSON.stringify(payloadJson);
      instance.appName = mcpAppTitle(nextPayload);
      instance.frame.title = instance.appName;
      instance.sendData();
      instance.sendHostContext();
    } else {
      void invoke("update_mcp_app_context", {
        instanceId,
        appName: instance.appName,
        context: {},
      });
      // Detach first so the old iframe's cleanup does not revoke the host
      // bridge the newer presentation just registered under this same id.
      if (mcpAppInstances.get(instanceId) === instance) {
        mcpAppInstances.delete(instanceId);
      }
      instance.requestTeardown("replaced by a newer MCP App presentation");
      instance = null;
    }
  }
  instance ||= createMcpAppInstance(instanceId, payloadJson);
  if (!instance) return false;

  instance.resizeObserver?.disconnect();
  target.replaceChildren(instance.frame);
  instance.target = target;
  instance.resizeObserver = new ResizeObserver(() => instance.sendHostContext());
  instance.resizeObserver.observe(target);
  instance.sendHostContext();
  return true;
}

/** Keep a live iframe attached off-screen while another center tab is active. */
export function park_mcp_app(instanceId) {
  legacyMcpMounts.delete(instanceId);
  if (useIsolatedHost()) return suspendIsolatedApp(instanceId);
  const instance = isolatedApps.get(instanceId) || mcpAppInstances.get(instanceId);
  if (!instance) return;
  instance.resizeObserver?.disconnect();
  instance.target = null;
  ensureMcpAppParkingRoot().appendChild(instance.frame);
}

/** Read one explicitly selected local sequence file and load it into the live
 * Motif instance through the MCP connection that opened the workbench. */
const SNAPGENE_FEATURE_TYPES = Object.freeze({
  cds: "cds",
  gene: "gene",
  promoter: "promoter",
  terminator: "terminator",
  rbs: "rbs",
  rep_origin: "origin",
  origin: "origin",
  primer_bind: "primer_bind",
  protein_bind: "regulatory",
  regulatory: "regulatory",
  enhancer: "enhancer",
  mrna: "mRNA",
  rrna: "rRNA",
  trna: "tRNA",
  ncrna: "ncRNA",
  repeat_region: "repeat_region",
  sig_peptide: "sig_peptide",
  mat_peptide: "mat_peptide",
  transit_peptide: "transit_peptide",
  intron: "intron",
  exon: "exon",
  polya_signal: "polyA_signal",
  misc_feature: "misc_feature",
});

function snapGeneFeatureType(value) {
  const key = String(value || "misc_feature").trim().toLowerCase();
  return SNAPGENE_FEATURE_TYPES[key] || "custom";
}

function snapGenePlainText(value) {
  const text = String(value || "").trim();
  if (!text) return "";
  if (!/[<&]/.test(text)) return text.slice(0, 16_384);
  const document = new DOMParser().parseFromString(text, "text/html");
  return (document.body?.textContent || text.replace(/<[^>]*>/g, " "))
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 16_384);
}

function snapGeneFeatureMetadata(feature, originalType) {
  const metadata = Object.create(null);
  if (originalType) metadata.snapGeneType = originalType;
  for (const qualifier of feature.getElementsByTagName("Q")) {
    const name = qualifier.getAttribute("name")?.trim();
    if (!name || name === "__proto__" || name === "constructor" || name === "prototype") continue;
    const values = [];
    for (const value of qualifier.getElementsByTagName("V")) {
      const raw = value.getAttribute("text")
        ?? value.getAttribute("predef")
        ?? value.getAttribute("int")
        ?? value.getAttribute("real")
        ?? value.getAttribute("bool")
        ?? value.textContent;
      const normalized = snapGenePlainText(raw);
      if (normalized) values.push(normalized);
    }
    if (values.length > 0) metadata[name] = values.length === 1 ? values[0] : values;
  }
  return metadata;
}

function snapGeneSegment(range, sequenceLength, strand) {
  const match = String(range || "").match(/^\s*(\d+)\s*-\s*(\d+)\s*$/);
  if (!match) return [];
  const oneBasedStart = Number(match[1]);
  const oneBasedEnd = Number(match[2]);
  if (!Number.isSafeInteger(oneBasedStart) || !Number.isSafeInteger(oneBasedEnd)
      || oneBasedStart < 1 || oneBasedEnd < 1
      || oneBasedStart > sequenceLength || oneBasedEnd > sequenceLength) return [];
  if (oneBasedStart <= oneBasedEnd) {
    return [{ start: oneBasedStart - 1, end: oneBasedEnd, strand }];
  }
  // Circular features may cross coordinate 1. Motif accepts split subranges.
  return [
    { start: oneBasedStart - 1, end: sequenceLength, strand },
    { start: 0, end: oneBasedEnd, strand },
  ];
}

function parseSnapGeneFeatureXml(packet, sequenceLength) {
  let xml;
  try { xml = new TextDecoder("utf-8", { fatal: true }).decode(packet).replace(/^\uFEFF/, "").trim(); }
  catch { return null; }
  // Compressed legacy packets are skipped when the file also carries the
  // modern XML packet. Treating compressed bytes as XML would invent data.
  if (!xml.startsWith("<")) return null;
  const document = new DOMParser().parseFromString(xml, "application/xml");
  if (document.querySelector("parsererror")) return null;
  const annotations = [];
  const features = document.getElementsByTagName("Feature");
  for (let index = 0; index < features.length && index < 10_000; index += 1) {
    const feature = features[index];
    const directionality = feature.getAttribute("directionality");
    const strand = directionality === "2" ? -1 : directionality === "1" ? 1 : 0;
    const subRanges = [];
    let color = null;
    for (const segment of feature.getElementsByTagName("Segment")) {
      subRanges.push(...snapGeneSegment(segment.getAttribute("range"), sequenceLength, strand));
      color ||= segment.getAttribute("color")?.trim() || null;
    }
    if (subRanges.length === 0) continue;
    const originalType = feature.getAttribute("type")?.trim() || "misc_feature";
    annotations.push({
      id: `snapgene-feature-${index + 1}`,
      name: (feature.getAttribute("name")?.trim() || `Feature ${index + 1}`).slice(0, 512),
      type: snapGeneFeatureType(originalType),
      start: Math.min(...subRanges.map((segment) => segment.start)),
      end: Math.max(...subRanges.map((segment) => segment.end)),
      strand,
      ...(color ? { color } : {}),
      metadata: snapGeneFeatureMetadata(feature, originalType),
      ...(subRanges.length > 1 ? { subRanges } : {}),
    });
  }
  return annotations;
}

function parseSnapGeneDna(input, filename) {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const decoder = new TextDecoder("ascii", { fatal: true });
  let offset = 0;
  let sawCookie = false;
  let sequence = null;
  let circular = false;
  const modernFeaturePackets = [];
  const legacyFeaturePackets = [];
  while (offset < bytes.length) {
    if (bytes.length - offset < 5) throw new Error("The SnapGene .dna file has a truncated packet header.");
    const type = bytes[offset];
    const length = view.getUint32(offset + 1, false);
    offset += 5;
    if (length > bytes.length - offset) throw new Error("The SnapGene .dna file has a truncated packet.");
    const packet = bytes.subarray(offset, offset + length);
    offset += length;
    if (type === 0x09) {
      if (length < 14 || decoder.decode(packet.subarray(0, 8)) !== "SnapGene") {
        throw new Error("The selected .dna file is not a valid SnapGene file.");
      }
      sawCookie = true;
    } else if (type === 0x00) {
      if (length < 2) throw new Error("The SnapGene .dna file has no sequence data.");
      circular = (packet[0] & 0x01) !== 0;
      const sequenceOffset = packet.length > 1 && packet[1] === 0x6e ? 2 : 1;
      sequence = decoder.decode(packet.subarray(sequenceOffset)).toUpperCase();
    } else if (type === 0x0a) {
      modernFeaturePackets.push(packet);
    } else if (type === 0x07) {
      legacyFeaturePackets.push(packet);
    }
  }
  if (!sawCookie || sequence == null) throw new Error("The selected .dna file has no SnapGene DNA sequence packet.");
  if (!/^[ACGTRYSWKMBDHVNU]+$/.test(sequence)) {
    throw new Error("The SnapGene sequence contains unsupported DNA symbols.");
  }
  if (sequence.length > 2_000_000) throw new Error("Motif DNA imports are limited to 2,000,000 bases.");
  let annotations = null;
  for (const packet of [...modernFeaturePackets, ...legacyFeaturePackets]) {
    const parsed = parseSnapGeneFeatureXml(packet, sequence.length);
    if (parsed !== null) {
      annotations = parsed;
      if (parsed.length > 0) break;
    }
  }
  const name = filename.replace(/\.dna$/i, "").trim() || "SnapGene sequence";
  return {
    records: [{
      name,
      type: "dna",
      topology: circular ? "circular" : "linear",
      sequence,
      annotations: annotations || [],
    }],
  };
}

function motifArgumentsForBytes(bytes, filename) {
  if (/\.dna$/i.test(filename)) return { payload: parseSnapGeneDna(bytes, filename) };
  if (bytes.byteLength > 2_000_000) throw new Error("Motif text imports are limited to 2,000,000 bytes.");
  let content;
  try { content = new TextDecoder("utf-8", { fatal: true }).decode(bytes); }
  catch { throw new Error("This is a binary file. Use SnapGene .dna or a supported text sequence format."); }
  if (!content.trim()) throw new Error("The selected DNA file is empty.");
  if (content.includes("\0")) throw new Error("This is a binary file. Use SnapGene .dna or a supported text sequence format.");
  return { content, filename };
}

async function callMotifOpen(instance, motifArgs) {
  const result = await invoke_strict("call_mcp_app_tool", {
    instanceId: instance.id,
    name: "motif_open_workbench",
    arguments: motifArgs,
  });
  if (result?.isError) {
    const message = result.content?.find((item) => item?.type === "text")?.text;
    throw new Error(message || "Motif rejected the selected DNA file.");
  }
  return result;
}

function motifBridgeRequest(instance, method, params = {}) {
  if (instance.isolated) return isolatedAction(instance, method, params);
  return new Promise((resolve, reject) => {
    const requestId = ++instance.motifRequestId;
    instance.motifRequests.set(requestId, { resolve, reject });
    instance.frame.contentWindow?.postMessage({
      jsonrpc: "2.0",
      method,
      params: { ...params, requestId },
    }, "*");
    window.setTimeout(() => {
      if (instance.motifRequests.delete(requestId)) reject(new Error("Motif did not respond in time."));
    }, 5000);
  });
}

export async function import_motif_dna_file(instanceId) {
  const instance = isolatedApps.get(instanceId) || mcpAppInstances.get(instanceId);
  if (!instance?.initialized || (!instance.isolated && !instance.frame.contentWindow)) {
    throw new Error("The Motif workbench is not ready.");
  }
  if (instance.payload?.tool?.name !== "motif_open_workbench") {
    throw new Error("Local DNA import is only available in the Motif workbench.");
  }

  const file = await new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".dna,.fa,.fasta,.fna,.ffn,.faa,.frn,.gb,.gbk,.genbank,.txt,.seq,.json,text/plain,application/json";
    input.style.display = "none";
    const finish = (selected) => {
      input.remove();
      resolve(selected || null);
    };
    input.addEventListener("change", () => finish(input.files?.[0]), { once: true });
    input.addEventListener("cancel", () => finish(null), { once: true });
    document.body.appendChild(input);
    input.click();
  });
  if (!file) return { imported: false, cancelled: true };
  if (file.size > 32 * 1024 * 1024) throw new Error("The selected DNA file exceeds the 32 MiB import limit.");

  const motifArgs = motifArgumentsForBytes(
    new Uint8Array(await file.arrayBuffer()),
    file.name,
  );

  const result = await callMotifOpen(instance, motifArgs);
  if (instance.isolated) {
    await isolatedAction(instance, "ui/notifications/tool-result", result || { content: [] });
    instance.payload.result = result;
  } else {
    instance.frame.contentWindow.postMessage({jsonrpc:"2.0", method:"ui/notifications/tool-result", params:result || {content:[]}}, "*");
  }
  return {
    imported: true,
    filename: file.name,
    recordCount: result?.structuredContent?.recordCount ?? 0,
  };
}

/** Parse a project file with Motif's MCP tool, then append its records to the
 * already-open workbench without replacing the current inventory. */
export async function add_workspace_file_to_motif(instanceId, path) {
  const instance = isolatedApps.get(instanceId) || mcpAppInstances.get(instanceId);
  if (!instance?.initialized || instance.payload?.tool?.name !== "motif_open_workbench") {
    throw new Error("Open Motif in the current conversation before adding a project file.");
  }
  const filename = String(path).split(/[\\/]/).pop() || "sequence";
  const bytes = await previewBytes({ path, maxBytes: 32 * 1024 * 1024 });
  const result = await callMotifOpen(instance, motifArgumentsForBytes(bytes, filename));
  const records = result?.structuredContent?.payload?.records;
  if (!Array.isArray(records) || records.length === 0) throw new Error("Motif did not return any DNA records.");
  await motifBridgeRequest(instance, "wisp/motif-add-records", { records });
  return { imported: true, filename, recordCount: records.length };
}

export async function request_motif_selection(instanceId) {
  const instance = isolatedApps.get(instanceId) || mcpAppInstances.get(instanceId);
  if (!instance?.initialized || instance.payload?.tool?.name !== "motif_open_workbench") {
    throw new Error("The Motif workbench is not ready.");
  }
  return await motifBridgeRequest(instance, "wisp/motif-get-selection");
}

/** Close a center-tab MCP App and give it a bounded graceful teardown window. */
export function close_mcp_app(instanceId) {
  if (useIsolatedHost()) return closeIsolatedApp(instanceId);
  mcpAppInstances.get(instanceId)?.requestTeardown("user closed the app");
}

const NOTEBOOK_BLOCKED_ELEMENTS = [
  "script", "iframe", "frame", "object", "embed", "foreignObject",
  "animate", "animateMotion", "animateTransform", "set", "mpath",
  "form", "input", "button", "textarea", "select", "option",
  "link", "meta", "base", "audio", "video", "source", "track",
].join(",");

const NOTEBOOK_URL_ATTRIBUTES = new Set([
  "href", "xlink:href", "src", "srcset", "action", "formaction",
  "poster", "ping", "target", "download", "srcdoc",
]);

function notebookSafeResource(value) {
  const normalized = String(value || "").trim();
  return normalized.startsWith("#") ||
    /^data:image\/(?:png|jpeg|gif|webp);base64,/i.test(normalized);
}

function notebookUnsafeCss(value) {
  const withoutLocalFragments = String(value || "")
    .replace(/url\(\s*(['"]?)#[^)]+\)/gi, "");
  return /@import|url\s*\(/i.test(withoutLocalFragments);
}

/**
 * Defense in depth for saved notebook output. The iframe sandbox below is the
 * security boundary; this scrub also removes active elements and references so
 * opening a notebook cannot quietly make network requests.
 */
function scrubNotebookMarkup(doc) {
  doc.querySelectorAll(NOTEBOOK_BLOCKED_ELEMENTS).forEach((node) => node.remove());
  doc.querySelectorAll("*").forEach((node) => {
    for (const attr of [...node.attributes]) {
      const name = attr.name.toLowerCase();
      if (name.startsWith("on") ||
          (NOTEBOOK_URL_ATTRIBUTES.has(name) && !notebookSafeResource(attr.value)) ||
          (name === "style" && notebookUnsafeCss(attr.value))) {
        node.removeAttribute(attr.name);
      }
    }
    if (doc.contentType === "text/html" && node.localName?.toLowerCase() === "img") {
      node.setAttribute("loading", "lazy");
      node.setAttribute("decoding", "async");
      node.setAttribute("referrerpolicy", "no-referrer");
    }
  });
  doc.querySelectorAll("style").forEach((style) => {
    if (notebookUnsafeCss(style.textContent)) style.remove();
  });
  return doc;
}

function staticNotebookHtml(html) {
  const parsed = scrubNotebookMarkup(
    new DOMParser().parseFromString(String(html || ""), "text/html"),
  );
  const styles = [...parsed.head.querySelectorAll("style")]
    .map((style) => style.outerHTML)
    .join("");
  const body = parsed.body?.innerHTML || "";
  const csp = [
    "default-src 'none'", "script-src 'none'", "connect-src 'none'",
    "frame-src 'none'", "object-src 'none'", "base-uri 'none'",
    "form-action 'none'", "img-src data: blob:", "font-src data:",
    "style-src 'unsafe-inline'",
  ].join("; ");
  return `<!doctype html><html><head>` +
    `<meta http-equiv="Content-Security-Policy" content="${csp}">` +
    `<meta name="referrer" content="no-referrer">` +
    `<meta name="viewport" content="width=device-width, initial-scale=1">` +
    `<style>html{color-scheme:light dark}body{margin:12px;overflow-wrap:anywhere}` +
    `img,svg,table{max-width:100%}table{border-collapse:collapse}` +
    `th,td{padding:4px 7px;border:1px solid #8886}</style>${styles}` +
    `</head><body>${body}</body></html>`;
}

function staticNotebookSvg(svg) {
  const parsed = new DOMParser().parseFromString(String(svg || ""), "image/svg+xml");
  if (parsed.querySelector("parsererror") || parsed.documentElement?.localName !== "svg") {
    throw new Error("Invalid SVG notebook output");
  }
  scrubNotebookMarkup(parsed);
  return new XMLSerializer().serializeToString(parsed.documentElement);
}

/** @param {string} kind @param {string} elId @param {string} payloadJson */
export async function mount_preview(kind, elId, payloadJson) {
  const el = document.getElementById(elId);
  if (!el) return;
  cleanupPreview(el);
  const p = JSON.parse(payloadJson);
  el.innerHTML = "";
  el.classList.add("rp-heavy");

  switch (kind) {
    case "latex": {
      const k = await katex();
      el.innerHTML = k.renderToString(p.tex, { displayMode: !!p.display, throwOnError: false });
      break;
    }
    case "pdf": {
      await renderPdf(el, p);
      break;
    }
    case "docx": {
      await renderDocx(el, p);
      break;
    }
    case "xlsx": {
      await renderXlsx(el, p);
      break;
    }
    case "pptx": {
      await renderPptx(el, p);
      break;
    }
    case "image": {
      const src = p.b64 ? `data:${p.mime || "image/png"};base64,${p.b64}` : p.url;
      el.innerHTML = `<img class="rp-img" src="${src}" alt="${p.alt || ""}"/>`;
      break;
    }
    case "html": {
      const frame = document.createElement("iframe");
      frame.className = "rp-html";
      const pluginArtifact = /(^|[\\/])\.wisp[\\/]plugin-artifacts[\\/]/.test(p.path || "");
      frame.setAttribute("sandbox", pluginArtifact ? "allow-scripts" : "allow-same-origin allow-scripts");
      if (pluginArtifact) frame.setAttribute("referrerpolicy", "no-referrer");
      frame.setAttribute("scrolling", "no");
      frame.srcdoc = injectResponsiveHtmlPreview(p.text || "", htmlBaseHref(p.path || ""));
      el.appendChild(frame);
      break;
    }
    case "notebook-html": {
      const frame = document.createElement("iframe");
      frame.className = "rp-notebook-html";
      // No sandbox tokens: scripts, same-origin access, forms, popups, downloads,
      // and navigation out of the frame all stay disabled.
      frame.setAttribute("sandbox", "");
      frame.setAttribute("referrerpolicy", "no-referrer");
      frame.setAttribute("title", p.title || "Notebook HTML output");
      frame.srcdoc = staticNotebookHtml(p.text || "");
      el.appendChild(frame);
      el.__wispPreviewCleanup = () => frame.remove();
      break;
    }
    case "notebook-svg": {
      try {
        const safeSvg = staticNotebookSvg(p.text || "");
        const url = URL.createObjectURL(new Blob([safeSvg], { type: "image/svg+xml" }));
        const img = document.createElement("img");
        img.className = "rp-img rp-notebook-svg";
        img.alt = p.alt || "";
        img.loading = "lazy";
        img.decoding = "async";
        img.referrerPolicy = "no-referrer";
        img.src = url;
        el.appendChild(img);
        el.__wispPreviewCleanup = () => {
          img.remove();
          URL.revokeObjectURL(url);
        };
      } catch (error) {
        console.warn("Failed to render notebook SVG", error);
        el.textContent = p.error || "Unable to preview this SVG output.";
        el.classList.add("rp-error");
      }
      break;
    }
    case "structure":
    case "molecule":
    case "fasta":
    case "msa": {
      linkCss("/vendor-runtime/scientific-preview.css");
      await renderScientific(el, kind, p);
      break;
    }
    default: {
      // textContent on a plain div collapses the file's newlines — a <pre> is
      // what keeps an unrecognised kind readable instead of one long paragraph.
      const pre = document.createElement("pre");
      pre.className = "rp-pre";
      pre.textContent = p.text || "";
      el.appendChild(pre);
    }
  }
}
