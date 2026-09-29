import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

async function openHistory(page: Page, session = "s1") {
  await expect.poll(() => page.evaluate(() => Boolean((window as any).__tauriListenerReady?.("open-session")))).toBe(true);
  await page.evaluate(sessionId => (window as any).__tauriEmit("open-session", { projectId: "other", sessionId }), session);
  await expect(page.locator(".msg.assistant")).toContainText("Annotation directory");
}

async function calls(page: Page, command: string) {
  return page.evaluate(cmd => ((window as any).__skillInvokeLog ?? []).filter((call: any) => call.cmd === cmd)
    .map((call: any) => call.args instanceof Map ? Object.fromEntries(call.args) : call.args), command);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto("/?mockPathTypes=1");
});

test("historical directory links open the exact folder and own a directory menu across reloads", async ({ page }) => {
  for (const reload of [false, true]) {
    if (reload) await page.reload();
    await openHistory(page);
    const reply = page.locator(".msg.assistant");
    const directory = reply.getByRole("link", { name: "Annotation directory", exact: true });
    await expect(directory).toHaveAttribute("data-workspace-kind", "directory");
    await expect(reply.locator('[data-resource-status="unresolved"]')).toHaveCount(0);
    await directory.click();
    await expect.poll(() => calls(page, "list_dir")).toContainEqual({ path: "docs/07.celltype_auto_annotation" });
    await expect(page.locator(".artifact-modal")).toHaveCount(0);
    await expect(page.locator(".center-file-preview")).toHaveCount(0);
    expect((await calls(page, "read_file")).some(arg => arg.path === "docs/07.celltype_auto_annotation")).toBe(false);

    await directory.click({ button: "right" });
    const menu = page.locator(".ctx-menu");
    await expect(menu.getByRole("button", { name: "Open in Files", exact: true })).toBeVisible();
    await expect(menu.getByRole("button", { name: "Open in center" })).toHaveCount(0);
    if (!reload) await page.screenshot({ path: test.info().outputPath("directory-menu.png") });
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
    await directory.click({ button: "right" });
    await menu.getByRole("button", { name: "Open in file manager", exact: true }).click();
    await expect.poll(() => calls(page, "open_workspace_path")).toContainEqual({ path: "docs/07.celltype_auto_annotation" });
    await reply.getByRole("link", { name: "spaced directory" }).click();
    await expect.poll(() => calls(page, "list_dir")).toContainEqual({ path: "my data" });

    await expect(reply.locator("code").filter({ hasText: "Missing path" })).toBeVisible();
    await expect(reply.locator("code").filter({ hasText: "Unverified path" })).toBeVisible();
    await expect(reply.getByRole("link", { name: /Missing path|Unverified path/ })).toHaveCount(0);
    await reply.getByRole("link", { name: "Existing file" }).click();
    await expect(page.locator('.artifact-modal .am-figure[data-file-path="notes/FIGURE_LEGEND.md"]')).toBeVisible();
    await page.keyboard.press("Escape");
    await reply.getByRole("link", { name: "Saved report" }).click();
    await expect(page.locator('.artifact-modal .am-figure[data-file-path="artifact-version:resource-version-markdown"]')).toBeVisible();
    await page.keyboard.press("Escape");
  }
});

test("paths stay plain while classification is pending or fails", async ({ page }) => {
  await page.evaluate(() => {
    const w = window as any;
    w.__pathClassificationFailure = true;
    w.__pathClassificationGate = new Promise(resolve => { w.__releasePathClassification = resolve; });
  });
  await openHistory(page);
  const reply = page.locator(".msg.assistant");
  await expect(reply.locator("code").filter({ hasText: "Annotation directory" })).toBeVisible();
  await expect(reply.locator("a[data-workspace-path]")).toHaveCount(0);
  await expect(reply.getByRole("link", { name: "Saved report" })).toBeVisible();
  await page.evaluate(() => {
    (window as any).__releasePathClassification();
  });
  await expect.poll(() => page.evaluate(() => (window as any).__pathClassificationsFailed ?? 0)).toBeGreaterThan(0);
  await expect(reply.locator("a[data-workspace-path]")).toHaveCount(0);
  await expect(reply.locator("code").filter({ hasText: "Annotation directory" })).toBeVisible();
});

test("a delayed previous session result cannot reactivate paths in another session", async ({ page }) => {
  await page.evaluate(() => {
    const w = window as any;
    w.__pathClassificationGate = new Promise(resolve => { w.__releaseOldPaths = resolve; });
  });
  await openHistory(page);
  await expect.poll(() => calls(page, "classify_workspace_paths")).not.toHaveLength(0);
  await page.evaluate(() => {
    (window as any).__pathClassificationGate = null;
    (window as any).__pathClassificationFailure = true;
  });
  await openHistory(page, "s2");
  await expect.poll(() => page.evaluate(() => (window as any).__pathClassificationsFailed ?? 0)).toBeGreaterThan(0);
  const settled = await page.evaluate(() => (window as any).__pathClassificationsSettled ?? 0);
  await page.evaluate(() => {
    (window as any).__releaseOldPaths();
  });
  await expect.poll(() => page.evaluate(() => (window as any).__pathClassificationsSettled ?? 0)).toBeGreaterThan(settled);
  await expect(page.locator('.msg.assistant a[data-workspace-path]')).toHaveCount(0);
});
