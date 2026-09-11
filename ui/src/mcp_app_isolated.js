// Windows native MCP App controller. Never create an iframe in this document.
// The explicit mock override is used by browser tests to separate the two
// backends; native creation errors deliberately do NOT fall back to an iframe.
export const isolatedApps = new Map();
export const useIsolatedHost = () => window.__WISP_MCP_APP_BACKEND__
  ? window.__WISP_MCP_APP_BACKEND__ === "native-child"
  : navigator.userAgent.includes("Windows");
const invoke = (command, args = {}) => {
  const core = window.__TAURI__?.core || window.__TAURI_INTERNALS__;
  if (!core) return Promise.reject(new Error("The native MCP App host is unavailable."));
  return core.invoke(command, args);
};
let hostPromise, serial = 0, revision = 0, active = null;
let frame = 0, observer, inputOccluded = false;
const host = () => hostPromise ||= invoke("mcp_app_host_info").then((info) => {
  if (info?.backend !== "native-child" || !info.ownerEpoch) throw new Error("Native MCP App isolation is unavailable; refusing unsafe iframe fallback.");
  return info;
});
const current = (instance) => active === instance && !instance.closed && instance.target?.isConnected;
const text = () => document.documentElement.lang.startsWith("zh") ? {
  hint:"此 App 在独立视图中运行；切走将销毁，切回重建。未提交的勾选、分页和编辑可能丢失。",
  retry:"重新加载 App", loading:"正在创建隔离视图…", error:"App 隔离视图不可用：",
} : {
  hint:"Isolated App: leaving destroys this view. Reopening restores saved results, not unsubmitted selections, pages or edits.",
  retry:"Reload App", loading:"Creating isolated view…", error:"Isolated App unavailable: ",
};

const visibleElement = (el) => {
  const style = getComputedStyle(el);
  return style.display !== "none" && style.visibility !== "hidden" && el.getClientRects().length > 0;
};
const intersects = (a, b) => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
function occluded(target, rect) {
  if (inputOccluded || document.visibilityState !== "visible") return true;
  // Native children do not participate in DOM stacking. Covers root dialogs,
  // component-local menus/popovers, and explicit future occlusion surfaces.
  for (const el of document.querySelectorAll('[aria-modal="true"], [role="dialog"], [role="menu"], .overlay, .project-search-overlay, .compose-menu, .mention-menu, .context-menu, .drag-overlay, [data-mcp-app-occludes]')) {
    if (!target.contains(el) && visibleElement(el) && intersects(rect, el.getBoundingClientRect())) return true;
  }
  // Also catch positioned host UI that does not yet use the semantic markers.
  for (const fx of [0.02, 0.5, 0.98]) for (const fy of [0.02, 0.5, 0.98]) {
    const el = document.elementFromPoint(rect.left + rect.width * fx, rect.top + rect.height * fy);
    if (el && el !== target && !target.contains(el)) return true;
  }
  return false;
}
function bounds(instance, forceHidden = false) {
  const rect = instance.target?.getBoundingClientRect();
  return {x:rect?.x || 0, y:rect?.y || 0, width:rect?.width || 0, height:rect?.height || 0,
    viewportWidth:window.innerWidth, viewportHeight:window.innerHeight,
    visible:!!rect && current(instance) && visibleElement(instance.target) && !forceHidden && !occluded(instance.target, rect),
    revision:++revision};
}
function context(instance, b) {
  return {theme:document.documentElement.dataset.theme === "dark" ? "dark" : "light", displayMode:"inline",
    availableDisplayModes:["inline"], containerDimensions:{width:b.width, height:b.height},
    locale:document.documentElement.lang || navigator.language, timeZone:Intl.DateTimeFormat().resolvedOptions().timeZone,
    platform:"desktop", toolInfo:{tool:instance.payload.tool || {}}};
}
function schedule() {
  if (frame || !active) return;
  frame = requestAnimationFrame(() => {
    frame = 0; sync();
    // ResizeObserver does not fire for an animated translate/position. Follow
    // only the content's ancestors, not arbitrary spinning guest/host icons.
    for (let el = active?.target; el; el = el.parentElement) {
      if (el.getAnimations().some((animation) => animation.playState === "running")) { schedule(); break; }
    }
  });
}
function sync(forceHidden = false) {
  const instance = active;
  if (!instance?.handle || !current(instance)) return;
  const b = bounds(instance, forceHidden), hostContext = context(instance, b);
  const key = JSON.stringify({...b, revision:0, hostContext});
  if (key === instance.boundsKey) return;
  instance.boundsKey = key;
  void invoke("update_mcp_app_child_bounds", {handle:instance.handle, bounds:b, hostContext})
    .catch((error) => { if (current(instance) && !String(error).includes("stale-instance")) showError(instance, error); });
}
function installObservers() {
  if (observer) return;
  observer = new MutationObserver(schedule);
  observer.observe(document.documentElement, {subtree:true, childList:true, attributes:true,
    attributeFilter:["class", "style", "hidden", "open", "data-theme"]});
  window.addEventListener("resize", schedule);
  window.addEventListener("scroll", schedule, true);
  window.addEventListener("transitionrun", schedule, true);
  window.addEventListener("animationstart", schedule, true);
  document.addEventListener("visibilitychange", schedule);
  window.addEventListener("focus", schedule);
  window.addEventListener("wisp-mcp-native-resize", () => { if (active) active.boundsKey = null; schedule(); });
  // Hide on the initiating event, before a host click opens a modal/menu.
  // Mutation/resize observation later restores only the still-current view.
  const beforeHostInteraction = (event) => {
    // A native select opens its picker during the pointer's default action.
    // Hiding/showing a sibling WebView2 here can dismiss that picker. Labels
    // can initiate the same action; DOM menus still use normal occlusion below.
    const target = event.target instanceof Element ? event.target : null;
    const control = target?.closest("select") || target?.closest("label")?.control;
    if (control instanceof HTMLSelectElement) {
      schedule();
      return;
    }
    inputOccluded = true;
    sync(true);
    requestAnimationFrame(() => { inputOccluded = false; schedule(); });
  };
  window.addEventListener("pointerdown", beforeHostInteraction, true);
  // Do not force-hide on every keydown: that would flash the child while
  // typing in the composer. Dialogs/menus are still caught by occlusion
  // observers after they appear.
  window.addEventListener("keydown", schedule, true);
}
function disposeObservers() {
  // The one window-level observer is bounded and shared. Per-view observers
  // and animation state are disconnected on every retirement below.
  if (!active && frame) { cancelAnimationFrame(frame); frame = 0; }
}
function showError(instance, error) {
  if (!current(instance)) return;
  instance.initialized = false;
  instance.status.textContent = text().error + String(error?.message || error).slice(0, 512);
  instance.status.setAttribute("role", "alert");
  instance.status.hidden = false;
}
function retire(instance, reason) {
  instance.closed = true;
  instance.initialized = false;
  instance.resizeObserver?.disconnect();
  if (active === instance) active = null;
  disposeObservers();
  // Use the mount serial, not a returned child label: a close may beat a slow
  // creation reply. Rust records the tombstone before it starts native work.
  return host().then((info) => invoke("close_mcp_app_child", {instanceId:instance.id,
    ownerEpoch:info.ownerEpoch, mountSerial:instance.serial, reason})).catch(()=>{});
}

