import { renderDocx, renderXlsx, renderPptx, cleanupPreview } from '/vendor-runtime/office-preview-runtime.mjs';
import { renderScientific } from '/vendor-runtime/scientific-preview.mjs';
import { documentSelection, clearDocumentSelection } from './selection.mjs';
const officeRenderers = { docx: renderDocx, xlsx: renderXlsx, pptx: renderPptx };
const scientificKinds = new Set(['structure', 'molecule', 'msa', 'fasta']);
const content = document.getElementById('content'), status = document.getElementById('status');
const nav = document.querySelector('nav');
let task, documentPdf, pdfLib, page = 1, scale = 1, renderTask, textTask, generation = 0, rendering = false;
let canQuote = false, quotePending = false, selectedText = '', activeKind = '', selectedQuote = null, sentActive = null;
const send = data => window.chrome.webview.postMessage(data);
const buttons = [...document.querySelectorAll('#pdf-actions button')];
const quoteButtons = [...document.querySelectorAll('[data-quote]')];
function selectionChanged() {
    selectedQuote = !rendering && canQuote ? documentSelection(content, activeKind, page) : null;
    selectedText = selectedQuote?.text ?? '';
    quoteButtons.forEach(button => { button.disabled = !canQuote || quotePending || !selectedText || selectedText.length > 32768; });
    // DOM mutations and selectionchange fire constantly while pages render and
    // sheets scroll; the host only mirrors the flag.
    if (sentActive !== !!selectedText) { sentActive = !!selectedText; send({ type: 'selection', active: sentActive }); }
}
// Always report after an explicit clear: the host resets its own flag then.
function clearSelection() { clearDocumentSelection(content); sentActive = null; selectionChanged(); }
document.addEventListener('selectionchange', selectionChanged);
content.addEventListener('click', () => queueMicrotask(selectionChanged));
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
    if (event.key === 'Escape') {
        event.preventDefault(); event.stopPropagation();
        if (selectedText) clearSelection(); else send({ type: 'escape' });
    }
}, true);
function controls() {
    buttons.forEach(button => button.disabled = rendering || quotePending || !documentPdf);
    document.getElementById('previous').disabled ||= page <= 1;
    document.getElementById('next').disabled ||= page >= (documentPdf?.numPages ?? 0);
    document.getElementById('page').textContent = `${page} / ${documentPdf?.numPages ?? 0}`;
    document.getElementById('zoom').textContent = `${Math.round(scale * 100)}%`;
    selectionChanged();
}
async function renderPage() {
    if (!documentPdf || rendering) return;
    const current = generation;
    rendering = true; clearSelection(); controls(); status.textContent = '正在渲染…';
    try {
        const pdfPage = await documentPdf.getPage(page);
        if (current !== generation) return;
        let viewport = pdfPage.getViewport({ scale });
        if (viewport.width * viewport.height > 16000000) viewport = pdfPage.getViewport({ scale: scale * Math.sqrt(16000000 / (viewport.width * viewport.height)) });
        const canvas = document.createElement('canvas');
        canvas.width = Math.ceil(viewport.width); canvas.height = Math.ceil(viewport.height);
        canvas.setAttribute('aria-label', `PDF 第 ${page} 页`);
        renderTask = pdfPage.render({ canvasContext: canvas.getContext('2d'), viewport });
        await renderTask.promise;
        if (current !== generation) return;
        const wrapper = document.createElement('div'); wrapper.className = 'pdf-page';
        wrapper.style.width = `${viewport.width}px`; wrapper.style.height = `${viewport.height}px`;
        wrapper.append(canvas); content.replaceChildren(wrapper); status.textContent = '';
        // A failed text layer must leave the readable page canvas intact.
        try {
            const layer = document.createElement('div'); layer.className = 'textLayer';
            layer.style.setProperty('--scale-factor', String(viewport.scale));
            layer.style.setProperty('--total-scale-factor', String(viewport.scale));
            wrapper.append(layer);
            textTask = new pdfLib.TextLayer({ textContentSource: pdfPage.streamTextContent(), container: layer, viewport });
            await textTask.render();
            if (current !== generation) return;
            if (!layer.textContent.trim()) status.textContent = '此页没有可选择的文字。';
        } catch (error) {
            if (current === generation && error?.name !== 'AbortException') status.textContent = '此页文字层不可用，仍可查看页面。';
        }
    } catch (error) { if (current === generation) status.textContent = `PDF 渲染失败：${error.message}`; }
    finally { if (current === generation) { rendering = false; controls(); } }
}
document.getElementById('previous').onclick = () => { page--; void renderPage(); };
document.getElementById('next').onclick = () => { page++; void renderPage(); };
document.getElementById('smaller').onclick = () => { scale = Math.max(.25, scale - .25); void renderPage(); };
document.getElementById('larger').onclick = () => { scale = Math.min(3, scale + .25); void renderPage(); };
window.chrome.webview.addEventListener('message', async ({ data }) => {
    if (data.type === 'clear-selection') { clearSelection(); return; }
    if (data.type === 'quote-result') {
        quotePending = false;
        if (data.accepted) clearSelection();
        status.textContent = data.accepted ? '已加入聊天草稿。' : '未能加入聊天，请检查当前会话后重试。';
        controls(); return;
    }
    const current = ++generation;
    cleanupPreview(content);
    renderTask?.cancel(); textTask?.cancel(); clearSelection(); quotePending = false;
    activeKind = data.kind;
    canQuote = (data.kind === 'pdf' || Object.hasOwn(officeRenderers, data.kind)) && data.canQuote === true;
    document.getElementById('quote-actions').hidden = !canQuote;
    const previous = task; task = documentPdf = null;
    try { await previous?.destroy(); } catch { /* A cancelled worker has no result to retain. */ }
    if (current !== generation) return;
    rendering = false; content.replaceChildren(); status.textContent = '';
    document.body.style.background = data.background; document.body.style.color = data.foreground;
    document.body.style.fontSize = `${data.fontSize}px`;
    document.body.style.setProperty('--preview-bg', data.background);
    document.body.style.setProperty('--preview-fg', data.foreground);
    document.body.classList.toggle('office', Object.hasOwn(officeRenderers, data.kind));
    document.body.classList.toggle('scientific', scientificKinds.has(data.kind));
    nav.hidden = data.kind !== 'pdf' && !canQuote;
    document.getElementById('pdf-actions').hidden = data.kind !== 'pdf';
    content.style.display = data.kind === 'math' && !data.display ? 'inline-block' : 'block';
    try {
        if (data.kind === 'math') {
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
                loading: '正在加载文档…', error: '无法预览此文档。文件可能损坏或包含暂不支持的内容。',
                formulaLabel: '公式', truncated: '工作簿较大，当前仅显示限定范围内的数据。',
            }, async payload => payload.bytes);
            if (current === generation) { rendering = false; controls(); }
        } else if (data.kind === 'pdf') {
            controls(); status.textContent = '正在加载 PDF…';
            const pdf = await import('./pdf.min.mjs');
            if (current !== generation) return;
            pdfLib = pdf;
            pdf.GlobalWorkerOptions.workerSrc = new URL('./pdf.worker.min.mjs', import.meta.url).href;
            const loading = pdf.getDocument({ data: Uint8Array.from(atob(data.base64), c => c.charCodeAt(0)), isEvalSupported: false, enableXfa: false });
            task = loading;
            const loaded = await loading.promise;
            if (current !== generation) { await loaded.destroy(); return; }
            documentPdf = loaded;
            page = 1; scale = 1; await renderPage();
        }
    } catch (error) {
        if (current !== generation) return;
        status.textContent = `${data.kind === 'math' ? '公式' : 'PDF'}预览失败：${error.message}`;
        if (data.kind === 'math') content.textContent = data.text;
        send({ type: 'size', height: document.body.scrollHeight, width: 400 });
    }
});
window.addEventListener('pagehide', () => { generation++; cleanupPreview(content); renderTask?.cancel(); textTask?.cancel(); void task?.destroy(); });
send({ type: 'ready' });
