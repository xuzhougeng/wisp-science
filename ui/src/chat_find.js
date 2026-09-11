import { reveal_chat_range } from "./scroll.js";

let state = null;
const ALL = "wisp-find";
const ACTIVE = "wisp-find-active";

// Only intercept the conversation shortcut. Editors, terminals and modal
// inputs retain their own find behavior, including in the split layout.
export function can_find_chat() {
  if (!CSS.highlights || typeof Highlight === "undefined") return false;
  const scroller = document.getElementById("chat-scroller");
  if (!scroller?.getClientRects().length) return false;
  const active = document.activeElement;
  if (active?.closest(".center-file, .terminal-dock, [role=dialog]")) return false;
  if (active?.matches("input, textarea, [contenteditable=true]")
      && !active.closest(".chat-stage, .composer")) return false;
  const rect = scroller.getBoundingClientRect();
  const top = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
  return !!top?.closest(".chat-stage");
}

export function focus_chat_find() {
  const input = document.getElementById("chat-find-input");
  input?.focus({ preventScroll: true });
  input?.select();
}

function collect(query) {
  if (!query || !state) return [];
  const matches = [];
  const pattern = new RegExp(query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "giu");
  for (const body of state.root.querySelectorAll(".msg .body")) {
    // Don't double-count nested message bodies or hidden/collapsed content.
    if (body.parentElement.closest(".body")) continue;
    const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT);
    let text = "";
    const spans = [];
    let lastBlock = null;
    while (walker.nextNode()) {
      const node = walker.currentNode;
      const parent = node.parentElement;
      if (!parent || parent.closest("button, script, style, textarea, [aria-hidden=true]")) continue;
      const probe = new Range();
      probe.selectNodeContents(node);
      if (!Array.from(probe.getClientRects()).some(rect => rect.width && rect.height)
          || getComputedStyle(parent).visibility !== "visible") continue;
      const block = parent.closest("p, div, li, pre, td, th, h1, h2, h3, h4, h5, h6");
      if (lastBlock && block !== lastBlock) text += "\n";
      lastBlock = block;
      spans.push({ node, start: text.length, end: text.length + node.length });
      text += node.data;
    }
    pattern.lastIndex = 0;
    let match;
    let ordinal = 0;
    while ((match = pattern.exec(text))) {
      const start = spans.find(span => span.end > match.index);
      const endOffset = match.index + match[0].length;
      const end = spans.find(span => span.end >= endOffset);
      if (!start || !end) continue;
      const range = new Range();
      range.setStart(start.node, match.index - start.start);
      range.setEnd(end.node, endOffset - end.start);
      const row = body.closest("[data-ui-index]");
      matches.push({ range, key: `${row?.dataset.uiIndex ?? ""}:${ordinal++}` });
    }
  }
  return matches;
}

function paint(reveal) {
  if (!state) return;
  const { matches, index, report } = state;
  const current = matches[index];
  if (CSS.highlights) {
    CSS.highlights.set(ALL, new Highlight(...matches.map(match => match.range)));
    CSS.highlights.set(ACTIVE, new Highlight(...(current ? [current.range] : [])));
  }
  report(current ? index + 1 : 0, matches.length);
  if (reveal && current) reveal_chat_range(current.range);
}

function rebuild(reset = false, reveal = false) {
  if (!state) return;
  const previous = state.matches[state.index];
  state.matches = collect(state.query);
  const retained = reset || !previous ? -1 : state.matches.findIndex(match =>
    (match.range.startContainer === previous.range.startContainer
      && match.range.startOffset === previous.range.startOffset)
    || match.key === previous.key);
  state.index = reset ? 0 : retained >= 0 ? retained : Math.min(state.index, state.matches.length - 1);
  if (state.index < 0) state.index = 0;
  paint(reveal);
}

export function start_chat_find(report) {
  stop_chat_find();
  const root = document.getElementById("chat-thread");
  if (!root) return;
  const observer = new MutationObserver(() => {
    cancelAnimationFrame(state?.pending);
    if (state) state.pending = requestAnimationFrame(() => rebuild());
  });
  state = { root, report, observer, query: "", matches: [], index: 0, pending: 0 };
  observer.observe(root, { childList: true, subtree: true, characterData: true, attributes: true,
    attributeFilter: ["open", "hidden", "style", "class"] });
}

export function query_chat_find(query) {
  if (!state) return;
  state.query = query;
  rebuild(true, true);
}

export function step_chat_find(direction) {
  if (!state) return;
  rebuild(); // Enter can race the pending DOM update notification.
  if (state.matches.length) {
    state.index = (state.index + direction + state.matches.length) % state.matches.length;
  }
  paint(true);
}

export function stop_chat_find() {
  state?.observer.disconnect();
  cancelAnimationFrame(state?.pending);
  state = null;
  CSS.highlights?.delete(ALL);
  CSS.highlights?.delete(ACTIVE);
}