export function mountIsolatedApp(id, elId, payloadJson) {
  const root = document.getElementById(elId);
  if (!root) return false;
  const old = isolatedApps.get(id);
  // A remount retains the latest imported result, but never automatically
  // replays a tools/call. A new presentation always replaces this snapshot.
  const source = typeof payloadJson === "string" ? payloadJson : JSON.stringify(payloadJson);
  let payload;
  try { payload = old?.source === source ? old.payload : JSON.parse(source); }
  catch { return false; }
  if (typeof payload?.resource?.text !== "string") return false;
  if (active) void retire(active, "suspend");
  const instance = {id, payload, source, serial:++serial, isolated:true, closed:false, initialized:false,
    target:null, handle:null, boundsKey:null, status:null, resizeObserver:null};
  isolatedApps.set(id, instance);
  active = instance;
  root.dataset.mcpBackend = "native-child";
  // Reload and the warning live OUTSIDE the native content rectangle.
  const controls = document.createElement("div");
  controls.className = "mcp-app-isolation-controls";
  const hint = document.createElement("span"); hint.textContent = text().hint;
  const reload = document.createElement("button"); reload.type = "button"; reload.textContent = text().retry;
  reload.addEventListener("click", () => mountIsolatedApp(id, elId, instance.source));
  controls.append(hint, reload);
  const target = document.createElement("div"); target.className = "mcp-app-native-content";
  const status = document.createElement("div"); status.className = "mcp-app-native-status"; status.textContent = text().loading;
  target.append(status);
  root.replaceChildren(controls, target);
  instance.target = target;
  instance.status = status;
  installObservers();
  instance.resizeObserver = new ResizeObserver(schedule);
  instance.resizeObserver.observe(target);
  void (async () => {
    try {
      const info = await host();
      if (!current(instance)) return;
      const b = bounds(instance);
      const handle = await invoke("open_mcp_app_child", {instanceId:id, payload:instance.payload,
        ownerEpoch:info.ownerEpoch, mountSerial:instance.serial, bounds:b, hostContext:context(instance, b)});
      if (!current(instance)) { await retire(instance, "suspend"); return; }
      instance.handle = handle;
      instance.initialized = true;
      status.hidden = true;
      schedule();
    } catch (error) { showError(instance, error); }
  })();
  return true;
}

export function suspendIsolatedApp(id) {
  const instance = isolatedApps.get(id);
  if (instance && !instance.closed) void retire(instance, "suspend");
}
export function closeIsolatedApp(id) {
  const instance = isolatedApps.get(id);
  if (!instance) return;
  isolatedApps.delete(id);
  void retire(instance, "user_close");
}
export async function isolatedAction(instance, method, params = {}) {
  if (!current(instance) || !instance.initialized) throw new Error("The App view is not ready or was closed.");
  const result = await invoke("request_mcp_app_child_action", {instanceId:instance.id, handle:instance.handle, method, params});
  if (!current(instance)) throw new Error("The App view was replaced while the request was running.");
  if (method === "wisp/motif-add-records" && Array.isArray(params.records)) {
    const previous = instance.payload.result?.structuredContent?.payload?.records || [];
    instance.payload.result = {...instance.payload.result, structuredContent:{...instance.payload.result?.structuredContent,
      payload:{...instance.payload.result?.structuredContent?.payload, records:[...previous, ...params.records]}}};
  }
  return result;
}
