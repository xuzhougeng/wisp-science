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

  const syncFollow = () => {
    follow = atBottom(scroller);
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
      if (e.deltaY < 0) follow = false;
      else if (atBottom(scroller)) follow = true;
    },
    { passive: true },
  );

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
