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
