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
  await page.getByTestId("hook-timeout").fill("10");
  await page.getByTestId("hook-save").click();
  await expect(row).toContainText("Stop");
  await expect(row).toContainText("10s");
  expect(await invoke(page, "get_command_hooks")).toEqual([
    { event: "Stop", matcher: "", command: "./lint.sh", enabled: true, timeout: 10 },
  ]);

  await row.getByTestId("hook-enabled").locator("xpath=..").click();
  await expect(page.getByTestId("hook-form")).toHaveCount(0);
  await expect.poll(async () => (await invoke(page, "get_command_hooks"))[0].enabled).toBe(false);

  await row.getByTestId("hook-remove").click();
  await expect(page.getByTestId("hook-row")).toHaveCount(0);
  expect(await invoke(page, "get_command_hooks")).toEqual([]);
});

test("project hooks are reviewed in full and run only once trusted", async ({ page }) => {
  const command = "python3 .wisp/hooks/deny_destructive_shell.py --refuse rm -rf --refuse 'git push --force'";
  const file = (sha256: string) => ({
    path: "/work/demo/.wisp/hooks.json",
    hooks: [{ event: "PreToolUse", matcher: "shell", command, enabled: true, timeout: 10 }],
    sha256,
    trusted: false,
    error: null,
  });
  await page.goto("/?mockLocale=en");
  await page.evaluate((value) => (window as any).__setMockProjectHooks(value), file("abc"));
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByTestId("settings-nav-hooks").click();

  const project = page.getByTestId("project-hooks");
  const status = page.getByTestId("project-hooks-status");
  await expect(project).toContainText("/work/demo/.wisp/hooks.json");
  await expect(status).toHaveText("Not trusted: these hooks do not run.");
  const row = project.getByTestId("project-hook-row");
  await expect(row).toContainText("PreToolUse");
  await expect(row).toContainText("shell");
  await expect(row).toContainText("10s");
  // Shown in full, not ellipsized: this is what the user is trusting.
  await expect(row.locator(".hooks-command")).toHaveText(command);
  await expect(row.locator(".hooks-command")).toHaveCSS("white-space", "pre-wrap");
  await expect(page.getByTestId("project-hooks-revoke")).toHaveCount(0);

  await page.getByTestId("project-hooks-trust").click();
  await expect(status).toHaveText("Trusted: these hooks run.");
  expect(await invoke(page, "get_project_hooks")).toMatchObject({ sha256: "abc", trusted: true });

  await page.getByTestId("project-hooks-revoke").click();
  await expect(status).toHaveText("Not trusted: these hooks do not run.");

  // The file changed after it was shown: trusting the stale view is refused.
  await page.evaluate((value) => (window as any).__setMockProjectHooks(value), file("def"));
  await page.getByTestId("project-hooks-trust").click();
  await expect(project.getByRole("alert")).toContainText("changed since it was shown");
  expect(await invoke(page, "get_project_hooks")).toMatchObject({ trusted: false });
});

test("hooks page is localized", async ({ page }) => {
  await openHooks(page, "zh");
  await expect(page.getByTestId("settings-nav-hooks")).toHaveText("钩子");
  await expect(page.getByTestId("hook-auto-review")).toContainText("自动审查");
  await expect(page.getByTestId("hook-failure-analysis")).toContainText("自动分析工具失败");
  await expect(page.getByTestId("hook-new")).toContainText("新建钩子");
});
