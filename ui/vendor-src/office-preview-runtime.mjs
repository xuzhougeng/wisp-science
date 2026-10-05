// Shared offline Office rendering for the WebView and native WebView2 sandbox.
// Hosts own scoped byte reads; this module owns DOM, workers and disposal.
let docxLib;
function docxPreview() {
  // Self-contained ESM bundle (docx-preview + jszip, no bare imports) so .docx
  // renders fully offline in the WebView. See ui/sync-vendor.ps1.
  if (!docxLib) docxLib = import("/vendor-runtime/docx-preview.mjs");
  return docxLib;
}

export function cleanupPreview(el) {
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

export async function renderDocx(el, payload, previewBytes) {
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
    el.__wispPreviewCleanup = () => container.replaceChildren();
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
    if (el.__wispPreviewToken !== renderToken) container.replaceChildren();
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
        node.dataset.sheet = sheet.name;
        node.dataset.cells = `${spreadsheetColumnName(col)}${row + 1}`
          + (merge ? `:${spreadsheetColumnName(merge.endCol)}${merge.endRow + 1}` : '');
        if (cell?.formula) node.dataset.formula = cell.formula;
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

export async function renderXlsx(el, payload, previewBytes) {
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
      window.getSelection()?.removeAllRanges();
      cleanupSheet();
      tabs.querySelector(".active")?.classList.remove("active");
      tabs.children[index]?.classList.add("active");
      formulaBar.querySelector("code").textContent = "";
      grid.scrollTop = grid.scrollLeft = 0;
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

export async function renderPptx(el, payload, previewBytes) {
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
