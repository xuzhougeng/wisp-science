import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => { await page.addInitScript(tauriMock); });

async function open(page: Page, mode = "", locale = "en", github = false) {
  await page.goto(`/?mockLocale=${locale}&mockSkillStore=${mode}`);
  await page.getByRole("button", { name: locale === "zh" ? "设置" : "Settings", exact: true }).click();
  await page.locator(".settings-nav").getByRole("button", { name: locale === "zh" ? "技能" : "Skills", exact: true }).click();
  await page.getByRole("button", { name: github ? "Add from GitHub" : locale === "zh" ? "浏览社区技能" : "Browse community Skills", exact: true }).click();
}
async function preview(page: Page) {
  await page.getByRole("button", { name: "Preview package", exact: true }).click();
  await page.locator(".skill-store-candidate").first().click();
}
async function installs(page: Page) {
  return page.evaluate(() => (window as any).__skillInvokeLog.filter((call: any) => call.cmd === "install_github_skill").length);
}

test("community directory searches, filters, and distinguishes counts and unverified source", async ({ page }) => {
  await open(page);
  await expect(page.getByTestId("skill-store")).toContainText("1 / 1 directory entries");
  await page.getByRole("textbox", { name: "Search community Skills" }).fill("absent");
  await expect(page.getByText("No matching community Skills.")).toBeVisible();
  await page.getByRole("textbox", { name: "Search community Skills" }).fill("handoff");
  await page.getByRole("combobox", { name: "Filter community tags" }).selectOption("research");
  await expect(page.locator(".skill-store-card")).toHaveCount(1);
  await preview(page);
  await expect(page.getByTestId("skill-store-preview")).toContainText("Author-declared compatibility");
  await expect(page.getByTestId("skill-store-preview")).toContainText("Format validation passed");
  await expect(page.getByTestId("skill-store-preview")).toContainText("Dependencies: pending review");
  await expect(page.locator(".skill-store-markdown")).toContainText("<script>");
  expect(await page.evaluate(() => (window as any).__storeUnsafe)).toBeUndefined();
  expect(await installs(page)).toBe(0);
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Add from GitHub", exact: true }).click();
  await expect(page.locator(".skill-store-candidate")).toHaveCount(0);
});

test("Escape immediately closes only confirmation, then preview, then store", async ({ page }) => {
  await open(page);
  await preview(page);
  await page.getByRole("button", { name: "Review installation", exact: true }).click();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Confirm Skill installation" })).toBeHidden();
  await expect(page.getByTestId("skill-store-preview")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("skill-store-preview")).toBeHidden();
  await expect(page.getByTestId("skill-store")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("skill-store")).toBeHidden();
  await expect(page.locator(".settings-nav")).toBeVisible();
  expect(await installs(page)).toBe(0);
});

test("explicit selection installs a single complete pinned package and refreshes installed details", async ({ page }) => {
  await open(page, "multi", "en", true);
  await page.getByTestId("skill-github-url").fill("https://github.com/example/science-skills/blob/release/v1/skills/research-handoff/SKILL.md");
  await page.getByTestId("skill-github-ref").fill("release/v1");
  await page.getByRole("button", { name: "Discover Skills", exact: true }).click();
  await expect(page.locator(".skill-store-candidate")).toHaveCount(2);
  await page.locator(".skill-store-candidate").filter({ hasText: "research-handoff" }).click();
  await expect(page.getByTestId("skill-store-preview")).toContainText("User-added GitHub source");
  await page.getByRole("button", { name: "Review installation", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Confirm Skill installation" });
  await expect(dialog).toContainText("all projects can discover");
  await expect(dialog).toContainText("a".repeat(40));
  expect(await installs(page)).toBe(0);
  await dialog.getByRole("button", { name: "Confirm and install", exact: true }).click();
  await expect(dialog).toBeHidden();
  expect(await installs(page)).toBe(1);
  await expect(page.getByRole("button", { name: "Review installation", exact: true })).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await expect(page.locator('.skill-catalog-row[data-skill-name="research-handoff"]')).toBeVisible();
  await expect(page.locator('.skill-catalog-row[data-skill-name="second-skill"]')).toHaveCount(0);
  await page.locator('.skill-catalog-row[data-skill-name="research-handoff"] .skill-row-open').click();
  await expect(page.locator(".skill-origin")).toContainText("a".repeat(40));
  await expect(page.locator(".skill-origin")).toContainText("release/v1");
});

for (const mode of ["conflict", "invalid"]) {
  test(`${mode} explains the blocker without overwriting any package`, async ({ page }) => {
    await open(page, mode); await preview(page);
    await expect(page.getByRole("button", { name: "Review installation", exact: true })).toBeDisabled();
    if (mode === "conflict") await expect(page.getByTestId("skill-store-preview")).toContainText("/app/skills/literature-review/SKILL.md");
    else {
      await expect(page.getByTestId("skill-store-preview")).toContainText("Format error");
      await expect(page.getByTestId("skill-store-preview")).toContainText("Resource error");
    }
    expect(await installs(page)).toBe(0);
  });
}

test("failed install keeps confirmation and supports retry without duplicate install", async ({ page }) => {
  await open(page, "fail"); await preview(page);
  await page.getByRole("button", { name: "Review installation", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Confirm Skill installation" });
  await dialog.getByRole("button", { name: "Confirm and install", exact: true }).click();
  await expect(dialog.getByRole("alert")).toContainText("download interrupted");
  await page.evaluate(() => { (window as any).__skillStoreRetry = true; });
  await dialog.getByRole("button", { name: "Confirm and install", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByTestId("skill-store")).toContainText("Installed: /home/test/.wisp/skills/research-handoff");
});

test("cancelled preview ignores its late result and never installs", async ({ page }) => {
  await open(page, "slow");
  await page.getByRole("button", { name: "Preview package", exact: true }).click();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.waitForTimeout(850);
  await expect(page.locator(".skill-store-candidate")).toHaveCount(0);
  expect(await installs(page)).toBe(0);
});

test("GitHub errors remain visible and failed directory refresh retains offline entries", async ({ page }) => {
  await open(page, "network");
  await page.getByRole("button", { name: "Preview package", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("404");
  await open(page, "offline");
  await page.getByRole("button", { name: "Refresh directory", exact: true }).click();
  await expect(page.getByTestId("skill-store")).toContainText("Showing the directory shipped");
  await expect(page.locator(".skill-store-card")).toHaveCount(1);
});

test("Chinese store remains usable at a narrow window size", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 820, height: 740 });
  await open(page, "", "zh");
  await expect(page.getByRole("button", { name: "Skill 制备指南", exact: true })).toBeVisible();
  await expect(page.getByText("免费社区目录。被收录不代表官方维护，也不代表已验证运行效果。")).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("skill-store-zh.png"), animations: "disabled" });
  expect(await page.getByTestId("skill-store").evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("skill-store")).toBeHidden();
  await expect(page.locator(".settings-nav")).toBeVisible();
});
