// Ctrl/Cmd+A while reading a document selects that document, not every label
// in the app shell around it (sidebar, tabs, path chip, toolbars).
//
// A document surface carries `data-select-scope`. Inside it, the first
// `[data-select-all]` element in document order is what gets selected, so an
// outer root (a notebook) wins over the code views nested in it, and a code
// view's body is selected without its line-number gutter.

let anchor = null;

/** Remember where the user last pressed or moved focus; installed once. */
export function install_document_select_all() {
  const remember = (event) => {
    anchor = event.target instanceof Element ? event.target : null;
  };
  document.addEventListener("pointerdown", remember, true);
  document.addEventListener("focusin", remember, true);
}

// The surface must be what the user sees at its own center: a modal, drawer
// or hidden pane over it means the shortcut belongs to something else.
function onTop(region) {
  const rect = region.getBoundingClientRect();
  const left = Math.max(rect.left, 0);
  const right = Math.min(rect.right, window.innerWidth);
  const top = Math.max(rect.top, 0);
  const bottom = Math.min(rect.bottom, window.innerHeight);
  if (right <= left || bottom <= top) return false;
  const hit = document.elementFromPoint((left + right) / 2, (top + bottom) / 2);
  return !!hit && region.contains(hit);
}

/**
 * Select the text of the document the user is reading. Returns false when no
 * document surface owns the shortcut, which leaves the browser default.
 */
export function select_document_text() {
  const pointed = anchor?.isConnected ? anchor.closest("[data-select-scope]") : null;
  // Nothing pointed at (the tab strip, the sidebar, a fresh open): a center
  // document that fills the center is the obvious target. Split beside the
  // conversation, the user has to have pointed at the document.
  const center = document.querySelector("main.center:not(.split) > [data-select-scope]");
  const region = [pointed, center].find((candidate) => candidate && onTop(candidate));
  if (!region) return false;
  const target = region.querySelector("[data-select-all]");
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement) {
    target.focus({ preventScroll: true });
    target.select();
  } else if (target) {
    window.getSelection()?.selectAllChildren(target);
  }
  // No selectable text (an image, a loading view): still swallow the key —
  // selecting the app shell is never what the reader meant.
  return true;
}
