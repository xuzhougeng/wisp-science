import { trackInvokes, waitForInvokes } from "./async-observers";
import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

async function setup(page: Page) {
  await page.addInitScript(tauriMock);
  await page.goto("/");
  await expect.poll(() => page.evaluate(() => Boolean((window as any).__tauriListenerReady?.("open-session")))).toBe(true);
  await page.evaluate(() => {
    const w = window as any;
    const original = w.__TAURI__.core.invoke;
    w.historyRequests = [];
    w.historyDelay = 0;
    w.historyFail = false;
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
      if (cmd !== "load_session" || arg("id") !== "compacted") return original(cmd, args);
      const before = arg("beforeSeq") ?? null;
      w.historyRequests.push(before);
      if (before !== null && w.historyDelay) await new Promise(resolve => setTimeout(resolve, w.historyDelay));
      if (before !== null && w.historyFail) throw new Error("history temporarily unavailable");
      // Exact visual windows from the real Store 50 -> keep 10 -> add 30 replay.
      const [start, end, next] = before === 43 ? [1, 60, 3] : before === 3 ? [0, 1, null] : [60, 80, 43];
      return {
        items: Array.from({ length: end! - start! }, (_, n) => start! + n).flatMap(i => [
          { role: "user", text: `question ${i + 1}` },
          { role: "assistant", text: `answer ${i + 1}` },
        ]),
        user_offset: start, next_before_seq: next,
        outline: before === null ? Array.from({ length: 80 }, (_, i) => ({
          user_index: i, seq: null, text: `question ${i + 1}`,
          sent_at: 1783478400 + i * 60, response_at: 1783478430 + i * 60,
        })) : [],
      };
    };
  });
  await page.evaluate(() => (window as any).__tauriEmit("open-session", { projectId: "other", sessionId: "compacted" }));
  await expect(page.locator('[data-user-index="79"]')).toContainText("question 80");
  await page.getByTestId("conversation-outline-toggle").click();
}

async function assertOutline(page: Page) {
  const texts = await page.locator(".conversation-outline-text").allTextContents();
  expect(texts).toEqual(Array.from({ length: 80 }, (_, i) => `question ${i + 1}`));
}

test("compacted history keeps 80 exact questions, timestamps and first/middle/latest jumps", async ({ page }) => {
  await setup(page);
  await trackInvokes(page, ["load_session"]);
  await assertOutline(page);
  for (const index of [79, 45, 0, 79]) {
    await page.getByTestId("conversation-outline").getByRole("button", { name: `question ${index + 1}`, exact: true }).click();
    const row = page.locator(`[data-user-index="${index}"]`);
    await expect(row).toContainText(`question ${index + 1}`);
    await expect(row).toHaveClass(/outline-target/);
    await expect(row.locator(".user-message-time")).toHaveAttribute("data-timestamp", String(1783478400 + index * 60));
    await expect.poll(() => row.evaluate(element => {
      const scroller = document.querySelector("#chat-scroller")!;
      const box = element.getBoundingClientRect();
      const viewport = scroller.getBoundingClientRect();
      return { index: Number(element.getAttribute("data-user-index")), visible: box.top >= viewport.top - 1 && box.bottom <= viewport.bottom + 1 };
    })).toEqual({ index, visible: true });
    await assertOutline(page);
  }
  await page.screenshot({ path: "test-results/compacted-history.png", animations: "disabled" });
});

test("loading older pages never changes the full outline or duplicates questions", async ({ page }) => {
  await setup(page);
  await trackInvokes(page, ["load_session"]);
  await page.getByRole("button", { name: "Hide conversation outline" }).click();
  for (let attempt = 0; attempt < 12; attempt++) {
    const older = page.locator(".transcript-load-older").first();
    if (await page.locator('[data-user-index="0"]').count()) break;
    await older.click();
  }
  await expect(page.locator('[data-user-index="0"]')).toContainText("question 1");
  await page.getByTestId("conversation-outline-toggle").click();
  await assertOutline(page);
  await expect(page.locator(".conversation-outline-count")).toHaveText("80");
});

test("a late old-history jump cannot undo a newer selection", async ({ page }) => {
  await setup(page);
  await trackInvokes(page, ["load_session"]);
  await page.evaluate(() => { (window as any).historyDelay = 400; });
  const outline = page.getByTestId("conversation-outline");
  await outline.getByRole("button", { name: "question 1", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).historyRequests.includes(43))).toBe(true);
  await outline.getByRole("button", { name: "question 80", exact: true }).click();
  await waitForInvokes(page);
  await expect(page.locator('[data-user-index="79"]')).toHaveClass(/outline-target/);
  await assertOutline(page);
});

test("history jump failures are visible and can be retried", async ({ page }) => {
  await setup(page);
  await trackInvokes(page, ["load_session"]);
  await page.evaluate(() => { (window as any).historyFail = true; });
  await page.getByTestId("conversation-outline").getByRole("button", { name: "question 1", exact: true }).click();
  await expect(page.getByText(/history temporarily unavailable/)).toBeVisible();
  await page.evaluate(() => { (window as any).historyFail = false; });
  await page.getByTestId("conversation-outline").getByRole("button", { name: "question 1", exact: true }).click();
  await expect(page.locator('[data-user-index="0"]')).toContainText("question 1");
  await assertOutline(page);
});
