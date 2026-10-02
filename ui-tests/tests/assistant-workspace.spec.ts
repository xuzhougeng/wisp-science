import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.use({ timezoneId: "Asia/Shanghai", viewport: { width: 1440, height: 960 } });
test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(new Date("2026-09-09T08:00:00Z"));
  await page.addInitScript(tauriMock);
  await page.addInitScript(() => {
    (window as any).__assistantHistory = [
      { role: "user", text: "What should I work on next?", tool_name: null, ok: null },
      { role: "assistant", text: "## Research progress\n\nThe normalization comparison is complete. Review marker evidence before starting the next experiment.\n\n### Next steps\n\n1. Confirm the sample annotations.\n2. Discuss the study design and record your decisions.", tool_name: null, ok: null },
    ];
    (window as any).__assistantPlans = [
      { id: "carry", day: "2026-09-08", title: "Review marker evidence", project_id: "default", project_name: "Rice root atlas", session_id: null, status: "open" },
      { id: "today", day: "2026-09-09", title: "Confirm sample annotations", project_id: null, project_name: null, session_id: null, status: "done" },
      { id: "hidden", day: "2026-09-09", title: "Other private plan", project_id: "other", project_name: "Other project", session_id: "private-session", status: "open" },
    ];
  });
});
async function open(page: Page, query = "") {
  await page.goto(`/${query}`);
  await expect(page.locator(".proj-card-main").first()).toBeVisible();
  await page.getByTestId("open-research-assistant").click();
  await expect(page.getByTestId("assistant-header")).toBeVisible();
  await expect(page.locator(".chat")).toContainText("The normalization comparison is complete.");
}
async function calls(page: Page, command: string) {
  return page.evaluate(cmd => ((window as any).__skillInvokeLog ?? []).filter((call: any) => call.cmd === cmd).map((call: any) => {
    const plain = (v: any): any => v instanceof Map ? Object.fromEntries([...v].map(([k, v]) => [k, plain(v)])) : Array.isArray(v) ? v.map(plain) : v;
    return plain(call.args);
  }), command);
}
const composer = (page: Page) => page.locator(".composer textarea").first();

test("sidebars collapse independently without losing draft, date, scroll or conversation", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 640 });
  await open(page);
  const left = page.getByTestId("assistant-projects");
  const right = page.getByTestId("assistant-calendar");
  await expect(left).toBeVisible(); await expect(right).toBeVisible();
  await right.getByRole("button", { name: "2026-09-08", exact: true }).click();
  await expect(right).toContainText("Other project run failed");
  await composer(page).fill("Keep my unfinished question");
  await page.locator(".assistant-calendar-body").evaluate(el => el.scrollTop = 100);
  const scroll = await page.locator(".assistant-calendar-body").evaluate(el => el.scrollTop);
  expect(scroll).toBeGreaterThan(0);
  const reads = (await calls(page, "get_research_calendar")).length;
  await page.locator("#assistant-projects-toggle").click();
  await expect(left).toBeHidden(); await expect(right).toBeVisible();
  await page.locator("#assistant-calendar-toggle").click();
  await expect(right).toBeHidden();
  await expect(composer(page)).toHaveValue("Keep my unfinished question");
  await page.locator("#assistant-calendar-toggle").click();
  await expect(right.locator('[data-date="2026-09-08"]')).toHaveAttribute("aria-pressed", "true");
  expect(await page.locator(".assistant-calendar-body").evaluate(el => el.scrollTop)).toBe(scroll);
  expect((await calls(page, "get_research_calendar")).length).toBe(reads);
  expect((await calls(page, "load_session")).filter((a: any) => a.id === "research-assistant")).toHaveLength(1);
  await page.keyboard.press("Escape");
  await expect(page.locator(".projects-screen")).toBeVisible();
  await page.reload();
  await page.getByTestId("open-research-assistant").click();
  await expect(left).toBeHidden(); await expect(right).toBeVisible();
  await page.locator("#assistant-projects-toggle").click();
  await expect(left).toBeVisible();
});

test("project selection adds context to the same assistant session and survives sending", async ({ page }) => {
  await open(page);
  await page.getByTestId("assistant-projects").locator('[data-project-id="other"]').click();
  await expect(page.getByTestId("assistant-project-context")).toContainText("Other project");
  // A context selection alone must not send an empty message.
  await composer(page).press("Enter");
  expect(await calls(page, "send_message")).toHaveLength(0);
  await composer(page).fill("Summarize the next experiment");
  await composer(page).press("Enter");
  await expect.poll(() => calls(page, "send_message")).toHaveLength(1);
  const sent = (await calls(page, "send_message"))[0];
  expect(sent.sessionId).toBe("research-assistant");
  expect(sent.references).toContainEqual({ kind: "project", id: "other" });
  expect(await calls(page, "open_project")).toHaveLength(0);
  await expect(page.getByTestId("assistant-project-context")).toContainText("Other project");
  await page.getByRole("button", { name: "Clear project context", exact: true }).click();
  await expect(page.getByTestId("assistant-project-context")).toHaveCount(0);
});

