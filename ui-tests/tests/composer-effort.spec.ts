import { expect, test, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

// mockSessionModels=1 gives the default profile (deepseek-v4-pro) effort
// "low" and the opus profile effort "max".
async function enterApp(page: Page) {
  await page.goto("/?mockSessionModels=1");
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
  await expect(pill.getByTestId("composer-effort-value")).toHaveText("Low");

  await pill.click();
  const menu = page.getByTestId("composer-effort-menu");
  await expect(menu).toBeVisible();
  await expect(menu.locator(".composer-effort-option")).toHaveText([
    "Default",
    "Low",
    "High",
    "Max",
  ]);
  await expect(menu.locator('[data-effort="low"] .composer-effort-check')).toBeVisible();

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
  await menu.locator('[data-effort="high"]').click();
  await expect(menu).toHaveCount(0);
  await expect(pill.getByTestId("composer-effort-value")).toHaveText("High");
  await expect.poll(() => lastInvokeArgs(page, "save_model"))
    .toMatchObject({ profile: { reasoning_effort: "high" } });
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
  await page.goto("/?mockSessionModels=1");
  await page.locator(".proj-card-main").first().click();
  await page.locator('[data-session-id="s-model-a"]').click();
  await expect(page.locator(".model-picker-label")).toHaveText("deepseek-v4-pro");
  await expect(page.getByTestId("composer-effort-value")).toHaveText("Low");

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
