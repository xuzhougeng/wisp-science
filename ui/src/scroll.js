// Chat scroll follow (mirrors web-dist ConversationView pinned-at-bottom behavior).

const hooks = new Map();
const chatPositions = new Map();

// ponytail: single chat scroller, so the jump pill id is a constant.
const JUMP_PILL_ID = "chat-jump-pill";

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
  let readingTop = scroller.scrollTop;
  // The last scrollTop this module set itself. Real scroll events carry any
  // other value, so `readingTop` can track the user without a gesture-timing
  // guess — wheel events delayed past the 500ms window by a busy main thread,
  // held navigation keys, and scrollbar drags included (#61's window stays as
  // the follow/unfollow guard; it just no longer gatekeeps the bookmark).
  let programmaticTop = scroller.scrollTop;
  let activeSession = null;
  let restoreGeneration = 0;
  let hidden = false;
  let pointerDown = false;
  let jumping = false;
  const setFollow = (value) => {
    follow = value;
    scroller.style.overflowAnchor = value ? "none" : "auto";
  };
  // Timestamp of the last real user scroll gesture. The thread is re-rendered
  // on every streaming delta, which briefly collapses its height, clamps
  // scrollTop toward the top, and fires a spurious "scroll" event. Without this
  // guard that event unfollows and strands the view at the top mid-stream (#61).
  let lastUserScroll = -Infinity;
  const markUser = () => {
    lastUserScroll = performance.now();
  };
  const rememberProgrammatic = () => {
    programmaticTop = scroller.scrollTop;
  };
  const snapFollow = (force = false) => {
    // Write-only follow snap: skip the assignment when we already wrote this
    // max. Repeated programmatic scrollTop writes suppress Chromium overflow
    // anchoring, which #663 needs after the user scrolls away. `force` is for
    // jump-to-latest / rebuild clamps, where scrollTop moved without max
    // changing.
    const max = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    if (force || max - programmaticTop > 2) {
      scroller.scrollTop = max;
    }
    programmaticTop = max;
    readingTop = max;
  };
  const restoreBookmark = () => {
    const max = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    programmaticTop = Math.min(readingTop, max);
    scroller.scrollTop = programmaticTop;
  };
  const parkHere = () => {
    setFollow(false);
    readingTop = scroller.scrollTop;
    programmaticTop = scroller.scrollTop;
    lastHeight = content.scrollHeight;
    syncPill();
  };

  // Jump-to-latest pill: visible when the view is scrolled away from the
  // bottom. Class toggle on a static element — no reactive rebuild involved.
  const syncPill = () => {
    const pill = document.getElementById(JUMP_PILL_ID);
    if (!pill) return;
    pill.classList.toggle("visible", !follow && bottomGap(scroller) > 48);
  };

  const syncFollow = () => {
    if (jumping) {
      parkHere();
      return;
    }
    const userGesture = pointerDown || performance.now() - lastUserScroll < 500;
    if (atBottom(scroller)) {
      // A rebuild can shrink the thread to the viewport so scrollTop=0 is
      // temporarily "at bottom". Don't resume follow-bottom from that clamp
      // when the user was parked mid-thread.
      if (follow || userGesture) {
        setFollow(true);
        rememberProgrammatic();
        readingTop = scroller.scrollTop;
        syncPill();
        return;
      }
      restoreBookmark();
      syncPill();
      return;
    }
    // A click/drag on the scroller (tool cards, scrollbar) keeps `pointerDown`
    // until pointerup. Wheel/touch/key still use the 500ms window. Clicks
    // alone must not count as a scroll-up: tool-result rebuilds clamp
    // scrollTop to 0, and treating that click as a gesture parked the view
    // at the top (#927).
    if (userGesture) {
      // A rebuild clamp jumps from the followed bottom to ~0 in one event.
      // Don't treat that as the user scrolling to the top (#927).
      if (
        follow
        && scroller.scrollTop < 8
        && readingTop > Math.max(scroller.clientHeight, 200)
      ) {
        snapFollow(true);
        syncPill();
        return;
      }
      setFollow(false);
      readingTop = scroller.scrollTop;
      syncPill();
      return;
    }
    // Reflow-driven clamp. Scroll events fire before paint, so an instant
    // snap here means the clamped position is never painted — without it the
    // view visibly bounces on every thinking delta. When parked, restore the
    // bookmark if the thread collapsed toward 0 (#927); if scrollTop moved
    // down, that is overflow-anchor compensating a prepend (#663) — keep it.
    if (follow) {
      snapFollow(true);
    } else if (scroller.scrollTop + 2 < readingTop) {
      restoreBookmark();
    } else {
      readingTop = scroller.scrollTop;
      programmaticTop = scroller.scrollTop;
    }
    syncPill();
  };

  const onGrowth = (observedHeight = content.scrollHeight) => {
    const h = observedHeight;
    // Center-file tabs hide the chat with display:none. Ignore that temporary
    // zero-size layout and restore the exact reading position when the chat
    // becomes visible again instead of treating the reveal as new content.
    if (scroller.clientHeight === 0 || h === 0) {
      hidden = true;
      return;
    }
    if (hidden) {
      hidden = false;
      lastHeight = h;
      if (follow) {
        snapFollow(true);
      } else {
        restoreBookmark();
      }
      syncPill();
      return;
    }
    const grew = h > lastHeight;
    lastHeight = h;
    if (follow) {
      // ResizeObserver already coalesces streaming DOM changes. Keep the hot
      // path to one bottom snap and skip the row/viewport geometry used only
      // by the scroll-away pill — reading gap geometry here forces a layout
      // on every streaming frame, so the pill is only synced when it could
      // actually be visible (not following).
      snapFollow();
      return;
    }
    if (grew) {
      // Parked: overflow-anchor keeps the visible rows stable on prepend.
      // Do not restoreBookmark here — that writes the pre-prepend scrollTop
      // and undoes the compensation (#663).
      syncPill();
      return;
    }
    syncFollow();
  };

  setFollow(true);
  scroller.addEventListener("scroll", syncFollow, { passive: true });
  scroller.addEventListener(
    "wheel",
    (e) => {
      markUser();
      if (e.deltaY < 0) setFollow(false);
      else if (atBottom(scroller)) setFollow(true);
    },
    { passive: true },
  );
  scroller.addEventListener("touchmove", markUser, { passive: true });
  scroller.addEventListener(
    "pointerdown",
    () => {
      pointerDown = true;
    },
    { passive: true },
  );
  const endPointer = () => {
    pointerDown = false;
  };
  window.addEventListener("pointerup", endPointer, { passive: true });
  window.addEventListener("pointercancel", endPointer, { passive: true });
  scroller.addEventListener("keydown", markUser, { passive: true });

  const ro = new ResizeObserver((entries) => onGrowth(entries[0]?.contentRect.height));
  ro.observe(content);

  hooks.set(scrollerId, {
    ro,
    onGrowth,
    unfollow: parkHere,
    jumpTo: (el) => {
      jumping = true;
      setFollow(false);
      el.scrollIntoView({ block: "start" });
      parkHere();
      jumping = false;
    },
    snap: () => {
      const requested = performance.now();
      setFollow(true);
      snapFollow(true);
      lastHeight = content.scrollHeight;
      syncPill();
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          if (lastUserScroll < requested) {
            setFollow(true);
            snapFollow(true);
            lastHeight = content.scrollHeight;
            syncPill();
          }
        });
      });
    },
    switchSession: (sessionId) => {
      if (activeSession !== sessionId) {
        if (activeSession) {
          chatPositions.set(activeSession, {
            top: scroller.scrollTop,
            follow,
          });
        }
        activeSession = sessionId;
      }

      const generation = ++restoreGeneration;
      const saved = chatPositions.get(sessionId);
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          if (generation !== restoreGeneration || activeSession !== sessionId) return;
          if (!saved || saved.follow) {
            setFollow(true);
            snapFollow();
          } else {
            setFollow(false);
            readingTop = saved.top;
            restoreBookmark();
          }
          lastHeight = content.scrollHeight;
          syncPill();
        });
      });
    },
  });

  setFollow(true);
  snapFollow();
}

