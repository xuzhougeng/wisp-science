import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.use({ timezoneId: "Asia/Shanghai" });
test.beforeEach(async ({ page }) => {
  // Wednesday 2026-09-09 16:00 local.
  await page.clock.setFixedTime(new Date("2026-09-09T08:00:00Z"));
  await page.addInitScript(tauriMock);
});
// Automation is not tied to one project: it opens from the research assistant.
async function openAssistant(page: Page, query = "") {
  await page.goto(`/${query}`);
  await page.getByTestId("open-research-assistant").click();
  await expect(page.getByTestId("assistant-header")).toBeVisible();
}
async function open(page: Page, query = "") {
  await openAssistant(page, query);
  await page.getByTestId("open-automation").click();
  const automation = page.getByTestId("home-automation");
  await expect(automation).toBeVisible();
  return automation;
}

test("Escape closes the task form, then the page, and never a layer below Settings", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".proj-card-main").first()).toBeVisible();
  await expect(page.getByTestId("open-automation")).toHaveCount(0);
  await openAssistant(page);
  await page.getByTestId("open-automation").click();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("home-automation")).toHaveCount(0);
  await expect(page.getByTestId("assistant-header")).toBeVisible();
  await expect(page.locator("#assistant-automation-toggle")).toBeFocused();

  await page.getByTestId("open-automation").click();
  const automation = page.getByTestId("home-automation");
  await automation.getByTestId("automation-create").click();
  await page.keyboard.press("Escape");
  await expect(automation.getByTestId("automation-form")).toHaveCount(0);
  await expect(automation).toBeVisible();

  await automation.getByRole("button", { name: /Change model in Settings/ }).click();
  await expect(page.locator(".settings-page")).toBeVisible();
  await expect(page.locator(".settings-list-title").filter({ hasText: /^Recap$/ })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toHaveCount(0);
  await expect(automation).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(automation).toHaveCount(0);
  await expect(page.getByTestId("assistant-header")).toBeVisible();
});

test("an assistant turn re-reading projects keeps the open task form and its project list", async ({ page }) => {
  await openAssistant(page);
  await expect(page.getByTestId("assistant-projects").locator("[data-project-id]")).toHaveCount(2);
  await page.evaluate(() => { (window as any).__assistantProjectsDelay = 2000; });
  const composer = page.locator(".composer textarea").first();
  await composer.fill("Any news?");
  await composer.press("Enter");
  const loading = page.getByTestId("assistant-projects").getByRole("status");
  await expect(loading).toBeVisible();
  await page.getByTestId("open-automation").click();
  const automation = page.getByTestId("home-automation");
  await automation.locator(".automation-template").filter({ hasText: "Weekly report" }).click();
  const form = automation.getByTestId("automation-form");
  const prompt = form.getByLabel("Prompt");
  await prompt.evaluate(el => { (el as any).__kept = true; });
  await prompt.focus();
  // Checked once, without retrying: the list is there while the re-read is pending.
  expect(await form.getByLabel("Project").locator("option").count()).toBe(2);
  await expect(loading).toHaveCount(0);
  await expect(prompt).toBeFocused();
  expect(await prompt.evaluate(el => (el as any).__kept)).toBe(true);
  await expect(form.getByLabel("Project")).toHaveValue("default");
});

test("the built-in daily recap is on by default; switch, time and run-now persist", async ({ page }) => {
  const automation = await open(page);
  const card = automation.getByTestId("automation-daily-recap");
  await expect(card.getByTestId("daily-recap-enabled")).toBeChecked();
  await expect(card.getByTestId("daily-recap-time")).toHaveValue("09:00");
  await expect(card.getByTestId("daily-recap-status")).toHaveText("Not run yet");
  await card.getByTestId("daily-recap-time").fill("08:30");
  await expect(card.getByTestId("daily-recap-time")).toHaveValue("08:30");
  await card.getByTestId("daily-recap-enabled").uncheck();
  await expect(card).toContainText("Off");
  await automation.getByRole("button", { name: "Refresh automation", exact: true }).click();
  await expect(card.getByTestId("daily-recap-enabled")).not.toBeChecked();
  await expect(card.getByTestId("daily-recap-time")).toHaveValue("08:30");
  await card.getByTestId("daily-recap-run").click();
  await expect(card.getByTestId("daily-recap-status")).toHaveText("Running now…");
  await expect(card.getByTestId("daily-recap-status")).toContainText("2 drafted");
});

test("a template prefills a task listed with its project and cadence; run, pause and delete", async ({ page }) => {
  const automation = await open(page);
  await expect(automation.getByTestId("automation-empty")).toBeVisible();
  await automation.locator(".automation-template").filter({ hasText: "Weekly report" }).click();
  const form = automation.getByTestId("automation-form");
  await expect(form.getByLabel("Name")).toHaveValue("Weekly report");
  await expect(form.getByLabel("Repeat")).toHaveValue("weekly");
  await expect(form.getByLabel("Day")).toHaveValue("5");
  await expect(form.getByLabel("Time")).toHaveValue("17:00");
  await expect(form.getByLabel("Prompt")).toHaveValue(/weekly report/);
  await form.getByLabel("Project").selectOption({ label: "Other project" });
  await form.getByTestId("automation-save").click();
  await expect(form).toHaveCount(0);
  const row = automation.locator(".automation-row");
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("Other project · Fri 17:00 · Next 2026-09-11 17:00");
  await row.getByRole("button", { name: "Run now Weekly report", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__ranSchedule)).toBe("schedule-1");
  await expect(automation.locator(".automation-notice")).toContainText("new session");
  await row.getByRole("switch").uncheck();
  await expect(row).toContainText("Paused");
  const remove = row.getByRole("button", { name: "Delete Weekly report", exact: true });
  await remove.click();
  await expect(row).toHaveCount(1);
  await expect(remove).toHaveAttribute("title", "Click again to delete");
  await remove.click();
  await expect(automation.getByTestId("automation-empty")).toBeVisible();
});

test("a blank task needs a prompt; an hourly task shows its interval", async ({ page }) => {
  const automation = await open(page);
  await automation.getByTestId("automation-create").click();
  const form = automation.getByTestId("automation-form");
  await form.getByTestId("automation-save").click();
  await expect(form.getByRole("alert")).toHaveText("Choose a project and write a prompt.");
  await form.getByLabel("Repeat").selectOption("hourly");
  await form.getByLabel("Every (hours)").fill("6");
  await form.getByLabel("Prompt").fill("Check the cluster queue\nand report stuck jobs");
  await form.getByTestId("automation-save").click();
  const row = automation.locator(".automation-row");
  await expect(row.locator("strong")).toHaveText("Check the cluster queue");
  await expect(row).toContainText("wisp-science · Every 6 h");
});

test("Chinese automation page labels follow the locale", async ({ page }) => {
  const automation = await open(page, "?mockLocale=zh");
  await expect(automation.getByRole("heading", { name: "自动化", exact: true })).toBeVisible();
  await expect(automation.getByTestId("automation-daily-recap")).toContainText("每日研究回顾");
  await expect(automation.getByTestId("automation-daily-recap")).toContainText("内置");
  await expect(automation.locator(".automation-template")).toHaveCount(3);
  await expect(automation.locator(".automation-template").first()).toContainText("文献追踪");
  await expect(automation.locator(".automation-template").first()).toContainText("周一 09:00");
});