test("saved plans carry their dates and refresh; calendar actions only prepare drafts", async ({ page }) => {
  await open(page);
  const right = page.getByTestId("assistant-calendar");
  const plans = page.getByTestId("assistant-plans");
  await expect(plans).toContainText("Originally 2026-09-08");
  await expect(plans.locator('[data-plan-id="today"]')).toHaveAttribute("data-status", "done");
  await composer(page).fill("Existing draft");
  await right.getByRole("button", { name: "Plan this day", exact: true }).click();
  await expect(composer(page)).toHaveValue("Existing draft\n\nHelp me plan my research for 2026-09-09.");
  expect(await calls(page, "send_message")).toHaveLength(0);
  await right.getByRole("button", { name: "Other project · Ask about this day", exact: true }).click();
  await expect(composer(page)).toHaveValue(/Other project/);
  expect(await calls(page, "open_project")).toHaveLength(0);
  await page.evaluate(() => { (window as any).__assistantPlanError = true; });
  await right.getByRole("button", { name: "Refresh calendar", exact: true }).click();
  await expect(plans.getByRole("alert")).toContainText("Plan store unavailable");
  await page.evaluate(() => {
    (window as any).__assistantPlanError = false;
    (window as any).__assistantPlans[0].status = "done";
  });
  await plans.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(plans.locator('[data-plan-id="carry"]')).toHaveCount(0);
  await right.getByRole("button", { name: "2026-09-08", exact: true }).click();
  await expect(plans.locator('[data-plan-id="carry"]')).toHaveAttribute("data-status", "done");
  await expect(plans).not.toContainText("Originally");
});

test("project privacy is authoritative and unresolved visibility issues no calendar or plan reads", async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__assistantHiddenProjects = ["other"];
    (window as any).__assistantProjectsDelay = 800;
    (window as any).__assistantProjectsError = true;
  });
  await open(page);
  await expect(page.getByTestId("assistant-calendar")).toContainText("Loading visible projects");
  expect(await calls(page, "get_research_calendar")).toHaveLength(0);
  expect(await calls(page, "get_research_assistant_plan")).toHaveLength(0);
  await expect(page.getByTestId("assistant-calendar").getByRole("alert")).toBeVisible();
  expect(await calls(page, "get_research_calendar")).toHaveLength(0);
  await page.evaluate(() => { (window as any).__assistantProjectsError = false; });
  await page.getByTestId("assistant-calendar").getByRole("button", { name: "Retry", exact: true }).click();
  await expect(page.getByTestId("assistant-plans")).toContainText("Review marker evidence");
  await expect(page.getByTestId("assistant-projects")).not.toContainText("Other project");
  await expect(page.getByTestId("assistant-calendar")).not.toContainText("Other private plan");
  expect((await calls(page, "get_research_calendar")).flatMap((a: any) => a.projectIds)).not.toContain("other");
});

test("assistant turn completion refreshes saved plans without changing the selected date", async ({ page }) => {
  await open(page);
  await expect(page.getByTestId("assistant-plans")).toContainText("Review marker evidence");
  const reads = (await calls(page, "get_research_assistant_plan")).length;
  await page.evaluate(() => {
    (window as any).__assistantPlans[1].title = "Updated after the assistant turn";
  });
  await composer(page).fill("Save the plan");
  await composer(page).press("Enter");
  await expect(page.getByTestId("assistant-plans")).toContainText("Updated after the assistant turn");
  expect((await calls(page, "get_research_assistant_plan")).length).toBeGreaterThan(reads);
  await expect(page.locator('[data-date="2026-09-09"]')).toHaveAttribute("aria-pressed", "true");
});

test("narrow drawers and window palettes close in visual order on immediate Escape", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 900 });
  await page.addInitScript(() => Object.defineProperty(navigator, "platform", { get: () => "Win32" }));
  await open(page);
  const left = page.getByTestId("assistant-projects");
  const right = page.getByTestId("assistant-calendar");
  await expect(left).toBeHidden(); await expect(right).toBeHidden();
  await page.locator(".model-picker-btn").click();
  await page.keyboard.press("Escape");
  await expect(page.locator(".model-menu")).toHaveCount(0);
  await expect(page.getByTestId("assistant-header")).toBeVisible();
  await page.locator("#assistant-projects-toggle").click();
  await page.locator("#assistant-calendar-toggle").click();
  await page.keyboard.press("Control+p");
  const palette = page.getByRole("dialog", { name: "Command Palette", exact: true });
  await expect(palette).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(palette).toHaveCount(0); await expect(right).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(right).toBeHidden(); await expect(left).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(left).toBeHidden(); await expect(page.getByTestId("assistant-header")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("assistant-header")).toHaveCount(0);
});

test("three-column, reading and narrow layouts fit light and dark windows", async ({ page }, testInfo) => {
  const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
  await open(page, "?mockLocale=zh");
  await expect(page.getByTestId("assistant-plans")).toContainText("Review marker evidence");
  for (const width of [1600, 1024, 390]) {
    await page.setViewportSize({ width, height: 960 });
    if (width < 960) await page.locator("#assistant-calendar-toggle").click();
    const panel = page.getByTestId("assistant-calendar");
    await expect(panel).toBeVisible();
    expect(await panel.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
    const box = await panel.boundingBox(); expect(box!.x + box!.width).toBeLessThanOrEqual(width);
    expect(await page.locator(".app.assistant-mode").evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
    await page.screenshot({ path: testInfo.outputPath(`assistant-${width}.png`) });
  }
  await page.setViewportSize({ width: 1600, height: 960 });
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await page.screenshot({ path: testInfo.outputPath("assistant-dark.png") });
  await page.locator("#assistant-projects-toggle").click();
  await page.locator("#assistant-calendar-toggle").click();
  await page.screenshot({ path: testInfo.outputPath("assistant-reading.png") });
  expect(errors).toEqual([]);
});
