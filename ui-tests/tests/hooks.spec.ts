import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function openHooks(page: Page, locale = "en") {
  await page.goto(`/?mockLocale=${locale}`);
  await page.getByRole("button", { name: locale === "zh" ? "设置" : "Settings", exact: true }).click();
  await page.getByTestId("settings-nav-hooks").click();
  await expect(page.getByTestId("hooks-pane")).toBeVisible();
}

const invoke = (page: Page, cmd: string, args: object = {}) =>
  page.evaluate(([cmd, args]) => (window as any).__TAURI__.core.invoke(cmd, args), [cmd, args] as const);

test("hooks page edits the two default hooks", async ({ page }) => {
  await openHooks(page);
  const review = page.getByTestId("hook-auto-review-toggle");
  await expect(review).not.toBeChecked();
  await review.locator("xpath=..").click();
  await expect(review).toBeChecked();
  // No session id: the default new conversations inherit.
  expect(await invoke(page, "get_auto_review_enabled")).toBe(true);

  await expect(page.getByTestId("hook-failure-rate")).toHaveCount(0);
  await page.getByTestId("hook-failure-analysis-toggle").locator("xpath=..").click();
  await page.getByTestId("hook-failure-rate").fill("50");
  await page.getByTestId("hook-failure-rate").blur();
  await expect.poll(() => invoke(page, "get_auto_failure_analysis_settings")).toMatchObject({
    enabled: true,
    failure_rate_threshold: 50,
  });

  await page.getByTestId("hook-reviewer-model").click();
  await expect(page.getByTestId("reviewer-backend-select")).toBeVisible();
});

test("command hooks can be created, edited, toggled and removed", async ({ page }) => {
  await openHooks(page);
  await expect(page.getByTestId("hook-row")).toHaveCount(0);

  await page.getByTestId("hook-new").click();
  await page.getByTestId("hook-matcher").fill("(");
  await page.getByTestId("hook-command").fill("./check.sh");
  await page.getByTestId("hook-save").click();
  await expect(page.getByRole("alert")).toContainText("Invalid tool matcher");
  await expect(page.getByTestId("hook-form")).toBeVisible();

  await page.getByTestId("hook-matcher").fill("shell|write");
  await page.getByTestId("hook-save").click();
  await expect(page.getByTestId("hook-form")).toHaveCount(0);
  const row = page.getByTestId("hook-row");
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("PreToolUse");
  await expect(row).toContainText("shell|write");
  await expect(row.locator(".hooks-command")).toHaveText("./check.sh");

  // Typing keeps focus: the form is not rebuilt on every keystroke.
  await row.click();
  const command = page.getByTestId("hook-command");
  await command.fill("");
  await command.pressSequentially("./lint.sh");
  await expect(command).toHaveValue("./lint.sh");
  // Non-tool events have no matcher.
  await page.getByTestId("hook-event").selectOption("Stop");
  await expect(page.getByTestId("hook-matcher")).toHaveCount(0);
  await expect(command).toHaveValue("./lint.sh");
  await page.getByTestId("hook-save").click();
  await expect(row).toContainText("Stop");
  expect(await invoke(page, "get_command_hooks")).toEqual([
    { event: "Stop", matcher: "", command: "./lint.sh", enabled: true },
  ]);

  await row.getByTestId("hook-enabled").locator("xpath=..").click();
  await expect(page.getByTestId("hook-form")).toHaveCount(0);
  await expect.poll(async () => (await invoke(page, "get_command_hooks"))[0].enabled).toBe(false);

  await row.getByTestId("hook-remove").click();
  await expect(page.getByTestId("hook-row")).toHaveCount(0);
  expect(await invoke(page, "get_command_hooks")).toEqual([]);
});

test("hooks page is localized", async ({ page }) => {
  await openHooks(page, "zh");
  await expect(page.getByTestId("settings-nav-hooks")).toHaveText("钩子");
  await expect(page.getByTestId("hook-auto-review")).toContainText("自动审查");
  await expect(page.getByTestId("hook-failure-analysis")).toContainText("自动分析工具失败");
  await expect(page.getByTestId("hook-new")).toContainText("新建钩子");
});
