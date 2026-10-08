// Documents enter as bounded JSON. Only the owning Swift model assigns paths.
import { renderDocx, renderXlsx, renderPptx, cleanupPreview } from '/vendor-runtime/office-preview-runtime.mjs';
import { renderScientific } from '/vendor-runtime/scientific-preview.mjs';
import { documentSelection, clearDocumentSelection } from './selection.mjs';
const content = document.getElementById('content'), status = document.getElementById('status');
const nav = document.querySelector('nav'), quoteButton = document.getElementById('quote');
const office = { docx: renderDocx, xlsx: renderXlsx, pptx: renderPptx };
let lastSelection = null;
let generation = 0, currentKind = '', canQuote = false, rendering = false, pending = false, htmlSelection = null;
const send = value => window.webkit.messageHandlers.preview.postMessage(value);
function selected() {
    if (!canQuote || rendering) return null;
    if (currentKind === 'html') return htmlSelection;
    if (currentKind === 'fasta') {
        const selection = window.getSelection();
        if (!selection?.rangeCount || selection.isCollapsed) return null;
        const range = selection.getRangeAt(0);
        if (!content.contains(range.startContainer) || !content.contains(range.endContainer)) return null;
        const text = selection.toString().trim();
        return text ? { text, location: { kind: 'fasta' } } : null;
    }
    return documentSelection(content, currentKind, 1);
}
function controls() {
    const selection = selected();
    quoteButton.disabled = pending || !selection || new TextEncoder().encode(selection.text).length > 65536;
    const active = !quoteButton.disabled;
    if (lastSelection !== active) { lastSelection = active; send({ type: 'selection', active }); }
}
window.nativePreviewQuote = () => quoteButton.onclick();
document.addEventListener('selectionchange', controls);
content.addEventListener('click', () => queueMicrotask(controls));
new MutationObserver(controls).observe(content, { childList: true, subtree: true });
quoteButton.addEventListener('pointerdown', event => event.preventDefault());
quoteButton.onclick = () => {
    controls(); if (quoteButton.disabled) return;
    pending = true; const selection = selected(); controls(); send({ type: 'quote', ...selection });
};
window.addEventListener('message', event => {
    const frame = content.querySelector('iframe.rp-native-html');
    if (!frame || event.source !== frame.contentWindow || event.origin !== 'null' || event.data?.type !== 'native-html-selection' || event.data?.generation !== generation) return;
    htmlSelection = typeof event.data.text === 'string' && new TextEncoder().encode(event.data.text).length <= 65536 && event.data.text.trim()
        ? { text: event.data.text, location: { kind: 'html' } } : null;
    controls();
});
window.nativePreviewReceive = async ({ data }) => {
    if (data.type === 'quote-result') {
        pending = false;
        if (data.accepted) { htmlSelection = null; clearDocumentSelection(content); }
        controls(); return;
    }
    const current = ++generation;
    cleanupPreview(content); clearDocumentSelection(content); htmlSelection = null; pending = false;
    currentKind = data.kind; canQuote = data.canQuote === true; rendering = true;
    document.body.classList.toggle('office', Object.hasOwn(office, data.kind));
    content.replaceChildren(); status.textContent = data.loading;
    document.body.style.background = data.background; document.body.style.color = data.foreground;
    document.body.style.setProperty('--preview-bg', data.background); document.body.style.setProperty('--preview-fg', data.foreground);
    document.body.style.fontSize = `${data.fontSize}px`;
    quoteButton.textContent = data.quoteLabel; nav.hidden = true; controls();
    try {
        if (Object.hasOwn(office, data.kind)) {
            const bytes = Uint8Array.from(atob(data.base64), c => c.charCodeAt(0));
            if (bytes.length > 32 * 1024 * 1024) throw new Error(data.oversized);
            await office[data.kind](content, { bytes, loading: data.loading, error: data.error, formulaLabel: data.formulaLabel, truncated: data.truncatedLabel }, async value => value.bytes);
        } else if (data.kind === 'html') {
            if (typeof data.text !== 'string' || new TextEncoder().encode(data.text).length > 32 * 1024 * 1024) throw new Error(data.oversized);
            const frame = document.createElement('iframe'); frame.className = 'rp-native-html'; frame.title = data.title;
            frame.setAttribute('sandbox', 'allow-scripts'); frame.referrerPolicy = 'no-referrer';
            const parsed = new DOMParser().parseFromString(data.text, 'text/html');
            // This policy is first and intersects with any policy in the file.
            const policy = parsed.createElement('meta'); policy.httpEquiv = 'Content-Security-Policy';
            policy.content = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; connect-src 'none'; font-src data:; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
            parsed.head.prepend(policy);
            const theme = parsed.createElement('style');
            theme.textContent = `html,body{color:${data.foreground};background:${data.background}}body{margin:16px;font:${data.fontSize}px system-ui}`;
            policy.after(theme);
            parsed.querySelectorAll('iframe,object,embed,base,meta[http-equiv="refresh" i]').forEach(node => node.remove());
            const script = parsed.createElement('script');
            script.textContent = `document.addEventListener('selectionchange', () => {const s=window.getSelection();const text=s&&!s.isCollapsed?s.toString().trim():'';parent.postMessage({type:'native-html-selection',generation:${current},text:text.slice(0,32768)},'*')});`;
            parsed.body.append(script); frame.srcdoc = '<!doctype html>' + parsed.documentElement.outerHTML; content.append(frame);
            content.__wispPreviewCleanup = () => frame.remove();
        } else {
            await renderScientific(content, data.kind, { text: data.text, format: data.format, truncated: data.truncated, error: data.error });
        }
        if (current === generation) { status.textContent = ''; rendering = false; controls(); }
    } catch (error) { if (current === generation) { status.textContent = data.error + ' ' + error.message; rendering = false; controls(); } }
};
window.addEventListener('pagehide', () => { generation++; cleanupPreview(content); content.replaceChildren(); });
send({ type: 'ready' });
