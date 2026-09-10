import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const scrollSource = readFileSync(resolve(__dirname, "../../ui/src/scroll.js"), "utf8");

test("a new conversation with a shorter transcript opens at the bottom (#1197)", async ({ page }) => {
  await page.route("**/scroll-regression", (route) => route.fulfill({
    contentType: "text/html",
    body: '<div id="chat-scroller" style="height:200px;overflow:auto"><div id="chat-thread" style="height:1600px"></div></div>',
  }));
  await page.goto("/scroll-regression");
  await page.evaluate(async (source) => {
    const scroll = await import(URL.createObjectURL(new Blob([source], { type: "text/javascript" })));
    (window as any).scrollHelper = scroll;
    scroll.attach_chat_scroll("chat-scroller", "chat-thread");
    scroll.switch_chat_scroll("chat-scroller", "first");
  }, scrollSource);
  const scroller = page.locator("#chat-scroller");
  await expect.poll(() => scroller.evaluate((el) => el.scrollTop)).toBe(1400);
  await scroller.evaluate((el) => {
    el.dispatchEvent(new WheelEvent("wheel", { deltaY: -100 }));
    el.scrollTop = 100;
  });
  await expect.poll(() => scroller.evaluate((el) => el.scrollTop)).toBe(100);
  await page.evaluate(() => {
    document.getElementById("chat-thread")!.style.height = "900px";
    (window as any).scrollHelper.switch_chat_scroll("chat-scroller", "second");
  });
  await expect.poll(() => scroller.evaluate((el) => el.scrollTop)).toBe(700);
  // Returning to a conversation still restores the user's reading bookmark.
  await page.evaluate(() => {
    document.getElementById("chat-thread")!.style.height = "1600px";
    (window as any).scrollHelper.switch_chat_scroll("chat-scroller", "first");
  });
  await expect.poll(() => scroller.evaluate((el) => el.scrollTop)).toBe(100);
});
