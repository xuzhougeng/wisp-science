// Chat scroll follow (mirrors web-dist ConversationView pinned-at-bottom behavior).

const hooks = new Map();

function bottomGap(el) {
  return Math.max(0, el.scrollHeight - el.clientHeight - el.scrollTop);
}

function atBottom(el, eps = 2) {
  return bottomGap(el) <= eps;
}

function snapBottom(el) {
  const max = el.scrollHeight - el.clientHeight;
  if (max - el.scrollTop > 2) el.scrollTop = max;
}

/** @param {string} scrollerId @param {string} contentId */
export function attach_chat_scroll(scrollerId, contentId) {
  const scroller = document.getElementById(scrollerId);
  const content = document.getElementById(contentId);
  if (!scroller || !content || hooks.has(scrollerId)) return;

  let follow = true;
  let lastHeight = content.scrollHeight;
  // Timestamp of the last real user scroll gesture. The thread is re-rendered
  // on every streaming delta, which briefly collapses its height, clamps
  // scrollTop toward the top, and fires a spurious "scroll" event. Without this
  // guard that event unfollows and strands the view at the top mid-stream (#61).
  let lastUserScroll = -Infinity;
  const markUser = () => {
    lastUserScroll = performance.now();
  };

  const syncFollow = () => {
    if (atBottom(scroller)) {
      follow = true;
      return;
    }
    // Not at bottom: only treat it as an intentional scroll-up if a real gesture
    // happened just now. Reflow-driven scrolls leave `follow` untouched.
    if (performance.now() - lastUserScroll < 500) follow = false;
  };

  const onGrowth = () => {
    const h = content.scrollHeight;
    const grew = h > lastHeight;
    lastHeight = h;
    if (follow && grew) snapBottom(scroller);
    syncFollow();
  };

  scroller.style.overflowAnchor = "none";
  scroller.addEventListener("scroll", syncFollow, { passive: true });
  scroller.addEventListener(
    "wheel",
    (e) => {
      markUser();
      if (e.deltaY < 0) follow = false;
      else if (atBottom(scroller)) follow = true;
    },
    { passive: true },
  );
  scroller.addEventListener("touchmove", markUser, { passive: true });
  scroller.addEventListener("pointerdown", markUser, { passive: true });
  scroller.addEventListener("keydown", markUser, { passive: true });

  const ro = new ResizeObserver(() => onGrowth());
  ro.observe(content);

  hooks.set(scrollerId, {
    ro,
    onGrowth,
    snap: () => {
      follow = true;
      snapBottom(scroller);
      lastHeight = content.scrollHeight;
    },
  });

  follow = true;
  snapBottom(scroller);
}

/** @param {string} scrollerId */
export function notify_chat_scroll(scrollerId) {
  const hook = hooks.get(scrollerId);
  if (!hook) return;
  requestAnimationFrame(() => {
    requestAnimationFrame(() => hook.onGrowth());
  });
}

/** @param {string} scrollerId */
export function force_chat_scroll_bottom(scrollerId) {
  const hook = hooks.get(scrollerId);
  if (hook) {
    hook.snap();
    return;
  }
  const scroller = document.getElementById(scrollerId);
  if (scroller) snapBottom(scroller);
}
