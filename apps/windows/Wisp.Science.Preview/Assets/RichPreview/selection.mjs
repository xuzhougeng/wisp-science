// Selection metadata is local to the mounted document. File/scope identity is
// supplied only by the native host, never by markup or a document hyperlink.
export function documentSelection(content, kind, page) {
    if (kind === 'xlsx') {
        const cell = content.querySelector('.rp-xlsx-cell.selected');
        if (!cell || !cell.dataset.sheet || !cell.dataset.cells) return null;
        const value = cell.textContent.trim(), formula = cell.dataset.formula;
        const text = formula ? `值：${value}\n公式：=${formula}` : value;
        return text ? { text, location: { kind, sheet: cell.dataset.sheet, cells: cell.dataset.cells } } : null;
    }
    const selection = window.getSelection();
    if (!selection?.rangeCount || selection.isCollapsed) return null;
    const range = selection.getRangeAt(0), text = selection.toString().trim();
    if (!text || !content.contains(range.startContainer) || !content.contains(range.endContainer)) return null;
    const element = node => node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
    if (kind === 'pdf') {
        const layer = content.querySelector('.textLayer');
        return layer?.contains(range.startContainer) && layer.contains(range.endContainer) ? { text, page } : null;
    }
    const selector = kind === 'docx' ? 'section.docx' : kind === 'pptx' ? '[data-slide-index]' : null;
    if (!selector) return null;
    const first = element(range.startContainer)?.closest(selector), last = element(range.endContainer)?.closest(selector);
    if (!first || !last) return null;
    const pages = [...content.querySelectorAll(selector)];
    const number = node => kind === 'docx' ? pages.indexOf(node) + 1 : Number(node.dataset.slideIndex) + 1;
    const start = number(first), end = number(last);
    if (!Number.isInteger(start) || start < 1 || !Number.isInteger(end) || end < start) return null;
    return { text, location: { kind, page: start, endPage: end } };
}

export function clearDocumentSelection(content) {
    window.getSelection()?.removeAllRanges();
    content.querySelectorAll('.rp-xlsx-cell.selected').forEach(cell => cell.classList.remove('selected'));
}

// Anchor to the topmost visible fragment, independent of drag direction.
// Coordinates stay inside the renderer and never enter the quote payload.
export function documentSelectionAnchor(content, kind) {
    const bounds = content.getBoundingClientRect();
    let clip = { left: Math.max(0, bounds.left), top: Math.max(0, bounds.top),
        right: Math.min(innerWidth, bounds.right), bottom: Math.min(innerHeight, bounds.bottom) };
    let rects;
    if (kind === 'xlsx') {
        const cell = content.querySelector('.rp-xlsx-cell.selected');
        if (!cell) return null;
        const grid = cell.closest('.rp-xlsx-grid')?.getBoundingClientRect();
        if (grid) clip = { left: Math.max(clip.left, grid.left), top: Math.max(clip.top, grid.top),
            right: Math.min(clip.right, grid.right), bottom: Math.min(clip.bottom, grid.bottom) };
        rects = [cell.getBoundingClientRect()];
    } else {
        const selection = window.getSelection();
        if (!selection?.rangeCount || selection.isCollapsed) return null;
        rects = [...selection.getRangeAt(0).getClientRects()];
    }
    return rects.map(rect => ({ left: Math.max(rect.left, clip.left), top: Math.max(rect.top, clip.top),
        right: Math.min(rect.right, clip.right), bottom: Math.min(rect.bottom, clip.bottom) }))
        .filter(rect => rect.right > rect.left && rect.bottom > rect.top)
        .sort((a, b) => a.top - b.top || a.left - b.left)[0] ?? null;
}
