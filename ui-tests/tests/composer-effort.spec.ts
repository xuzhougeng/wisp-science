import { expect, test, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

// Catalog fixtures bind exact model IDs: DeepSeek supports high/max,
// while the Opus profile uses claude-opus-4-8 and supports five levels.
async function enterApp(page: Page) {
  await page.goto("/?mockSessionModels=1&mockComposerCatalog=1");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator("#composer-input")).toBeVisible();
}

function lastInvokeArgs(page: Page, cmd: string) {
  return page.evaluate((name) => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      return value;
    };
    const calls = ((window as any).__skillInvokeLog ?? []).filter((call: any) => call.cmd === name);
    return plain(calls.at(-1)?.args ?? null);
  }, cmd);
}

test("composer footer lays out context, model, effort, fast, send", async ({ page }) => {
  await enterApp(page);
  // The context gauge only renders once a session reports a usage snapshot.
  await page.locator("#composer-input").fill("CONTEXTUSAGE");
  await page.locator("button.send").click();
  const bar = page.locator(".composer-buttons");
  await expect(bar.getByTestId("context-usage-trigger")).toBeVisible({ timeout: 10_000 });
  await expect(bar.locator(".model-picker-btn")).toBeVisible();
  await expect(bar.getByTestId("composer-effort-trigger")).toBeVisible();
  await expect(bar.getByTestId("composer-fast-toggle")).toBeVisible();
  await expect(bar.locator("button.send")).toBeVisible();
  // The send dropdown (and its chevron button) is gone entirely.
  await expect(page.locator(".send-menu-toggle")).toHaveCount(0);

  const order = await bar.evaluate((element) => {
    const mark = (el: Element): string => {
      if (el.classList.contains("context-usage-trigger")) return "context";
      if (el.classList.contains("model-picker")) return "model";
      if (el.classList.contains("composer-effort")) return "effort";
      if (el.classList.contains("composer-fast")) return "fast";
      if (el.classList.contains("send")) return "send";
      return "";
    };
    return Array.from(element.children)
      .map(mark)
      .filter((name) => name !== "");
  });
  expect(order).toEqual(["context", "model", "effort", "fast", "send"]);
});

test("the effort pill shows the selected model's effort and opens its dropdown", async ({ page }) => {
  await enterApp(page);
  const pill = page.getByTestId("composer-effort-trigger");
  await expect(pill.getByTestId("composer-effort-value")).toHaveText("High");

  await pill.click();
  const menu = page.getByTestId("composer-effort-menu");
  await expect(menu).toBeVisible();
  await expect(menu.locator(".composer-effort-option")).toHaveText([
    "Default",
    "High",
    "Max",
  ]);
  await expect(menu.locator('[data-effort="high"] .composer-effort-check')).toBeVisible();

  // Escape closes only the dropdown; the pill button stays.
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(pill).toBeVisible();
  await expect(page.locator(".model-menu")).toHaveCount(0);
});

test("picking an effort persists it on the model profile and updates the pill", async ({ page }) => {
  await enterApp(page);
  const pill = page.getByTestId("composer-effort-trigger");
  await pill.click();
  const menu = page.getByTestId("composer-effort-menu");
  await menu.locator('[data-effort="max"]').click();
  await expect(menu).toHaveCount(0);
  await expect(pill.getByTestId("composer-effort-value")).toHaveText("Max");
  await expect.poll(() => lastInvokeArgs(page, "save_model"))
    .toMatchObject({ profile: { reasoning_effort: "max" } });
  await expect.poll(() => lastInvokeArgs(page, "set_session_reasoning_effort")).toBeNull();

  await pill.click();
  await menu.locator('[data-effort="default"]').click();
  await expect(pill.getByTestId("composer-effort-value")).toHaveText("Default");
  await expect.poll(() => lastInvokeArgs(page, "save_model"))
    .toMatchObject({ profile: { reasoning_effort: "" } });
});

