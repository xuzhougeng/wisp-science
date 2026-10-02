const content = document.getElementById('content'), status = document.getElementById('status');
const nav = document.querySelector('nav');
let task, documentPdf, page = 1, scale = 1, renderTask, generation = 0, rendering = false;
const send = data => window.chrome.webview.postMessage(data);
const buttons = [...nav.querySelectorAll('button')];
function controls() {
    buttons.forEach(button => button.disabled = rendering || !documentPdf);
    document.getElementById('previous').disabled ||= page <= 1;
    document.getElementById('next').disabled ||= page >= (documentPdf?.numPages ?? 0);
    document.getElementById('page').textContent = `${page} / ${documentPdf?.numPages ?? 0}`;
    document.getElementById('zoom').textContent = `${Math.round(scale * 100)}%`;
}
async function renderPage() {
    if (!documentPdf || rendering) return;
    const current = generation;
    rendering = true; controls(); status.textContent = '正在渲染…';
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
        content.replaceChildren(canvas); status.textContent = '';
    } catch (error) { if (current === generation) status.textContent = `PDF 渲染失败：${error.message}`; }
    finally { if (current === generation) { rendering = false; controls(); } }
}
document.getElementById('previous').onclick = () => { page--; void renderPage(); };
document.getElementById('next').onclick = () => { page++; void renderPage(); };
document.getElementById('smaller').onclick = () => { scale = Math.max(.25, scale - .25); void renderPage(); };
document.getElementById('larger').onclick = () => { scale = Math.min(3, scale + .25); void renderPage(); };
window.chrome.webview.addEventListener('message', async ({ data }) => {
    const current = ++generation;
    renderTask?.cancel(); const previous = task; task = documentPdf = null;
    try { await previous?.destroy(); } catch { /* A cancelled worker has no result to retain. */ }
    if (current !== generation) return;
    rendering = false; content.replaceChildren(); status.textContent = '';
    document.body.style.background = data.background; document.body.style.color = data.foreground;
    document.body.style.fontSize = `${data.fontSize}px`;
    nav.hidden = data.kind !== 'pdf';
    content.style.display = data.kind === 'math' && !data.display ? 'inline-block' : 'block';
    try {
        if (data.kind === 'math') {
            const { k: katex } = await import('./katex-Dn761jRB.js');
            if (current !== generation) return;
            katex.render(data.text, content, { displayMode: data.display, throwOnError: true, trust: false, maxExpand: 1000 });
            await document.fonts.ready;
            send({ type: 'size', height: document.body.scrollHeight, width: content.scrollWidth + 12 });
        } else if (data.kind === 'pdf') {
            controls(); status.textContent = '正在加载 PDF…';
            const pdf = await import('./pdf.min.mjs');
            if (current !== generation) return;
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
window.addEventListener('pagehide', () => { generation++; renderTask?.cancel(); void task?.destroy(); });
send({ type: 'ready' });
