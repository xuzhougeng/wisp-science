import { renderDocx, renderXlsx, renderPptx, cleanupPreview } from '/vendor-runtime/office-preview-runtime.mjs';
import { renderScientific } from '/vendor-runtime/scientific-preview.mjs';
import { documentSelection, clearDocumentSelection, documentSelectionAnchor } from './selection.mjs';
const officeRenderers = { docx: renderDocx, xlsx: renderXlsx, pptx: renderPptx };
const scientificKinds = new Set(['structure', 'molecule', 'msa', 'fasta']);
const content = document.getElementById('content'), status = document.getElementById('status');
const nav = document.querySelector('nav');
let task, documentPdf, pdfLib, page = 1, scale = 1, renderTask, textTask, generation = 0, rendering = false;
let canQuote = false, quotePending = false, selectedText = '', activeKind = '', selectedQuote = null, sentActive = null;
let selectionPopupEnabled = true, manualQuoteKey = '';
let locale = 'zh', renderedFitWidth = 0, refitTimer, dragStart = null;
const localized = (zh, en) => locale === 'en' ? en : zh;
const officeLabels = data => ({
    loading: localized('加载中…', 'Loading…'),
    error: data.kind === 'xlsx' ? localized('无法预览此工作簿。', 'Unable to preview this workbook.')
        : data.kind === 'pptx' ? localized('无法预览此演示文稿。', 'Unable to preview this presentation.')
        : localized('无法预览此文档。', 'Unable to preview this document.'),
    formulaLabel: localized('公式', 'Formula'),
    truncated: localized('工作簿较大，仅显示受限范围内的预览。', 'Large workbook: only a bounded preview is shown.'),
    ...data.officeLabels,
});
const fitWidth = () => Math.max(240, Math.min(content.clientWidth || 800, 1000));
const pageLabel = () => localized(`第 ${page} 页，共 ${documentPdf?.numPages ?? 0} 页`, `Page ${page} of ${documentPdf?.numPages ?? 0}`);
const send = data => window.chrome.webview.postMessage(data);
const buttons = [...document.querySelectorAll('#pdf-actions button')];
const quoteButtons = [...document.querySelectorAll('[data-quote]')];
const quoteActions = document.getElementById('quote-actions');
function positionQuoteActions() {
    const anchor = canQuote && (selectionPopupEnabled || manualQuoteKey) && selectedText && selectedText.length <= 32768
        ? documentSelectionAnchor(content, activeKind) : null;
    quoteActions.hidden = !anchor;
    if (!anchor) return;
    const margin = 12, minimumTop = Math.max(margin, content.getBoundingClientRect().top);
    quoteActions.style.maxHeight = `${Math.max(1, innerHeight - minimumTop - margin)}px`;
    const { width, height } = quoteActions.getBoundingClientRect();
    const left = Math.max(margin, Math.min((anchor.left + anchor.right - width) / 2, innerWidth - width - margin));
    // Embedded previews cannot draw over the native title bar. Flip below a
    // selection near the top, then clamp to keep every action reachable.
    const above = anchor.top - height - 10;
    const top = Math.max(minimumTop, Math.min(above >= minimumTop ? above : anchor.bottom + 10, innerHeight - height - margin));
    quoteActions.style.left = `${left}px`; quoteActions.style.top = `${top}px`;
}
function selectionChanged() {
    selectedQuote = !rendering && canQuote ? documentSelection(content, activeKind, page) : null;
    if (manualQuoteKey && manualQuoteKey !== JSON.stringify(selectedQuote)) manualQuoteKey = '';
    selectedText = selectedQuote?.text ?? '';
    quoteButtons.forEach(button => { button.disabled = !canQuote || quotePending || !selectedText || selectedText.length > 32768; });
    positionQuoteActions();
    // DOM mutations and selectionchange fire constantly while pages render and
    // sheets scroll; the host only mirrors the flag.
    const active = !!selectedText && (selectionPopupEnabled || !!manualQuoteKey);
    if (sentActive !== active) { sentActive = active; send({ type: 'selection', active }); }
}
function showManualQuoteActions() {
    selectionChanged();
    if (!selectedQuote || quotePending) return false;
    manualQuoteKey = JSON.stringify(selectedQuote); selectionChanged(); return true;
}
// Always report after an explicit clear: the host resets its own flag then.
function clearSelection() { clearDocumentSelection(content); sentActive = null; selectionChanged(); }
document.addEventListener('selectionchange', selectionChanged);
content.addEventListener('click', () => queueMicrotask(selectionChanged));
content.addEventListener('contextmenu', event => { if (showManualQuoteActions()) event.preventDefault(); });
document.addEventListener('pointerdown', event => {
    if (event.button === 0 && manualQuoteKey && !quoteActions.contains(event.target)) {
        manualQuoteKey = ''; selectionChanged();
    }
});
content.addEventListener('scroll', positionQuoteActions, true);
window.addEventListener('resize', positionQuoteActions);
// Sheet changes and virtualized cell disposal retire the previous cell selection.
new MutationObserver(selectionChanged).observe(content, { childList: true, subtree: true });
// Keep the browser selection when pressing the toolbar. Keyboard activation
// also uses the current range, without inventing text from the page canvas.
quoteButtons.forEach(button => {
    button.addEventListener('pointerdown', event => event.preventDefault());
    button.onclick = () => {
        selectionChanged();
        if (button.disabled) return;
        quotePending = true; controls(); selectionChanged();
        send({ type: 'quote', ...selectedQuote, jump: button.dataset.quote === 'jump' });
    };
});
window.addEventListener('keydown', event => {
    if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey) {
        if (showManualQuoteActions()) { event.preventDefault(); event.stopPropagation(); }
        return;
    }
    if (event.key === 'Escape') {
        event.preventDefault(); event.stopPropagation();
        if (selectedText && (selectionPopupEnabled || manualQuoteKey)) clearSelection(); else send({ type: 'escape' });
    }
}, true);
function controls() {
    buttons.forEach(button => button.disabled = rendering || quotePending || !documentPdf);
    document.getElementById('previous').disabled ||= page <= 1;
    document.getElementById('next').disabled ||= page >= (documentPdf?.numPages ?? 0);
    document.getElementById('smaller').disabled ||= scale <= .25;
    document.getElementById('larger').disabled ||= scale >= 4;
    document.getElementById('page').textContent = pageLabel();
    document.getElementById('zoom').textContent = `${Math.round(scale * 100)}%`;
    selectionChanged();
}
async function renderPage() {
    if (!documentPdf || rendering) return;
    const current = generation;
    rendering = true; clearSelection(); controls(); status.textContent = localized('正在渲染…', 'Rendering…');
    try {
        const pdfPage = await documentPdf.getPage(page);
        if (current !== generation) return;
        // Rasterize a fit-to-width page once. Zoom is a CSS multiple shared by
        // the canvas and selectable glyph layer, as in the canonical viewer.
        const width = fitWidth(), natural = pdfPage.getViewport({ scale: 1 });
        const cssViewport = pdfPage.getViewport({ scale: width / natural.width });
        const ratio = Math.min(window.devicePixelRatio || 1, 2, Math.sqrt(16000000 / (cssViewport.width * cssViewport.height)));
        const viewport = pdfPage.getViewport({ scale: cssViewport.scale * ratio });
        const canvas = document.createElement('canvas');
        canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
        canvas.setAttribute('aria-label', pageLabel());
        renderTask = pdfPage.render({ canvasContext: canvas.getContext('2d'), viewport });
        await renderTask.promise;
        if (current !== generation) return;
        const wrapper = document.createElement('div'); wrapper.className = 'pdf-page';
        wrapper.style.setProperty('--preview-zoom', String(scale));
        wrapper.style.width = `calc(${width}px * var(--preview-zoom))`;
        wrapper.style.height = `calc(${cssViewport.height}px * var(--preview-zoom))`;
        wrapper.dataset.page = String(page); renderedFitWidth = width;
        wrapper.append(canvas); content.replaceChildren(wrapper); status.textContent = '';
        // A failed text layer must leave the readable page canvas intact.
        try {
            const layer = document.createElement('div'); layer.className = 'textLayer';
            layer.style.setProperty('--scale-factor', String(cssViewport.scale));
            layer.style.setProperty('--total-scale-factor', `calc(${cssViewport.scale} * var(--preview-zoom))`);
            wrapper.append(layer);
            textTask = new pdfLib.TextLayer({ textContentSource: pdfPage.streamTextContent(), container: layer, viewport: cssViewport });
            await textTask.render();
            if (current !== generation) return;
            if (!layer.textContent.trim()) status.textContent = localized('此页没有可选择的文字。', 'This page has no selectable text.');
        } catch (error) {
            if (current === generation && error?.name !== 'AbortException') status.textContent = localized('此页文字层不可用，仍可查看页面。', 'The text layer is unavailable; the page remains viewable.');
        }
    } catch (error) { if (current === generation) status.textContent = localized(`PDF 渲染失败：${error.message}`, `PDF rendering failed: ${error.message}`); }
    finally { if (current === generation) { rendering = false; controls(); } }
}
function setPage(value) {
    if (!documentPdf || rendering || quotePending || !Number.isInteger(value) || value < 1 || value > documentPdf.numPages || value === page) return;
    page = value; void renderPage();
}
function setZoom(value) {
    if (!documentPdf || rendering || quotePending) return;
    scale = Math.max(.25, Math.min(4, value));
    content.querySelector('.pdf-page')?.style.setProperty('--preview-zoom', String(scale)); controls();
}
document.getElementById('previous').onclick = () => setPage(page - 1);
document.getElementById('next').onclick = () => setPage(page + 1);
document.getElementById('smaller').onclick = () => setZoom(scale - .25);
document.getElementById('larger').onclick = () => setZoom(scale + .25);
document.getElementById('zoom').onclick = () => setZoom(1);
window.addEventListener('keydown', event => {
    if (activeKind !== 'pdf' || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey
        || event.target.closest?.('input,textarea,[contenteditable="true"]')) return;
    const step = ['PageUp', 'ArrowUp', 'ArrowLeft'].includes(event.key) ? -1 : ['PageDown', 'ArrowDown', 'ArrowRight'].includes(event.key) ? 1 : 0;
    if (step) { event.preventDefault(); event.stopPropagation(); setPage(page + step); }
});
content.addEventListener('wheel', event => {
    if (activeKind !== 'pdf' || !event.deltaY) return;
    event.preventDefault(); setZoom(scale + (event.deltaY < 0 ? .25 : -.25));
}, { passive: false });
content.addEventListener('pointerdown', event => {
    if (activeKind !== 'pdf' || event.button !== 0 || event.target.closest?.('.textLayer span')
        || (content.scrollWidth <= content.clientWidth && content.scrollHeight <= content.clientHeight)) return;
    event.preventDefault(); content.setPointerCapture(event.pointerId);
    dragStart = { x: event.clientX, y: event.clientY, left: content.scrollLeft, top: content.scrollTop };
    content.classList.add('is-dragging');
});
content.addEventListener('pointermove', event => {
    if (!dragStart) return;
    event.preventDefault(); content.scrollLeft = dragStart.left - (event.clientX - dragStart.x);
    content.scrollTop = dragStart.top - (event.clientY - dragStart.y);
});
function endDrag(event) {
    dragStart = null; content.classList.remove('is-dragging');
    if (event && content.hasPointerCapture(event.pointerId)) content.releasePointerCapture(event.pointerId);
}
for (const name of ['pointerup', 'pointercancel', 'lostpointercapture']) content.addEventListener(name, endDrag);
function scheduleRefit() {
    clearTimeout(refitTimer);
    if (activeKind !== 'pdf' || !documentPdf) return;
    refitTimer = setTimeout(() => {
        if (rendering) { scheduleRefit(); return; }
        if (activeKind === 'pdf' && documentPdf && renderedFitWidth !== fitWidth()) void renderPage();
    }, 150);
}
const resizeObserver = new ResizeObserver(scheduleRefit); resizeObserver.observe(content);
window.chrome.webview.addEventListener('message', async ({ data }) => {
    if (data.type === 'selection-settings') {
        selectionPopupEnabled = data.enabled !== false; manualQuoteKey = ''; selectionChanged(); return;
    }
    if (data.type === 'pdf-page') { if (activeKind === 'pdf') setPage(page + Math.sign(data.direction)); return; }
    if (data.type === 'clear-selection') { clearSelection(); return; }
    if (data.type === 'quote-result') {
        quotePending = false;
        if (data.accepted) clearSelection();
        status.textContent = data.accepted ? localized('已加入聊天草稿。', 'Added to the chat draft.') : localized('未能加入聊天，请检查当前会话后重试。', 'Could not add to chat. Check the current session and retry.');
        controls(); return;
    }
    const current = ++generation;
    clearTimeout(refitTimer); endDrag(); renderedFitWidth = 0;
    cleanupPreview(content);
    renderTask?.cancel(); textTask?.cancel(); clearSelection(); quotePending = false;
    activeKind = data.kind;
    selectionPopupEnabled = data.selectionPopupEnabled !== false; manualQuoteKey = '';
    locale = data.locale === 'en' ? 'en' : 'zh';
    canQuote = (data.kind === 'pdf' || Object.hasOwn(officeRenderers, data.kind)) && data.canQuote === true;
    quoteActions.hidden = true;
    const previous = task; task = documentPdf = null;
    try { await previous?.destroy(); } catch { /* A cancelled worker has no result to retain. */ }
    if (current !== generation) return;
    rendering = false; content.replaceChildren(); status.textContent = '';
    document.body.style.background = data.background; document.body.style.color = data.foreground;
    document.body.style.fontSize = `${data.fontSize}px`;
    document.body.style.setProperty('--preview-bg', data.background);
    document.body.style.setProperty('--preview-fg', data.foreground);
    document.body.style.setProperty('--ui-font-scale', String((data.fontSize || 14) / 14));
    for (const [variable, value] of Object.entries({ border: data.border, elev: data.elevated, muted: data.muted, hover: data.hover, shadow: data.shadow }))
        if (value) document.body.style.setProperty(`--preview-${variable}`, value); else document.body.style.removeProperty(`--preview-${variable}`);
    document.body.classList.toggle('pdf', data.kind === 'pdf');
    for (const [id, zh, en, icon] of [['previous', '上一页', 'Previous page', 'chevron-left'], ['next', '下一页', 'Next page', 'chevron-right'],
        ['smaller', '缩小', 'Zoom out', 'minus'], ['larger', '放大', 'Zoom in', 'plus'], ['zoom', '重置缩放', 'Reset zoom', null]]) {
        const button = document.getElementById(id), label = localized(zh, en);
        button.setAttribute('aria-label', label); button.title = label;
        if (icon) { button.textContent = label; if (data.pdfIcons?.[icon]) button.innerHTML = data.pdfIcons[icon]; }
    }
    quoteActions.setAttribute('aria-label', localized('选区操作', 'Selection actions'));
    for (const [index, zh, en, icon] of [[0, '添加到对话', 'Add to chat', 'plus'],
        [1, '添加到对话并返回对话', 'Add to chat and jump back to chat', 'chat']]) {
        const label = localized(zh, en), text = document.createElement('span'); text.textContent = label;
        const button = quoteButtons[index]; button.replaceChildren();
        if (data.quoteIcons?.[icon]) button.innerHTML = data.quoteIcons[icon];
        button.append(text); button.setAttribute('aria-label', label);
    }
    document.body.classList.toggle('office', Object.hasOwn(officeRenderers, data.kind));
    document.body.classList.toggle('scientific', scientificKinds.has(data.kind));
    document.body.classList.toggle('image', data.kind === 'image');
    nav.hidden = data.kind !== 'pdf';
    document.getElementById('pdf-actions').hidden = data.kind !== 'pdf';
    content.style.display = data.kind === 'math' && !data.display ? 'inline-block' : 'block';
    try {
        if (data.kind === 'image') {
            if (data.format !== 'image/svg+xml' || typeof data.base64 !== 'string') throw new Error('Unsupported image format');
            const image = new Image(); image.alt = data.label || '';
            image.src = `data:image/svg+xml;base64,${data.base64}`;
            await image.decode();
            if (current !== generation) return;
            content.replaceChildren(image);
            send({ type: 'image-size', width: image.naturalWidth, height: image.naturalHeight });
        } else if (data.kind === 'math') {
            const { k: katex } = await import('./katex-Dn761jRB.js');
            if (current !== generation) return;
            katex.render(data.text, content, { displayMode: data.display, throwOnError: true, trust: false, maxExpand: 1000 });
            await document.fonts.ready;
            send({ type: 'size', height: document.body.scrollHeight, width: content.scrollWidth + 12 });
        } else if (scientificKinds.has(data.kind)) {
            await renderScientific(content, data.kind, { text: data.text, format: data.format, error: '无法预览科学数据' });
        } else if (Object.hasOwn(officeRenderers, data.kind)) {
            rendering = true; controls();
            await officeRenderers[data.kind](content, {
                bytes: Uint8Array.from(atob(data.base64), c => c.charCodeAt(0)),
                ...officeLabels(data),
            }, async payload => payload.bytes);
            if (current === generation) { rendering = false; controls(); }
        } else if (data.kind === 'pdf') {
            controls(); status.textContent = localized('正在加载 PDF…', 'Loading PDF…');
            const pdf = await import('./pdf.min.mjs');
            if (current !== generation) return;
            pdfLib = pdf;
            pdf.GlobalWorkerOptions.workerSrc = new URL('./pdf.worker.min.mjs', import.meta.url).href;
            // Keep both image and color conversion in the local worker. ICC
            // support is disabled by PDF.js when useWorkerFetch is false.
            const loading = pdf.getDocument({ data: Uint8Array.from(atob(data.base64), c => c.charCodeAt(0)),
                wasmUrl: new URL('./vendor-runtime/', import.meta.url).href, useWorkerFetch: true,
                isEvalSupported: false, enableXfa: false });
            task = loading;
            const loaded = await loading.promise;
            if (current !== generation) { await loaded.destroy(); return; }
            documentPdf = loaded;
            page = 1; scale = 1; await renderPage();
        }
    } catch (error) {
        if (current !== generation) return;
        status.textContent = data.kind === 'image' ? (data.imageError || '图像预览失败')
            : Object.hasOwn(officeRenderers, data.kind) ? officeLabels(data).error
            : localized(`${data.kind === 'math' ? '公式' : 'PDF'}预览失败：${error.message}`, `${data.kind === 'math' ? 'Formula' : 'PDF'} preview failed: ${error.message}`);
        if (data.kind === 'math') content.textContent = data.text;
        send({ type: 'size', height: document.body.scrollHeight, width: 400 });
    }
});
window.addEventListener('pagehide', () => { generation++; clearTimeout(refitTimer); resizeObserver.disconnect(); cleanupPreview(content); renderTask?.cancel(); textTask?.cancel(); void task?.destroy(); });
send({ type: 'ready' });