test("model rows keep only useful identity information and no effort editor", async ({ page }, testInfo) => {
  await page.goto("/?mockSessionModels=1&mockComposerCatalog=1&mockComposerMenus=1");
  await page.locator(".proj-card-main").first().click();
  await page.locator(".model-picker-btn").click();
  const menu = page.locator(".model-menu");
  const active = menu.locator(".model-menu-row.active");
  await expect(active.locator(".model-menu-label")).toHaveText("DEEPSEEK-V4-PRO");
  await expect(active.locator(".model-menu-sub")).toHaveCount(0);
  await expect(active.locator(".model-menu-pick")).toHaveAttribute("title", /openai · deepseek-v4-pro/);
  await expect(active.locator(".model-menu-check svg")).toHaveCount(1);
  await active.hover();
  await expect(menu.locator('[class*="model-menu-effort"]')).toHaveCount(0);
  await expect(menu.locator(".model-menu-row", { hasText: /^Shared model/ }).locator(".model-menu-sub"))
    .toHaveText("anthropic · opus-4.8");
  await expect(menu.locator(".model-menu-row", { hasText: /^shared model/ }).locator(".model-menu-sub"))
    .toHaveText("openai · gpt-5.5");
  const longName = menu.locator(".model-menu-label", { hasText: "A very long" });
  expect(await longName.evaluate(el => el.scrollWidth > el.clientWidth)).toBe(true);
  await expect(longName.locator("xpath=../..")).toHaveAttribute("title", /custom-reasoner/);
  await page.screenshot({ path: testInfo.outputPath("model-menu.png"), animations: "disabled" });
});

test("Chinese effort menu uses one concise localized set of labels", async ({ page }, testInfo) => {
  await page.goto("/?mockLocale=zh&mockComposerModel=test-reasoner&mockComposerEffort=ultra");
  await page.locator(".proj-card-main").first().click();
  await page.getByTestId("composer-effort-trigger").click();
  const menu = page.getByTestId("composer-effort-menu");
  await expect(menu.locator(".composer-effort-menu-label")).toHaveText("推理强度");
  await expect(menu.locator(".composer-effort-option")).toHaveText([
    "默认", "无", "极低", "低", "中", "高", "极高", "最高", "超强",
  ]);
  await expect(menu.locator('[data-effort="ultra"] .composer-effort-check')).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("effort-menu.png"), animations: "disabled" });
});

for (const model of ["test-no-effort", "gpt-5.6-luna-new", "unknown-model"]) {
  test(`unconfirmed effort values are never offered for ${model}`, async ({ page }) => {
    await page.goto(`/?mockComposerModel=${model}&mockComposerEffort=ultra`);
    await page.locator(".proj-card-main").first().click();
    await page.getByTestId("composer-effort-trigger").click();
    const menu = page.getByTestId("composer-effort-menu");
    await expect(menu.locator(".composer-effort-option")).toHaveText(["Default"]);
    await expect(menu.locator(".composer-effort-hint")).toContainText("not confirmed");
    await menu.locator('[data-effort="default"]').click();
    await expect.poll(() => lastInvokeArgs(page, "save_model"))
      .toMatchObject({ profile: { reasoning_effort: "" } });
  });
}

test("settings uses exact catalog support and ignores a late previous-model lookup", async ({ page }) => {
  await page.goto("/?mockSlowCatalog=1");
  await page.locator(".proj-card-main").first().click();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await page.locator(".settings-list-row").first().click();
  const effort = page.getByRole("combobox", { name: "Reasoning effort", exact: true });
  await expect(effort.locator("option:not([disabled])")).toHaveText(["Default (provider)", "High", "Max"]);
  const model = page.getByRole("textbox", { name: "Model ID", exact: true });
  await model.fill("gpt-5.6-luna");
  await expect.poll(() => lastInvokeArgs(page, "model_catalog_lookup")).toMatchObject({ model: "gpt-5.6-luna" });
  await model.fill("gpt-5.6-luna-new");
  await expect.poll(() => page.evaluate(() => (window as any).__slowCatalogCompleted)).toBe(true);
  await expect(effort.locator("option:not([disabled])")).toHaveText(["Default (provider)"]);
  await model.fill("openai/gpt-5.6-luna");
  await expect(effort.locator("option:not([disabled])")).toHaveText([
    "Default (provider)", "None", "Low", "Medium", "High", "Extra high",
  ]);
});

