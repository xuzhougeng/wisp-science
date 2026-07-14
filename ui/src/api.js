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
    katexMod = (await import("/vendor/katex-Dn761jRB.js")).k;
    linkCss("/vendor/katex-DwwF5kvc.css");
  }
  return katexMod;
}

let rdkitInit;
async function rdkit() {
  if (!rdkitInit) {
    const mod = await import("/vendor/RDKit_minimal-B7RkdM0_.js");
    rdkitInit = mod.R.default();
  }
  return rdkitInit;
}

let mol3dLib;
async function loadMol3d() {
  if (!mol3dLib) {
    const mod = await import("/vendor/3Dmol-DfD4xImO.js");
    mol3dLib = mod._.default;
  }
  return mol3dLib;
}

let msaLoaded;
async function ensureMsa() {
  if (!msaLoaded) {
    await import("/vendor/nightingale-msa-5.6.0.js");
    msaLoaded = true;
  }
}

let pdfjsLib;
async function pdfjs() {
  if (!pdfjsLib) {
    pdfjsLib = import("/vendor/pdf.min.mjs").then((mod) => {
      // WebView2 does not ship a browser PDF plugin, so PDFs are rendered to
      // canvas with the worker bundled alongside the application.
      mod.GlobalWorkerOptions.workerSrc = "/vendor/pdf.worker.min.mjs";
      return mod;
    });
  }
  return pdfjsLib;
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
  let currentPage = 1;
  let rendering = false;
  let disposed = false;
  try {
    const lib = await pdfjs();
    const source = payload.b64 ? { data: base64Bytes(payload.b64) } : payload.url;
    if (!source) throw new Error("PDF data is empty");

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

    nav.append(prevButton, pageIndicator, nextButton);
    toolbar.appendChild(nav);

    const viewer = document.createElement("div");
    viewer.className = "rp-pdf-viewer";

    root.append(toolbar, viewer);
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

    const renderPage = async (pageNumber) => {
      rendering = true;
      syncControls();

      const page = await pdf.getPage(pageNumber);
      try {
        // Render at up to 2x the displayed width so text remains crisp on HiDPI
        // screens without making the single-page preview consume unbounded canvas memory.
        const availableWidth = Math.max(
          240,
          Math.min(viewer.clientWidth || el.clientWidth || 800, 1000),
        );
        const pixelRatio = Math.min(window.devicePixelRatio || 1, 2);
        const natural = page.getViewport({ scale: 1 });
        const cssScale = availableWidth / natural.width;
        const viewport = page.getViewport({ scale: cssScale * pixelRatio });
        const wrapper = document.createElement("div");
        wrapper.className = "rp-pdf-page";
        wrapper.dataset.page = String(pageNumber);
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
      if (event.key === "PageUp") {
        event.preventDefault();
        stepPage(-1);
      } else if (event.key === "PageDown") {
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
      document.removeEventListener("keydown", onKeyDown);
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
    case "image": {
      const src = p.b64 ? `data:${p.mime || "image/png"};base64,${p.b64}` : p.url;
      el.innerHTML = `<img class="rp-img" src="${src}" alt="${p.alt || ""}"/>`;
      break;
    }
    case "html": {
      const frame = document.createElement("iframe");
      frame.className = "rp-html";
      frame.setAttribute("sandbox", "allow-same-origin allow-scripts");
      frame.setAttribute("scrolling", "no");
      frame.srcdoc = injectResponsiveHtmlPreview(p.text || "", htmlBaseHref(p.path || ""));
      el.appendChild(frame);
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
    default:
      el.textContent = p.text || "";
  }
}