/** Save the previous conversation and restore this conversation after render.
 * Calling this again for the same session reapplies its saved position after an
 * asynchronous transcript load without overwriting the saved state.
 * @param {string} scrollerId @param {string} sessionId */
export function switch_chat_scroll(scrollerId, sessionId) {
  hooks.get(scrollerId)?.switchSession(sessionId);
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
  if (!scroller) return;
  snapBottom(scroller);
  requestAnimationFrame(() => requestAnimationFrame(() => snapBottom(scroller)));
}

/** @param {string} scrollerId @param {string} contentId */
export function preserve_chat_scroll_on_prepend(scrollerId, contentId) {
  const scroller = document.getElementById(scrollerId);
  const content = document.getElementById(contentId);
  if (!scroller || !content) return;
  const oldHeight = content.scrollHeight;
  const oldTop = scroller.scrollTop;
  const oldAnchor = scroller.style.overflowAnchor;
  scroller.style.overflowAnchor = "none";
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      scroller.scrollTop = oldTop + content.scrollHeight - oldHeight;
      scroller.style.overflowAnchor = oldAnchor;
    });
  });
}

// Run output panels (chat monitor card, Runs modal) are rebuilt from scratch
// on every poll, so any per-element scroll state is lost and the view can
// never stay pinned to the latest output (#654). Keep the follow state here,
// keyed by run id, and re-apply it to each fresh element after a refresh.
const runOutputFollow = new Map();
const attachedRunOutputs = new WeakSet();

export function follow_run_outputs() {
  const apply = () => {
    document.querySelectorAll("[data-run-output-for]").forEach((el) => {
      const key = el.getAttribute("data-run-output-for");
      let state = runOutputFollow.get(key);
      if (!state) {
        state = { follow: true, top: 0 };
        runOutputFollow.set(key, state);
      }
      if (!attachedRunOutputs.has(el)) {
        attachedRunOutputs.add(el);
        // Scroll anchoring would fight the explicit snap on rebuild.
        el.style.overflowAnchor = "none";
        el.addEventListener(
          "scroll",
          () => {
            state.top = el.scrollTop;
            state.follow = atBottom(el);
          },
          { passive: true },
        );
      }
      if (state.follow) {
        snapBottom(el);
      } else {
        // A scrolled-up user keeps their place across the rebuild; the tail
        // buffer may have dropped lines, so clamp instead of trusting `top`.
        const max = Math.max(0, el.scrollHeight - el.clientHeight);
        el.scrollTop = Math.min(state.top, max);
      }
    });
  };
  // The first pass runs before the frame is painted, so a rebuilt panel never
  // shows its top edge. The second is the safety net for a panel whose height
  // only settles after layout (wrapped lines, late fonts).
  requestAnimationFrame(() => {
    apply();
    requestAnimationFrame(apply);
  });
}

/** @param {string} scrollerId @param {string} selector */
export function jump_chat_scroll(scrollerId, selector) {
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      const target = document.querySelector(selector);
      if (!target) return;
      const hook = hooks.get(scrollerId);
      if (hook) {
        hook.jumpTo(target);
        return;
      }
      target.scrollIntoView({ block: "start" });
    });
  });
}