test("Escape immediately dismisses each composer menu and keeps the conversation open", async ({ page }) => {
  await enterApp(page);
  for (const trigger of [page.locator(".model-picker-btn"), page.getByTestId("composer-effort-trigger")]) {
    await trigger.click();
    await page.keyboard.press("Escape");
    await expect(page.locator(".composer-select-menu")).toHaveCount(0);
    await expect(page.locator("#composer-input")).toBeVisible();
  }
});

test("composer menus fit a short narrow window at large text size and share styling", async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 440 });
  await page.goto("/?mockComposerMenus=1&mockComposerModel=test-reasoner");
  await page.locator(".proj-card-main").first().click();
  await page.evaluate(() => document.documentElement.style.setProperty("--ui-font-scale", "1.5"));
  const styles: unknown[] = [];
  for (const [trigger, menuSelector, rowSelector] of [
    [".model-picker-btn", ".model-menu", ".model-menu-pick"],
    ['[data-testid="composer-effort-trigger"]', ".composer-effort-menu", ".composer-effort-option"],
  ]) {
    await page.locator(trigger).click();
    const menu = page.locator(menuSelector);
    await expect(menu).toBeVisible();
    await expect(menu).toHaveAttribute("style", /--composer-menu-height:[0-9.]+px/);
    const box = (await menu.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(7);
    expect(box.y).toBeGreaterThanOrEqual(7);
    expect(box.x + box.width).toBeLessThanOrEqual(713);
    expect(box.y + box.height).toBeLessThanOrEqual(433);
    expect(await menu.evaluate(el => el.scrollHeight > el.clientHeight)).toBe(true);
    styles.push(await menu.evaluate((el, selector) => {
      const surface = getComputedStyle(el);
      const row = getComputedStyle(el.querySelector(selector)!);
      return [surface.borderRadius, surface.boxShadow, surface.padding, row.fontFamily, row.fontSize, row.padding];
    }, rowSelector));
    await menu.locator("button").last().scrollIntoViewIfNeeded();
    await expect(menu.locator("button").last()).toBeInViewport();
    await page.keyboard.press("Escape");
  }
  expect(styles[0]).toEqual(styles[1]);
  await page.getByTestId("composer-effort-trigger").click();
  await page.setViewportSize({ width: 700, height: 420 });
  await expect(page.locator(".composer-select-menu")).toHaveCount(0);
});

test("the model picker and the effort dropdown never open together", async ({ page }) => {
  await enterApp(page);
  await page.locator(".model-picker-btn").click();
  await expect(page.locator(".model-menu")).toBeVisible();
  // Every menu sits on a full-window backdrop, so clicking elsewhere (the
  // realistic gesture) dismisses the model menu before the pill can open.
  await page.locator(".model-menu-backdrop").click();
  await expect(page.locator(".model-menu")).toHaveCount(0);

  await page.getByTestId("composer-effort-trigger").click();
  await expect(page.getByTestId("composer-effort-menu")).toBeVisible();
  await expect(page.locator(".model-menu")).toHaveCount(0);
  // Escape closes the topmost layer (the effort dropdown) in one press.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("composer-effort-menu")).toHaveCount(0);
  await expect(page.locator(".composer-effort")).toBeVisible();
});

test("the effort pill follows the session's model binding", async ({ page }) => {
  await page.goto("/?mockSessionModels=1&mockComposerCatalog=1");
  await page.locator(".proj-card-main").first().click();
  await page.locator('[data-session-id="s-model-a"]').click();
  await expect(page.locator(".model-picker-label")).toHaveText("deepseek-v4-pro");
  await expect(page.getByTestId("composer-effort-value")).toHaveText("High");

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /opus-4\.8/ }).click();
  await page.getByTestId("model-switch-confirm")
    .getByRole("button", { name: "Yes, switch" }).click();
  await expect(page.locator(".model-picker-label")).toHaveText("opus-4.8");
  await expect(page.getByTestId("composer-effort-value")).toHaveText("Max");

  await page.getByTestId("composer-effort-trigger").click();
  const menu = page.getByTestId("composer-effort-menu");
  await expect(menu.locator(".composer-effort-option")).toHaveText([
    "Default",
    "Low",
    "Medium",
    "High",
    "Extra high",
    "Max",
  ]);
});
