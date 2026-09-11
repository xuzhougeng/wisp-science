import { test, expect, type Page, type Locator } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function open(page: Page, label: string, locale = "en") {
  await page.goto(`/?mockLocale=${locale}`);
  await page.getByRole("button", { name: locale === "zh" ? "设置" : "Settings", exact: true }).click();
  await page.locator(".settings-nav").getByRole("button", { name: label, exact: true }).click();
}

async function fits(locator: Locator) {
  expect(await locator.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
}

for (const locale of ["en", "zh"]) {
  const zh = locale === "zh";
  test(`settings navigation groups and searches without changing the open page (${locale})`, async ({ page }) => {
    await open(page, zh ? "模型" : "Models", locale);
    const nav = page.locator(".settings-nav");
    await expect(nav.locator(".settings-nav-label")).toHaveText(zh
      ? ["基础偏好", "AI 配置", "工具与连接", "系统与资源"]
      : ["Preferences", "AI configuration", "Tools & connections", "System & resources"]);
    const routes = [
      ["general", "session", "appearance", "pet"],
      ["models", "quick-actions", "workflows", "specialists", "memory"],
      ["skills", "plugins", "browser", "connections", "channels"],
      ["credentials", "permissions", "environments", "storage", "usage"],
    ];
    for (const [i, sections] of routes.entries()) {
      expect(await nav.getByRole("group").nth(i).locator("button").evaluateAll(
        buttons => buttons.map(button => button.getAttribute("data-testid")),
      )).toEqual(sections.map(section => `settings-nav-${section}`));
    }
    const search = nav.getByRole("searchbox");
    await search.fill("  API KEY  ");
    await expect(nav.getByRole("group").locator("button")).toHaveText(zh ? ["模型", "凭据"] : ["Models", "Credentials"]);
    await expect(page.locator(".model-settings-pane")).toBeVisible();
    await search.fill("no-such-setting");
    await expect(nav.getByRole("status")).toBeVisible();
    await expect(nav.getByRole("group")).toHaveCount(0);
    await expect(page.locator(".model-settings-pane")).toBeVisible();
    // Both languages and aliases work regardless of the display locale.
    await search.fill(zh ? "font" : "字体");
    await nav.getByTestId("settings-nav-appearance").click();
    await expect(page.getByTestId("appearance-live-preview")).toBeVisible();
    await expect(nav.getByTestId("settings-nav-appearance")).toHaveAttribute("aria-current", "page");
    await search.fill("");
    await expect(nav.getByRole("group").locator("button")).toHaveCount(19);
    await nav.getByTestId("settings-nav-models").click();
    await page.locator(".model-settings-pane .settings-list-row", { hasText: "opus-4.8" }).click();
    const name = page.getByLabel(zh ? "显示名称（别名）" : "Display name", { exact: true });
    await name.fill("Unsaved profile name");
    await search.fill("no-such-setting");
    await expect(name).toHaveValue("Unsaved profile name");
    await search.fill("");
    await expect(name).toHaveValue("Unsaved profile name");
    await page.keyboard.press("Escape");
    await expect(page.locator(".model-settings-pane")).toBeVisible();
    await page.setViewportSize({ width: 820, height: 580 });
    await nav.getByTestId("settings-nav-usage").click();
    await expect(page.locator(".settings-head")).toContainText(zh ? "用量" : "Usage");
    await fits(nav);
  });

  test(`model defaults save immediately and the list reflows (${locale})`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await open(page, zh ? "模型" : "Models", locale);
    const pane = page.locator(".model-settings-pane");
    await expect(page.getByTestId("models-category-http")).toHaveText(zh ? "API 模型 (2)" : "API models (2)");
    await expect(pane.locator(".settings-cap-badge")).toHaveText(zh ? "视觉" : "Vision");
    await expect(pane.locator(".settings-footer")).toHaveCount(0);
    await expect(page.getByTestId("acp-models-list-hint")).toContainText(zh ? "即时保存" : "saved immediately");
    const opus = pane.locator(".settings-list-row", { hasText: "opus-4.8" });
    await opus.getByRole("button", { name: zh ? "设为默认模型" : "Set as default", exact: true }).click();
    await expect(opus.locator(".settings-model-default")).toHaveText(zh ? "当前默认" : "Default");
    await expect(pane.locator(".settings-model-default")).toHaveCount(1);
    await page.locator(".settings-nav").getByRole("button", { name: zh ? "返回应用" : "Back to app" }).click();
    await page.getByRole("button", { name: zh ? "设置" : "Settings", exact: true }).click();
    await page.getByTestId("settings-nav-models").click();
    await expect(opus.locator(".settings-model-default")).toBeVisible();
    expect((await pane.boundingBox())!.width).toBeLessThanOrEqual(920);
    // The heading and tab strip share the same content edge.
    const head = await page.locator(".settings-head-main").boundingBox();
    const tabs = await pane.locator(".settings-category-tabs").boundingBox();
    expect(Math.abs(head!.x - tabs!.x)).toBeLessThan(2);
    await page.screenshot({ path: testInfo.outputPath(`models-${locale}.png`), animations: "disabled" });
    for (const width of [820, 600, 390]) {
      await page.setViewportSize({ width, height: 740 });
      await fits(page.locator(".settings-content"));
      await fits(pane);
      await fits(opus);
      const badge = pane.locator(".settings-cap-badge");
      const singleLine = await badge.evaluate(el => el.getBoundingClientRect().height < 24);
      expect(singleLine).toBe(true);
    }
    await page.screenshot({ path: testInfo.outputPath(`models-narrow-${locale}.png`), animations: "disabled" });
    await opus.click();
    await expect(page.locator(".settings-footer").getByRole("button", { name: zh ? "保存" : "Save", exact: true })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(pane).toBeVisible();
    await expect(page.locator(".settings-page")).toBeVisible();
  });

  test(`settings cards reflow and keep controls reachable (${locale})`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await open(page, zh ? "外观" : "Appearance", locale);
    const preview = page.getByTestId("appearance-live-preview");
    const controls = page.locator(".appearance-controls");
    expect((await preview.boundingBox())!.x).toBeGreaterThan((await controls.boundingBox())!.x);
    await expect(page.getByTestId("appearance-custom-css")).toBeHidden();
    await page.screenshot({ path: testInfo.outputPath(`appearance-${locale}.png`), animations: "disabled" });
    await page.setViewportSize({ width: 820, height: 740 });
    expect((await preview.boundingBox())!.y).toBeGreaterThan((await controls.boundingBox())!.y);
    for (const field of ["appearance-ui-font", "appearance-code-font"]) {
      await page.getByTestId(field).scrollIntoViewIfNeeded();
      await fits(page.locator(".settings-appearance-pane"));
    }
    await page.getByTestId("appearance-custom-css-summary").click();
    await expect(page.getByTestId("appearance-custom-css")).toBeVisible();

    for (const [label, first, second, filename] of [
      [zh ? "记忆" : "Memory", "memory-project-card", "memory-global-card", "memory"],
      [zh ? "远程接入" : "Remote Access", "project-sync-card", "channels-overview", "remote"],
    ]) {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.locator(".settings-nav").getByRole("button", { name: label, exact: true }).click();
      await expect(page.getByTestId(first)).toBeVisible();
      const a = (await page.getByTestId(first).boundingBox())!;
      const b = (await page.getByTestId(second).boundingBox())!;
      expect(Math.abs(a.y - b.y)).toBeLessThan(2);
      expect(b.x).toBeGreaterThan(a.x);
      await page.screenshot({ path: testInfo.outputPath(`${filename}-${locale}.png`), animations: "disabled" });
      await page.setViewportSize({ width: 820, height: 740 });
      const narrowA = (await page.getByTestId(first).boundingBox())!;
      const narrowB = (await page.getByTestId(second).boundingBox())!;
      expect(narrowB.y).toBeGreaterThanOrEqual(narrowA.y + narrowA.height);
      await fits(page.locator(".settings-content"));
    }
    const sync = page.getByTestId("project-sync-card");
    await sync.getByRole("button", { name: zh ? "保存" : "Save", exact: true }).scrollIntoViewIfNeeded();
    await fits(page.getByTestId("remote-settings-pane"));
    await page.getByTestId("feishu-channel-row").click();
    await expect(page.getByTestId("project-sync-card")).toHaveCount(0);
    await expect(page.getByTestId("feishu-channel-card")).toBeVisible();
    // One immediate Escape closes only the channel detail, retaining Settings.
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("channels-overview")).toBeVisible();
    await expect(page.getByTestId("project-sync-card")).toBeVisible();
  });
}

