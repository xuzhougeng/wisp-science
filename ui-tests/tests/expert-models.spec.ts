import { test, expect, Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

async function profiles(page: Page) {
  return page.evaluate(() => (window as any).__TAURI__.core.invoke("list_models", {}));
}

for (const locale of ["en", "zh"]) {
  test(`expert opt-in survives editing and preserves the everyday model (${locale})`, async ({ page }) => {
    await page.addInitScript(tauriMock);
    await page.goto(`/?mockLocale=${locale}`);
    await page.getByRole("button", { name: locale === "zh" ? "设置" : "Settings", exact: true }).click();
    await page.getByTestId("settings-nav-models").click();
    const row = page.locator(".settings-list-row").filter({ hasText: "opus-4.8" });
    await row.click();
    const toggle = page.getByTestId("use-for-expert");
    await expect(toggle).not.toBeChecked();
    await toggle.check();
    await page.locator(".model-form").getByRole("button", { name: locale === "zh" ? "保存" : "Save", exact: true }).click();
    await expect(row.getByTestId("expert-role-badge")).toBeVisible();
    await expect.poll(async () => (await profiles(page)).find((p: any) => p.id === "opus").use_for_expert).toBe(true);
    expect((await profiles(page)).find((p: any) => p.active).id).toBe("default");
    await row.click();
    await expect(toggle).toBeChecked();
    await toggle.uncheck();
    await page.locator(".model-form").getByRole("button", { name: locale === "zh" ? "保存" : "Save", exact: true }).click();
    await expect(row.getByTestId("expert-role-badge")).toHaveCount(0);
    await expect.poll(async () => (await profiles(page)).find((p: any) => p.id === "opus").use_for_expert).toBe(false);
  });
}

test("a provider can add multiple expert profiles without replacing the everyday model", async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto("/");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByTestId("settings-nav-models").click();
  await page.getByTestId("model-presets").getByRole("button", { name: "DeepSeek", exact: true }).click();
  const rows = page.getByTestId("provider-model-row");
  await expect(rows).toHaveCount(2);
  for (const row of await rows.all()) await row.getByTestId("provider-use-for-expert").check();
  await page.getByTestId("save-provider").click();
  await expect.poll(async () => (await profiles(page)).filter((p: any) => p.use_for_expert).length).toBe(2);
  expect((await profiles(page)).find((p: any) => p.active).id).toBe("default");
});

test("expert diagnosis is visible in Agents and Escape closes only its detail", async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto("/?mockSubagent=1&mockSubagentActivity=1&mockExpertActivity=1");
  await page.locator(".proj-card-main").first().click();
  await page.locator('.sidebar [data-session-id="subagent-parent"]').click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator(".rightpane").getByRole("button", { name: "Agents", exact: true }).click();
  const panel = page.getByTestId("agent-workflows");
  const row = panel.locator('.agent-activity-row[data-activity-id="expert:x1"]');
  await expect(row).toHaveAttribute("data-activity-kind", "expert");
  await expect(row).toContainText("opus-4.8");
  await row.click();
  const detail = panel.getByTestId("agent-activity-detail");
  await expect(detail.locator(".agent-activity-facts")).toContainText("Expert consultation");
  await expect(detail).toContainText("opus-4.8");
  await expect(detail.getByTestId("agent-activity-answer")).toContainText("Check sample IDs");
  await expect(detail.getByTestId("agent-activity-open")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(detail).toHaveCount(0);
  await expect(row).toBeVisible();
});
