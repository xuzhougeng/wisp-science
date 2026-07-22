// Tauri v2 shim + scientific preview mounts (single file so Trunk ships one snippet).

function tauriCore() {
  return window.__TAURI__?.core;
}

function tauriEvent() {
  return window.__TAURI__?.event;
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
    return await core.invoke(cmd, args ?? {});
  } catch (err) {
    console.error(`Tauri command failed: ${cmd}`, err);
    return null;
  }
}

export async function invoke_strict(cmd, args) {
  const core = tauriCore();
  if (!core) {
    throw missingBridgeError(cmd);
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
  const gestureFinished = new Promise((resolve) => {
    window.addEventListener("click", resolve, { capture: true, once: true });
  });
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
  await gestureFinished;
  return String(path);
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

function pastedImageFiles(event) {
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

export function native_drop_in_composer(payload) {
  const el = document.querySelector(".composer-inner");
  if (!el || !payload) return false;
  const rect = el.getBoundingClientRect();
  const scale = window.devicePixelRatio || 1;
  const rawX = Number(payload.x || 0);
  const rawY = Number(payload.y || 0);
  const inside = (x, y) => x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
  return inside(rawX, rawY) || inside(rawX / scale, rawY / scale);
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

export async function listen_native_file_drop(cb) {
  const unlisten = [];
  const push = (fn) => { if (typeof fn === "function") unlisten.push(fn); };
  const handle = (event) => cb(normalizeNativeDropPayload(event));
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

let rdkitInit;
async function rdkit() {
  if (!rdkitInit) {
    const mod = await import("/vendor-runtime/RDKit_minimal-B7RkdM0_.js");
    rdkitInit = mod.R.default();
  }
  return rdkitInit;
}

let mol3dLib;
async function loadMol3d() {
  if (!mol3dLib) {
    const mod = await import("/vendor-runtime/3Dmol-DfD4xImO.js");
    mol3dLib = mod._.default;
  }
  return mol3dLib;
}

let msaLoaded;
async function ensureMsa() {
  if (!msaLoaded) {
    await import("/vendor-runtime/nightingale-msa-5.6.0.js");
    msaLoaded = true;
  }
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

let docxLib;
function docxPreview() {
  // Self-contained ESM bundle (docx-preview + jszip, no bare imports) so .docx
  // renders fully offline in the WebView. See ui/sync-vendor.ps1.
  if (!docxLib) docxLib = import("/vendor-runtime/docx-preview.mjs");
  return docxLib;
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

async function previewBytes(payload) {
  if (payload.bytes) return normalizeRawBytes(payload.bytes);
  if (payload.b64) return base64Bytes(payload.b64);
  const path = String(payload.path || "");
  if (!path) throw new Error("Preview path is empty");
  const maxBytes = Math.min(Number(payload.maxBytes) || 32 * 1024 * 1024, 32 * 1024 * 1024);

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

async function renderDocx(el, payload) {
  cleanupPreview(el);
  const renderToken = Symbol("docx-preview");
  el.__wispPreviewToken = renderToken;
  const loading = document.createElement("div");
  loading.className = "rp-pdf-loading";
  loading.textContent = payload.loading || "Loading…";
  el.replaceChildren(loading);
  try {
    const bytes = await previewBytes(payload);
    const lib = await docxPreview();
    if (!el.isConnected || el.__wispPreviewToken !== renderToken) return;
    const container = document.createElement("div");
    container.className = "rp-docx";
    el.replaceChildren(container);
    // renderAsync takes a Blob/ArrayBuffer; ignoreHeight lets the page reflow to
    // the preview column instead of a fixed A4 height. `experimental` enables
    // docx-preview's fuller feature set (incl. its OMML→MathML math rendering).
    // OMML support covers standard Word math; WPS's OMML dialect is only
    // partially handled upstream, so some WPS formulas can still garble (#274).
    await lib.renderAsync(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), container, null, {
      className: "docx",
      inWrapper: true,
      ignoreWidth: false,
      ignoreHeight: true,
      breakPages: true,
      experimental: true,
    });
    if (el.__wispPreviewToken !== renderToken) return;
    el.__wispPreviewCleanup = () => { container.replaceChildren(); };
  } catch (error) {
    console.error("Failed to render DOCX preview", error);
    if (el.isConnected && el.__wispPreviewToken === renderToken) {
      const message = document.createElement("div");
      message.className = "rp-error rp-pdf-error";
      message.textContent = payload.error || "Unable to preview this document.";
      el.replaceChildren(message);
    }
  }
}

function parseWorkbookInWorker(bytes, signal, timeoutMs = 15_000) {
  return new Promise((resolve, reject) => {
    const worker = new Worker("/vendor-runtime/xlsx-worker.js");
    let settled = false;
    const finish = (callback, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      worker.terminate();
      callback(value);
    };
    const onAbort = () => finish(reject, new DOMException("Aborted", "AbortError"));
    const timer = setTimeout(
      () => finish(reject, new Error("Workbook parsing timed out")),
      timeoutMs,
    );
    worker.onerror = (event) => finish(reject, new Error(event.message || "Workbook worker failed"));
    worker.onmessage = ({ data }) => {
      if (data?.ok) finish(resolve, data.workbook);
      else finish(reject, new Error(data?.error || "Unable to parse workbook"));
    };
    signal?.addEventListener("abort", onAbort, { once: true });
    if (signal?.aborted) {
      onAbort();
      return;
    }
    const copy = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    worker.postMessage(copy, [copy]);
  });
}

function spreadsheetColumnName(index) {
  let value = index + 1;
  let name = "";
  while (value > 0) {
    value -= 1;
    name = String.fromCharCode(65 + (value % 26)) + name;
    value = Math.floor(value / 26);
  }
  return name;
}

function safeSpreadsheetLink(value) {
  try {
    const url = new URL(value);
    return ["http:", "https:", "mailto:"].includes(url.protocol) ? url.href : null;
  } catch (_) {
    return null;
  }
}

function mountWorkbookSheet(root, sheet, payload) {
  const ROW_HEIGHT = 28;
  const COL_WIDTH = 140;
  const ROW_HEADER_WIDTH = 52;
  const COL_HEADER_HEIGHT = 28;
  const formula = root.querySelector(".rp-xlsx-formula-value");
  const viewport = root.querySelector(".rp-xlsx-grid");
  const content = document.createElement("div");
  content.className = "rp-xlsx-content";
  content.style.width = `${ROW_HEADER_WIDTH + sheet.cols * COL_WIDTH}px`;
  content.style.height = `${COL_HEADER_HEIGHT + sheet.rows * ROW_HEIGHT}px`;
  viewport.replaceChildren(content);

  const cellMap = new Map(sheet.cells.map((cell) => [`${cell.row}:${cell.col}`, cell]));
  let frame = 0;
  const render = () => {
    frame = 0;
    const rowStart = Math.max(0, Math.floor((viewport.scrollTop - COL_HEADER_HEIGHT) / ROW_HEIGHT) - 1);
    const rowEnd = Math.min(sheet.rows, Math.ceil((viewport.scrollTop + viewport.clientHeight) / ROW_HEIGHT) + 2);
    const colStart = Math.max(0, Math.floor((viewport.scrollLeft - ROW_HEADER_WIDTH) / COL_WIDTH) - 1);
    const colEnd = Math.min(sheet.cols, Math.ceil((viewport.scrollLeft + viewport.clientWidth) / COL_WIDTH) + 2);
    const visibleMerges = sheet.merges.filter((merge) => (
      merge.endRow >= rowStart && merge.startRow < rowEnd
      && merge.endCol >= colStart && merge.startCol < colEnd
    ));
    const covered = new Set();
    const anchors = new Map();
    for (const merge of visibleMerges) {
      anchors.set(`${merge.startRow}:${merge.startCol}`, merge);
      for (let row = Math.max(rowStart, merge.startRow); row <= Math.min(rowEnd - 1, merge.endRow); row += 1) {
        for (let col = Math.max(colStart, merge.startCol); col <= Math.min(colEnd - 1, merge.endCol); col += 1) {
          if (row !== merge.startRow || col !== merge.startCol) covered.add(`${row}:${col}`);
        }
      }
    }

    const fragment = document.createDocumentFragment();
    for (let row = rowStart; row < rowEnd; row += 1) {
      const header = document.createElement("div");
      header.className = "rp-xlsx-row-head";
      header.textContent = String(row + 1);
      header.style.transform = `translate(${viewport.scrollLeft}px, ${COL_HEADER_HEIGHT + row * ROW_HEIGHT}px)`;
      fragment.appendChild(header);
      for (let col = colStart; col < colEnd; col += 1) {
        const key = `${row}:${col}`;
        if (covered.has(key)) continue;
        const cell = cellMap.get(key);
        const node = document.createElement("div");
        node.className = "rp-xlsx-cell";
        node.style.transform = `translate(${ROW_HEADER_WIDTH + col * COL_WIDTH}px, ${COL_HEADER_HEIGHT + row * ROW_HEIGHT}px)`;
        const merge = anchors.get(key);
        if (merge) {
          node.style.width = `${(merge.endCol - merge.startCol + 1) * COL_WIDTH}px`;
          node.style.height = `${(merge.endRow - merge.startRow + 1) * ROW_HEIGHT}px`;
          node.classList.add("merged");
        }
        const href = cell?.hyperlink && safeSpreadsheetLink(cell.hyperlink);
        if (href) {
          const link = document.createElement("a");
          link.href = href;
          link.target = "_blank";
          link.rel = "noopener noreferrer";
          link.textContent = cell.text;
          node.appendChild(link);
        } else {
          node.textContent = cell?.text || "";
        }
        node.title = cell?.text || "";
        node.addEventListener("click", () => {
          content.querySelector(".rp-xlsx-cell.selected")?.classList.remove("selected");
          node.classList.add("selected");
          formula.textContent = cell?.formula ? `=${cell.formula}` : (cell?.text || "");
        });
        fragment.appendChild(node);
      }
    }
    for (let col = colStart; col < colEnd; col += 1) {
      const header = document.createElement("div");
      header.className = "rp-xlsx-col-head";
      header.textContent = spreadsheetColumnName(col);
      header.style.transform = `translate(${ROW_HEADER_WIDTH + col * COL_WIDTH}px, ${viewport.scrollTop}px)`;
      fragment.appendChild(header);
    }
    const corner = document.createElement("div");
    corner.className = "rp-xlsx-corner";
    corner.style.transform = `translate(${viewport.scrollLeft}px, ${viewport.scrollTop}px)`;
    fragment.appendChild(corner);
    content.replaceChildren(fragment);
  };
  const onScroll = () => {
    if (!frame) frame = requestAnimationFrame(render);
  };
  viewport.addEventListener("scroll", onScroll, { passive: true });
  render();
  return () => {
    viewport.removeEventListener("scroll", onScroll);
    if (frame) cancelAnimationFrame(frame);
  };
}

async function renderXlsx(el, payload) {
  cleanupPreview(el);
  const renderToken = Symbol("xlsx-preview");
  const abortController = new AbortController();
  el.__wispPreviewToken = renderToken;
  el.__wispPreviewCleanup = () => abortController.abort();
  const loading = document.createElement("div");
  loading.className = "rp-pdf-loading";
  loading.textContent = payload.loading || "Loading…";
  el.replaceChildren(loading);
  try {
    const bytes = await previewBytes(payload);
    const workbook = await parseWorkbookInWorker(bytes, abortController.signal);
    if (!el.isConnected || el.__wispPreviewToken !== renderToken) return;
    if (!workbook.sheets.length) throw new Error("Workbook contains no worksheets");

    const root = document.createElement("div");
    root.className = "rp-xlsx";
    const tabs = document.createElement("div");
    tabs.className = "rp-xlsx-tabs";
    const formulaBar = document.createElement("div");
    formulaBar.className = "rp-xlsx-formula";
    const formulaLabel = document.createElement("span");
    formulaLabel.textContent = payload.formulaLabel || "Formula";
    const formulaValue = document.createElement("code");
    formulaValue.className = "rp-xlsx-formula-value";
    formulaBar.append(formulaLabel, formulaValue);
    const grid = document.createElement("div");
    grid.className = "rp-xlsx-grid";
    root.append(tabs, formulaBar, grid);
    if (workbook.truncated) {
      const warning = document.createElement("div");
      warning.className = "rp-xlsx-warning";
      warning.textContent = payload.truncated || "Large workbook: only a bounded preview is shown.";
      root.prepend(warning);
    }
    el.replaceChildren(root);

    let cleanupSheet = () => {};
    const showSheet = (index) => {
      cleanupSheet();
      tabs.querySelector(".active")?.classList.remove("active");
      tabs.children[index]?.classList.add("active");
      formulaBar.querySelector("code").textContent = "";
      cleanupSheet = mountWorkbookSheet(root, workbook.sheets[index], payload);
    };
    workbook.sheets.forEach((sheet, index) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = sheet.name;
      button.title = `${sheet.name} · ${sheet.originalRows.toLocaleString()} × ${sheet.originalCols.toLocaleString()}`;
      button.addEventListener("click", () => showSheet(index));
      tabs.appendChild(button);
    });
    showSheet(0);
    el.__wispPreviewCleanup = () => {
      abortController.abort();
      cleanupSheet();
      root.replaceChildren();
    };
  } catch (error) {
    if (abortController.signal.aborted) return;
    console.error("Failed to render XLSX preview", error);
    if (el.isConnected && el.__wispPreviewToken === renderToken) {
      const message = document.createElement("div");
      message.className = "rp-error rp-pdf-error";
      message.textContent = payload.error || "Unable to preview this workbook.";
      el.replaceChildren(message);
    }
  }
}

let pptxLib;
function pptxPreview() {
  if (!pptxLib) pptxLib = import("/vendor-runtime/pptx-preview.mjs");
  return pptxLib;
}

async function renderPptx(el, payload) {
  cleanupPreview(el);
  const renderToken = Symbol("pptx-preview");
  const abortController = new AbortController();
  el.__wispPreviewToken = renderToken;
  el.__wispPreviewCleanup = () => abortController.abort();
  const loading = document.createElement("div");
  loading.className = "rp-pdf-loading";
  loading.textContent = payload.loading || "Loading…";
  el.replaceChildren(loading);
  let viewer;
  try {
    const [bytes, lib] = await Promise.all([previewBytes(payload), pptxPreview()]);
    if (!el.isConnected || el.__wispPreviewToken !== renderToken) return;
    const container = document.createElement("div");
    container.className = "rp-pptx";
    el.replaceChildren(container);
    viewer = await lib.PptxViewer.open(bytes, container, {
      zipLimits: lib.RECOMMENDED_ZIP_LIMITS,
      lazySlides: true,
      lazyMedia: true,
      scrollContainer: container,
      listOptions: {
        windowed: true,
        initialSlides: 4,
        batchSize: 4,
        overscanViewport: 1.5,
        showSlideLabels: true,
      },
      signal: abortController.signal,
      pdfjs: {
        moduleUrl: "/vendor-runtime/pdf.min.mjs",
        workerUrl: "/vendor-runtime/pdf.worker.min.mjs",
      },
    });
    if (!el.isConnected || el.__wispPreviewToken !== renderToken) {
      viewer.destroy();
      return;
    }
    el.__wispPreviewCleanup = () => {
      abortController.abort();
      viewer?.destroy();
    };
  } catch (error) {
    if (abortController.signal.aborted) return;
    console.error("Failed to render PPTX preview", error);
    viewer?.destroy();
    if (el.isConnected && el.__wispPreviewToken === renderToken) {
      const message = document.createElement("div");
      message.className = "rp-error rp-pdf-error";
      message.textContent = payload.error || "Unable to preview this presentation.";
      el.replaceChildren(message);
    }
  }
}

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
export function preview_selection() {
  const sel = window.getSelection?.();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return "";
  const text = sel.toString().trim();
  if (!text) return "";
  const range = sel.getRangeAt(0);
  let node = range.commonAncestorContainer;
  if (node && node.nodeType === 3) node = node.parentElement;
  const container = node && node.closest ? node.closest("[data-file-path]") : null;
  if (!container) return "";
  const rect = range.getBoundingClientRect();
  return JSON.stringify({
    text,
    path: container.getAttribute("data-file-path") || "",
    x: Math.round(rect.left + rect.width / 2),
    y: Math.round(rect.bottom),
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

let activeMcpAppCleanup = null;

function injectMcpAppCsp(html, resourceMeta) {
  const csp = resourceMeta?.ui?.csp || resourceMeta?.csp || {};
  const safeOrigins = (values, websocket = false) => (Array.isArray(values) ? values : [])
    .filter((value) => typeof value === "string"
      && new RegExp(`^(?:https${websocket ? "|wss" : ""}):\\/\\/(?:\\*\\.)?[a-z0-9.-]+(?::\\d+)?$`, "i").test(value));
  const connect = safeOrigins(csp.connectDomains, true);
  const resources = safeOrigins(csp.resourceDomains);
  const frames = safeOrigins(csp.frameDomains);
  const bases = safeOrigins(csp.baseUriDomains);
  const policy = [
    "default-src 'none'",
    `script-src 'unsafe-inline' 'unsafe-eval' blob: ${resources.join(" ")}`.trim(),
    `style-src 'unsafe-inline' ${resources.join(" ")}`.trim(),
    `img-src data: blob: ${resources.join(" ")}`.trim(),
    `font-src data: ${resources.join(" ")}`.trim(),
    `media-src blob: ${resources.length ? resources.join(" ") : "'none'"}`,
    `connect-src ${connect.length ? connect.join(" ") : "'none'"}`,
    `frame-src ${frames.length ? frames.join(" ") : "'none'"}`,
    `base-uri ${bases.length ? bases.join(" ") : "'self'"}`,
    "object-src 'none'",
    "form-action 'none'",
  ].join("; ");
  const tag = `<meta http-equiv="Content-Security-Policy" content="${escAttr(policy)}">`;
  if (/<head(\s[^>]*)?>/i.test(html)) {
    return html.replace(/<head(\s[^>]*)?>/i, (head) => `${head}${tag}`);
  }
  return `<!doctype html><html><head>${tag}</head><body>${html}</body></html>`;
}

/** Mount one MCP App in a host-owned full-window surface. The app gets an
 * opaque origin and scripts only; filesystem, forms, popups, top navigation,
 * downloads, and same-origin access remain unavailable. */
export function open_mcp_app(payloadJson) {
  const payload = typeof payloadJson === "string" ? JSON.parse(payloadJson) : payloadJson;
  const html = payload?.resource?.text;
  if (typeof html !== "string" || !html) return;
  activeMcpAppCleanup?.();

  const root = document.createElement("section");
  root.id = "wisp-mcp-app-host";
  root.setAttribute("aria-label", "MCP App");
  Object.assign(root.style, {
    position: "fixed", inset: "38px 0 0", zIndex: "2147483000",
    display: "grid", gridTemplateRows: "42px minmax(0, 1fr)",
    background: "var(--bg, #fff)", color: "var(--text, #171717)",
  });
  const head = document.createElement("header");
  Object.assign(head.style, {
    display: "flex", alignItems: "center", gap: "10px", padding: "0 12px",
    borderBottom: "1px solid var(--border, #ddd)", background: "var(--bg-elev, #fff)",
    font: "600 13px system-ui, sans-serif",
  });
  const title = document.createElement("span");
  title.textContent = payload?.tool?.title || payload?.tool?.name || "MCP App";
  title.style.flex = "1";
  const origin = document.createElement("span");
  origin.textContent = payload?.resource?.uri || "";
  Object.assign(origin.style, { color: "var(--text-muted, #666)", fontWeight: "400", fontSize: "11px" });
  const close = document.createElement("button");
  close.type = "button";
  close.textContent = "Close";
  Object.assign(close.style, {
    border: "1px solid var(--border, #ccc)", borderRadius: "7px", padding: "5px 10px",
    background: "var(--bg, #fff)", color: "inherit", cursor: "pointer",
  });
  head.append(title, origin, close);

  const frame = document.createElement("iframe");
  frame.title = title.textContent;
  frame.setAttribute("sandbox", "allow-scripts");
  frame.setAttribute("referrerpolicy", "no-referrer");
  Object.assign(frame.style, { width: "100%", height: "100%", border: "0", background: "#fff" });
  root.append(head, frame);
  document.body.appendChild(root);

  let initialized = false;
  let teardownId = 1000000;
  let teardownRequestId = null;
  let teardownTimer = null;
  const post = (message) => frame.contentWindow?.postMessage(message, "*");
  const sendData = () => {
    if (!initialized) return;
    post({ jsonrpc: "2.0", method: "ui/notifications/tool-input", params: { arguments: payload.arguments || {} } });
    post({ jsonrpc: "2.0", method: "ui/notifications/tool-result", params: payload.result || { content: [] } });
  };
  const onMessage = (event) => {
    if (event.source !== frame.contentWindow || !event.data || event.data.jsonrpc !== "2.0") return;
    const message = event.data;
    if (message.method === "ui/initialize" && message.id != null) {
      post({
        jsonrpc: "2.0", id: message.id, result: {
          protocolVersion: message.params?.protocolVersion || "2026-01-26",
          hostCapabilities: { sandbox: { csp: payload?.resource?._meta?.ui?.csp || payload?.resource?._meta?.csp || {} } },
          hostInfo: { name: "wisp-science", version: "0.19.0" },
          hostContext: {
            theme: document.documentElement.dataset.theme === "dark" ? "dark" : "light",
            displayMode: "fullscreen", availableDisplayModes: ["fullscreen"],
            containerDimensions: { width: Math.max(frame.clientWidth, 320), height: Math.max(frame.clientHeight, 320) },
            locale: document.documentElement.lang || navigator.language || "en",
            timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
            platform: "desktop", userAgent: "wisp-science/0.19.0",
            toolInfo: { tool: payload.tool || {} },
          },
        },
      });
      return;
    }
    if (message.method === "ui/notifications/initialized") {
      initialized = true;
      sendData();
      return;
    }
    if (message.method === "ping" && message.id != null) {
      post({ jsonrpc: "2.0", id: message.id, result: {} });
      return;
    }
    if (teardownRequestId != null && message.id === teardownRequestId) {
      cleanup();
      return;
    }
    if (message.id != null) {
      post({ jsonrpc: "2.0", id: message.id, error: { code: -32601, message: "Capability is not granted by Wisp" } });
    }
  };
  window.addEventListener("message", onMessage);
  const cleanup = () => {
    if (teardownTimer != null) window.clearTimeout(teardownTimer);
    window.removeEventListener("message", onMessage);
    root.remove();
    if (activeMcpAppCleanup === replaceCleanup) activeMcpAppCleanup = null;
  };
  const requestTeardown = (reason) => {
    if (initialized && teardownRequestId == null) {
      teardownRequestId = ++teardownId;
      post({ jsonrpc: "2.0", id: teardownRequestId, method: "ui/resource-teardown", params: { reason } });
      teardownTimer = window.setTimeout(cleanup, 500);
    } else {
      cleanup();
    }
  };
  const replaceCleanup = () => requestTeardown("replaced by another MCP App");
  activeMcpAppCleanup = replaceCleanup;
  close.addEventListener("click", () => requestTeardown("user closed the app"));
  frame.srcdoc = injectMcpAppCsp(html, payload?.resource?._meta);
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

function fastaStats(text) {
  const lines = (text || "").split("\n");
  let seqs = 0;
  let maxLen = 0;
  let cur = 0;
  for (const raw of lines) {
    const line = raw.trim();
    if (!line || line.startsWith(";")) continue;
    if (line.startsWith(">")) {
      seqs += 1;
      cur = 0;
      continue;
    }
    cur += line.length;
    if (cur > maxLen) maxLen = cur;
  }
  return { seqs, maxLen };
}

function renderFasta(el, text) {
  const lines = (text || "").split("\n");
  let rows = "";
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const cls = line.startsWith(">") ? "rp-fasta-hdr" : "rp-fasta-seq";
    rows += `<tr><td class="rp-fasta-ln">${i + 1}</td><td class="${cls}">${escHtml(line) || "&nbsp;"}</td></tr>`;
  }
  const stats = fastaStats(text);
  const note = stats.seqs
    ? `<div class="rp-fasta-bar">${stats.seqs} sequences · ${stats.maxLen.toLocaleString()} positions</div>`
    : "";
  el.innerHTML = `${note}<div class="rp-fasta-wrap"><table class="rp-fasta-table"><tbody>${rows}</tbody></table></div>`;
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
    case "structure": {
      const box = document.createElement("div");
      box.className = "rp-3dmol";
      el.appendChild(box);
      const $3Dmol = await loadMol3d();
      const v = $3Dmol.createViewer(box, { backgroundColor: "0x1e2024" });
      v.addModel(p.text, p.format || "pdb");
      v.setStyle({}, { cartoon: { color: "spectrum" } });
      v.zoomTo();
      v.render();
      break;
    }
    case "molecule": {
      const RDKit = await rdkit();
      const mol = RDKit.get_mol(p.smiles || p.text);
      if (!mol) {
        el.textContent = "Invalid molecule";
        break;
      }
      el.innerHTML = mol.get_svg(400, 300);
      mol.delete();
      break;
    }
    case "fasta": {
      renderFasta(el, p.text || "");
      break;
    }
    case "msa": {
      await ensureMsa();
      const text = p.text || p.fasta || "";
      const stats = fastaStats(text);
      const wrap = document.createElement("div");
      wrap.className = "rp-msa-wrap";
      const bar = document.createElement("div");
      bar.className = "rp-msa-bar";
      bar.textContent = `${stats.seqs} sequences · ${stats.maxLen.toLocaleString()} positions`;
      wrap.appendChild(bar);
      const tag = document.createElement("nightingale-msa");
      tag.setAttribute("width", "100%");
      tag.setAttribute("height", "420");
      tag.setAttribute("color-scheme", "clustal2");
      tag.setAttribute("label-width", "150");
      tag.setAttribute("tile-height", "20");
      tag.setAttribute("display-start", "1");
      tag.setAttribute("display-end", String(Math.max(stats.maxLen, 50)));
      wrap.appendChild(tag);
      el.appendChild(wrap);
      await customElements.whenDefined("nightingale-msa");
      tag.data = text;
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
