import type { Locator } from "@playwright/test";

// The chat thread follows its latest row (ui/src/scroll.js): a scroll that no
// wheel, touch, key or pointer gesture explains is treated as a reflow clamp
// and undone on the next frame — snapped back to the bottom while following,
// or to the parked bookmark otherwise. Playwright's click scrolls its target
// into view programmatically, so a control above the fold is scrolled away
// again one frame later. When that frame lands between mousedown and mouseup
// the click fires on a common ancestor and the control's handler never runs;
// Playwright still reports the click as done (it only verifies the first
// event's hit target). CI's loaded runners hit that window; a fast laptop
// rarely does.
//
// Reveal the control the way a reader would: a wheel gesture parks the thread
// and the element is centred in the same task, so the position sticks.
export async function revealInChat(target: Locator) {
  await target.evaluate((el) => {
    document.getElementById("chat-scroller")?.dispatchEvent(new WheelEvent("wheel", { deltaY: -1 }));
    el.scrollIntoView({ block: "center" });
  });
}

// Reveal, then click: the click itself needs no scroll, so nothing moves
// between mousedown and mouseup.
export async function clickInChat(target: Locator) {
  await revealInChat(target);
  await target.click();
}
