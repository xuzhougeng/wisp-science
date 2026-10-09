import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.use({ timezoneId: "Asia/Shanghai" });
test.beforeEach(async ({ page }) => {
  // Wednesday 2026-09-09 16:00 local.
  await page.clock.setFixedTime(new Date("2026-09-09T08:00:00Z"));
  await page.addInitScript(tauriMock);
});
// Mandates live on the Automation page, which opens from the research assistant.
async function open(page: Page, query = "") {
  await page.goto(`/${query}`);
  await page.getByTestId("open-research-assistant").click();
  await expect(page.getByTestId("assistant-header")).toBeVisible();
  await page.getByTestId("open-automation").click();
  const automation = page.getByTestId("home-automation");
  await expect(automation).toBeVisible();
  return automation;
}

test("a template becomes a mandate with its goal, KPI and constraints; run, pause, edit and delete", async ({ page }) => {
  const automation = await open(page);
  const section = automation.getByTestId("mandate-section");
  await expect(section.getByTestId("mandate-empty")).toBeVisible();
  await expect(section.locator(".mandate-template")).toHaveCount(4);
  await section.locator(".mandate-template").filter({ hasText: "Literature watch" }).click();
  const form = automation.getByTestId("mandate-form");
  await expect(form.getByLabel("Name", { exact: true })).toHaveValue("Literature watch");
  await expect(form.getByLabel("Goal", { exact: true })).toHaveValue(/keep this project's literature current/);
  await expect(form.getByLabel("Until", { exact: true })).toHaveValue("2027-09-09");
  await expect(form.getByTestId("mandate-kpi-row")).toHaveCount(1);
  await expect(form.getByLabel("KPI name")).toHaveValue("Papers screened and filed");
  await expect(form.getByLabel("Target", { exact: true })).toHaveValue("5");
  await expect(form.getByLabel("May do on its own")).toHaveValue(/Search literature databases/);
  await expect(form.getByTestId("mandate-review-mutations")).not.toBeChecked();
  await expect(form.getByLabel("Check in every (hours)")).toHaveValue("24");
  await form.getByLabel("Project").selectOption({ label: "Other project" });
  // Typing in a KPI cell keeps the cell: the rows are not rebuilt per keystroke.
  await form.getByLabel("Target", { exact: true }).fill("");
  await form.getByLabel("Target", { exact: true }).pressSequentially("12");
  await expect(form.getByLabel("Target", { exact: true })).toBeFocused();
  await form.getByTestId("mandate-save").click();
  await expect(form).toHaveCount(0);

  const draft = await page.evaluate(() => (window as any).__mandateDraft);
  expect(draft.project_id).toBe("other");
  expect(draft.kpis[0]).toMatchObject({ name: "Papers screened and filed", target: "12", period: "per week" });
  expect(draft.constraints).toMatchObject({ review_mutations: false, min_interval_secs: 900, max_interval_secs: 604800, max_rounds_per_day: 6 });
  expect(draft.interval_secs).toBe(86400);
  expect(draft.report_interval_secs).toBe(604800);
  // The first round starts now, and the period ends at the local end of that day.
  expect(draft.start_at).toBe(Math.floor(new Date("2026-09-09T08:00:00Z").getTime() / 1000));
  expect(draft.ends_at).toBe(Math.floor(new Date("2027-09-09T15:59:59Z").getTime() / 1000));

  const card = section.locator(".mandate-card");
  await expect(card).toHaveCount(1);
  await expect(card).toHaveAttribute("data-status", "active");
  await expect(card.locator(".mandate-status")).toHaveText("Active");
  await expect(card).toContainText("Other project · Next round 2026-09-09 16:00 · until 2027-09-09");
  await expect(card.locator(".mandate-kpis li")).toContainText("– / 12");
  await card.getByRole("button", { name: "Run a round now Literature watch", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__ranMandate)).toBe("mandate-1");
  await expect(automation.locator(".automation-notice")).toContainText("mandate's conversation");

  await card.getByRole("switch").uncheck();
  await expect(card).toHaveAttribute("data-status", "paused");
  await expect(card).toContainText("Other project · Paused");
  await card.getByRole("switch").check();
  await expect(card.locator(".mandate-status")).toHaveText("Active");

  await card.getByRole("button", { name: "Edit Literature watch", exact: true }).click();
  await expect(form.getByRole("heading", { name: "Edit mandate" })).toBeVisible();
  await expect(form.getByLabel("Project")).toBeDisabled();
  await expect(form.getByLabel("Target", { exact: true })).toHaveValue("12");
  await form.getByLabel("Name", { exact: true }).fill("Single-cell literature");
  await form.getByTestId("mandate-add-kpi").click();
  await expect(form.getByTestId("mandate-kpi-row")).toHaveCount(2);
  await form.getByLabel("KPI name").nth(1).fill("Reviews flagged");
  await form.getByRole("button", { name: "Remove KPI" }).first().click();
  await expect(form.getByTestId("mandate-kpi-row")).toHaveCount(1);
  await expect(form.getByLabel("KPI name")).toHaveValue("Reviews flagged");
  await form.getByTestId("mandate-save").click();
  await expect(form).toHaveCount(0);
  await expect(card.locator("strong").first()).toHaveText("Single-cell literature");
  await expect(card.locator(".mandate-kpis li")).toContainText("Reviews flagged");
  // An edit never restarts the mandate: only a new one carries a start time.
  expect(await page.evaluate(() => (window as any).__mandateDraft.start_at ?? null)).toBeNull();

  const remove = card.getByRole("button", { name: "Delete Single-cell literature", exact: true });
  await remove.click();
  await expect(card).toHaveCount(1);
  await expect(remove).toHaveAttribute("title", "Click again to delete");
  await remove.click();
  await expect(section.getByTestId("mandate-empty")).toBeVisible();
});

test("a mandate needs a goal and a future end date; the two forms share one Escape layer", async ({ page }) => {
  const automation = await open(page);
  await automation.getByTestId("mandate-create").click();
  const form = automation.getByTestId("mandate-form");
  await expect(form.getByTestId("mandate-review-mutations")).toBeChecked();
  await form.getByTestId("mandate-save").click();
  await expect(form.getByRole("alert")).toHaveText("Choose a project and write a goal.");
  await form.getByLabel("Goal", { exact: true }).fill("Improve the alignment benchmark\nover three months");
  await form.getByLabel("Until", { exact: true }).fill("2026-09-01");
  await form.getByTestId("mandate-save").click();
  await expect(form.getByRole("alert")).toHaveText("Choose an end date in the future, or leave it empty.");

  // Opening the task form replaces the mandate form instead of stacking on it.
  await automation.getByTestId("automation-create").click();
  await expect(form).toHaveCount(0);
  await expect(automation.getByTestId("automation-form")).toBeVisible();
  await automation.getByTestId("mandate-create").click();
  await expect(automation.getByTestId("automation-form")).toHaveCount(0);
  await expect(form).toBeVisible();
  // Escape right after opening closes only the form; the page stays.
  await page.keyboard.press("Escape");
  await expect(form).toHaveCount(0);
  await expect(automation).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(automation).toHaveCount(0);
  await expect(page.getByTestId("assistant-header")).toBeVisible();

  await page.getByTestId("open-automation").click();
  await automation.getByTestId("mandate-create").click();
  await form.getByLabel("Goal", { exact: true }).fill("Improve the alignment benchmark\nover three months");
  await form.getByTestId("mandate-save").click();
  const card = automation.locator(".mandate-card");
  await expect(card.locator("strong").first()).toHaveText("Improve the alignment benchmark");
  await expect(card).toContainText("wisp-science · Next round 2026-09-09 16:00");
  await expect(card).not.toContainText("until");
  await expect(card.locator(".mandate-kpis")).toHaveCount(0);
});

test("Chinese mandate labels follow the locale", async ({ page }) => {
  const automation = await open(page, "?mockLocale=zh");
  const section = automation.getByTestId("mandate-section");
  await expect(section.getByRole("heading", { name: "研究职责", exact: true })).toBeVisible();
  await expect(section.locator(".mandate-template")).toHaveCount(4);
  await expect(section.locator(".mandate-template").nth(1)).toContainText("投稿与返修跟进");
  await section.locator(".mandate-template").filter({ hasText: "方法指标迭代" }).click();
  const form = automation.getByTestId("mandate-form");
  await expect(form.getByRole("heading", { name: "新建职责" })).toBeVisible();
  await expect(form.getByLabel("指标名称")).toHaveValue("基准指标");
  await expect(form.getByLabel("需要先审核")).toHaveValue(/合并代码/);
  await form.getByTestId("mandate-save").click();
  await expect(section.locator(".mandate-card .mandate-status")).toHaveText("进行中");
  await expect(section.locator(".mandate-card")).toContainText("下一轮 2026-09-09 16:00 · 截至 2026-12-08");
});