test("conversation groups and browser lists use content height", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await open(page, "对话", "zh");
  await expect(page.locator(".session-preferences > h3")).toHaveText(["运行限制", "上下文管理", "后续交互"]);
  expect((await page.getByTestId("max-iter").boundingBox())!.width).toBeLessThan(150);
  await page.screenshot({ path: testInfo.outputPath("session-zh.png"), animations: "disabled" });
  await page.locator(".settings-nav").getByRole("button", { name: "浏览器", exact: true }).click();
  const cards = page.locator(".browser-filter-card");
  const first = (await cards.nth(0).boundingBox())!;
  const second = (await cards.nth(1).boundingBox())!;
  expect(Math.abs(first.y - second.y)).toBeLessThan(2);
  expect(first.height).toBeLessThan(300);
  await page.setViewportSize({ width: 820, height: 740 });
  const block = (await cards.nth(0).boundingBox())!;
  const prefer = (await cards.nth(1).boundingBox())!;
  expect(prefer.y - block.y - block.height).toBeLessThan(40);
  await fits(page.getByTestId("browser-url-filters"));
});

test("a failed default model change preserves the selection and can be retried", async ({ page }) => {
  await open(page, "Models");
  const pane = page.locator(".model-settings-pane");
  const opus = pane.locator(".settings-list-row", { hasText: "opus-4.8" });
  const previous = pane.locator(".settings-list-row", { hasText: "deepseek-v4-pro" });
  await page.evaluate(() => { (window as any).__failSetActiveModel = true; });
  await opus.getByRole("button", { name: "Set as default", exact: true }).click();
  await expect(pane.locator(".settings-status.fail")).toContainText("Could not save default model");
  await expect(previous.locator(".settings-model-default")).toBeVisible();
  await expect(opus.locator(".settings-model-default")).toHaveCount(0);
  await page.evaluate(() => { (window as any).__failSetActiveModel = false; });
  await opus.getByRole("button", { name: "Set as default", exact: true }).click();
  await expect(opus.locator(".settings-model-default")).toBeVisible();
  await expect(pane.locator(".settings-status.fail")).toHaveCount(0);
});
