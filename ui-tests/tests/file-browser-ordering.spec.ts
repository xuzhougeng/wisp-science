// `list_dir` and `search_files` run off the UI thread (#1380), so a slow older
// reply can arrive after a newer one. The Files panel must keep the newest.
import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function openFiles(page: Page) {
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator(".sidebar").getByRole("button", { name: "New session" })).toBeVisible();
  await page.getByRole("button", { name: "Files", exact: true }).click();
  await expect(page.locator('.fb-row[data-workspace-path="report.csv"]')).toBeVisible();
}

/** Hold the matching reply for 800 ms; `__staleReplies` counts released ones. */
async function delayReply(page: Page, cmd: string, key: string, value: string) {
  await page.evaluate(({ cmd, key, value }) => {
    const w = window as any;
    const core = w.__TAURI__.core;
    const original = core.invoke;
    w.__staleReplies = 0;
    core.invoke = async (name: string, args: any) => {
      const arg = args instanceof Map ? args.get(key) : args?.[key];
      if (name === cmd && arg === value) {
        const reply = await original(name, args);
        await new Promise((resolve) => setTimeout(resolve, 800));
        w.__staleReplies += 1;
        return reply;
      }
      return original(name, args);
    };
  }, { cmd, key, value });
}

async function afterStaleReply(page: Page) {
  await expect.poll(() => page.evaluate(() => (window as any).__staleReplies)).toBe(1);
  await page.evaluate(() => new Promise(requestAnimationFrame));
}

test("a slow older file search cannot overwrite newer results", async ({ page }) => {
  await openFiles(page);
  await delayReply(page, "search_files", "query", "report");
  const search = page.locator(".fb-search");
  await search.fill("report");
  await search.fill("counts");
  await expect(page.locator('.fb-row[data-workspace-path="counts.csv"]')).toBeVisible();
  await afterStaleReply(page);
  await expect(page.locator('.fb-row[data-workspace-path="counts.csv"]')).toBeVisible();
  await expect(page.locator('.fb-row[data-workspace-path="data/report.csv"]')).toHaveCount(0);
});

test("a slow older directory listing cannot replace the current folder", async ({ page }) => {
  await openFiles(page);
  await delayReply(page, "list_dir", "path", "DEG");
  await page.locator('.fb-row.dir[data-workspace-path="DEG"]').click();
  await page.locator(".fb-up").click();
  await afterStaleReply(page);
  await expect(page.locator(".fb-path")).toHaveText(".");
  await expect(page.locator('.fb-row[data-workspace-path="report.csv"]')).toBeVisible();
  await expect(page.locator('.fb-row[data-workspace-path="DEG/scripts"]')).toHaveCount(0);
});
