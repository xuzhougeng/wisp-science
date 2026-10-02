// Scrollbar reveal: classic scrollbars keep their reserved gutter but the
// thumb stays invisible until their container actually scrolls (`.is-scrolling`
// in base.css), then fades back out after a short idle. Stream-driven
// programmatic snaps also count — the indicator is informative exactly while
// content is moving, which is when it should be visible.

const IDLE_MS = 800;

const timers = new WeakMap();

function mark(element) {
  element.classList.add("is-scrolling");
  const previous = timers.get(element);
  if (previous) clearTimeout(previous);
  timers.set(
    element,
    setTimeout(() => {
      timers.delete(element);
      element.classList.remove("is-scrolling");
    }, IDLE_MS),
  );
}

/** Install the document-wide capture listener once, at app startup. */
export function install_scrollbar_reveal() {
  // `scroll` does not bubble, but the capture phase still reaches every
  // scrolling element, so one listener covers sidebars, transcripts, modals
  // and the document itself without per-container wiring.
  document.addEventListener(
    "scroll",
    (event) => {
      const target = event.target;
      if (target instanceof Document) {
        const root = target.scrollingElement || target.documentElement;
        if (root instanceof Element) mark(root);
        return;
      }
      if (target instanceof Element) mark(target);
    },
    { capture: true, passive: true },
  );
}
