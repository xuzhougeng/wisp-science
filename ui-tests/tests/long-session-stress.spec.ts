// Long-session stress coverage for the dead-window reports: dense tool
// events over a long transcript must not wedge the renderer, chat media must
// load as blob object URLs instead of base64 data URLs, and the idle thread
// must stay DOM-stable so timers alone cannot keep the main thread busy.

import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  // Install the Tauri bridge mock before the page's wasm runs.
  await page.addInitScript(tauriMock);
});

// The app boots to the Projects landing screen; open a real project (not
// the "Example project" card) to reach the chat UI the tests assert against.
async function enterApp(page: Page, path = "/") {
  await page.goto(path);
  await page.locator(".proj-card-main").first().click();
  await expect(
    page.locator(".sidebar").getByRole("button", { name: "New session" }),
  ).toBeVisible();
}

function composer(page: Page) {
  return page.locator(".composer-inner textarea").first();
}

test("idle long-session thread stays DOM-stable", async ({ page }) => {
  await enterApp(page, "/?mockLongPages=8&mockLongRows=40&mockLongRowBytes=2048");
  await expect(page.getByText(/Window page 0 row 39/)).toBeVisible({ timeout: 15_000 });

  // Any run polling or projection epoch that still rebuilds rows shows up as
  // chat DOM mutations. A few are tolerated (clock labels), a storm is not.
  const mutations = await page.evaluate(
    () =>
      new Promise<number>((resolve) => {
        let count = 0;
        const scroller = document.getElementById("chat-thread") ?? document.body;
        const observer = new MutationObserver((records) => {
          count += records.length;
        });
        observer.observe(scroller, { childList: true, subtree: true, characterData: true });
        setTimeout(() => {
          observer.disconnect();
          resolve(count);
        }, 4000);
      }),
  );
  expect(mutations).toBeLessThan(50);
});

test("dense tool events over a long session keep the renderer responsive", async ({ page }) => {
  await enterApp(page, "/?mockLongPages=8&mockLongRows=40&mockLongRowBytes=2048");
  await expect(page.getByText(/Window page 0 row 39/)).toBeVisible({ timeout: 15_000 });

  await composer(page).fill("TOOLSCROLLTEST");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Tools finished at the tail.")).toBeVisible({ timeout: 15_000 });

  // The renderer must still service rAF inside a normal frame budget after a
  // burst of tool results rebuilt parts of the thread.
  const frameGapMs = await page.evaluate(
    () =>
      new Promise<number>((resolve) => {
        const start = performance.now();
        requestAnimationFrame(() => resolve(performance.now() - start));
      }),
  );
  // CIWebView clocks make tight thresholds flaky; this only fails when the
  // main thread is genuinely wedged (multi-second stalls were the reports).
  expect(frameGapMs).toBeLessThan(1500);
});

test("chat media loads as blob object URLs, never base64 data URLs", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("ARTIFACTATTRIBUTION");
  await page.getByRole("button", { name: "Send" }).click();
  await page.locator(".message-artifacts-label").click();
  await page.locator(".generated-directory > summary").filter({ hasText: "results" }).click();
  const card = page.locator('.message-artifact-card[data-artifact-name="new.png"]');
  await expect(card).toBeVisible({ timeout: 10_000 });

  // The artifact thumbnail must come from the shared media cache (blob:) —
  // a data: URL here means the base64 inline path crept back.
  await expect
    .poll(async () => {
      const src = await card.locator("img").first().getAttribute("src").catch(() => null);
      return typeof src === "string" && src.length > 0 ? src : "";
    })
    .toContain("blob:");
  const src = await card.locator("img").first().getAttribute("src");
  expect(src).not.toContain("data:");
});
