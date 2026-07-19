// WeChat-Reading-style saved-excerpt marks over the chat transcript, drawn
// with the CSS Custom Highlight API so the Leptos-managed DOM is never
// mutated. No-op on WebViews without CSS.highlights.

const NAME = "wisp-saved";
let needles = [];
let pending = 0;

// Whitespace-insensitive match: markdown rendering inserts/normalizes
// whitespace between inline elements, so both the saved excerpt and the DOM
// text are compared with all whitespace stripped.
function squash(text) {
  return text.replace(/\s+/g, "");
}

/** @param {Element} body @returns {{hay: string, map: [Text, number][]}} */
function indexBody(body) {
  const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT);
  let hay = "";
  const map = [];
  while (walker.nextNode()) {
    const node = walker.currentNode;
    const data = node.data;
    for (let i = 0; i < data.length; i++) {
      if (/\s/.test(data[i])) continue;
      hay += data[i];
      map.push([node, i]);
    }
  }
  return { hay, map };
}

function findRanges(needle, onRange) {
  if (!needle) return;
  for (const body of document.querySelectorAll("#chat-thread .msg .body")) {
    const { hay, map } = indexBody(body);
    let from = 0;
    let at;
    while ((at = hay.indexOf(needle, from)) !== -1) {
      const [startNode, startOffset] = map[at];
      const [endNode, endOffset] = map[at + needle.length - 1];
      const range = new Range();
      range.setStart(startNode, startOffset);
      range.setEnd(endNode, endOffset + 1);
      if (onRange(range)) return;
      from = at + needle.length;
    }
  }
}

function apply() {
  if (!("highlights" in CSS)) return;
  const ranges = [];
  for (const needle of needles) {
    findRanges(needle, (range) => {
      ranges.push(range);
      return false;
    });
  }
  if (ranges.length) CSS.highlights.set(NAME, new Highlight(...ranges));
  else CSS.highlights.delete(NAME);
}

/** @param {string} json JSON array of saved excerpt strings */
export function set_saved_marks(json) {
  try {
    needles = JSON.parse(json).map(squash).filter(Boolean);
  } catch {
    needles = [];
  }
  cancelAnimationFrame(pending);
  // Double rAF so the walk runs after Leptos has patched the transcript DOM.
  pending = requestAnimationFrame(() => {
    pending = requestAnimationFrame(apply);
  });
}

/** Scroll the first occurrence of a saved excerpt into view and flash it. */
export function reveal_saved_mark(text) {
  findRanges(squash(text), (range) => {
    const el = range.startContainer.parentElement;
    if (!el) return true;
    el.scrollIntoView({ behavior: "smooth", block: "center" });
    if ("highlights" in CSS) {
      CSS.highlights.set(`${NAME}-active`, new Highlight(range));
      setTimeout(() => CSS.highlights.delete(`${NAME}-active`), 1600);
    }
    return true;
  });
}
