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

test("preview status animates, respects reduced motion, and clears on completion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await open(page, "pending");
  await page.getByRole("button", { name: "Preview package", exact: true }).click();
  const loading = page.getByTestId("skill-store-loading");
  await expect(loading.getByRole("status")).toContainText("Preparing preview");
  const previewButton = page.locator(".skill-store-preview-button");
  await expect(previewButton).toHaveText("Previewing…");
  await expect(previewButton).toHaveAttribute("aria-busy", "true");
  await expect(previewButton).toBeDisabled();
  await expect(previewButton.locator("svg")).toHaveCSS("animation-name", "skill-store-loading-spin");
  await expect(loading.locator("svg")).toHaveCSS("animation-name", "skill-store-loading-spin");
  await expect(loading.getByRole("button", { name: "Cancel", exact: true })).toBeEnabled();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(loading).toHaveCSS("animation-name", "none");
  await expect(loading.locator("svg")).toHaveCSS("animation-name", "none");
  await expect(previewButton.locator("svg")).toHaveCSS("animation-name", "none");
  await page.evaluate(() => (window as any).__resolveSkillPreview());
  await expect(loading).toBeHidden();
  await expect(previewButton).toHaveText("Preview package");
  await expect(previewButton).toBeEnabled();
  await expect(previewButton).toHaveAttribute("aria-busy", "false");
  await expect(previewButton.locator("svg")).toHaveCount(0);
  await expect(page.locator(".skill-store-candidate")).toHaveCount(1);
});

test("Chinese preview status fits a narrow window and Escape cancels only the preview", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 820, height: 740 });
  await open(page, "pending", "zh");
  await page.getByRole("button", { name: "预览技能包", exact: true }).click();
  await page.keyboard.press("Escape");
  const loading = page.getByTestId("skill-store-loading");
  await expect(loading).toBeHidden();
  await expect(page.getByTestId("skill-store")).toBeVisible();
  await page.evaluate(() => (window as any).__resolveSkillPreview());
  await expect(page.locator(".skill-store-candidate")).toHaveCount(0);
  await page.getByRole("button", { name: "预览技能包", exact: true }).click();
  await expect(loading.getByRole("status")).toContainText("正在准备预览");
  await expect(page.locator(".skill-store-preview-button")).toHaveText("正在预览…");
  await loading.scrollIntoViewIfNeeded();
  await expect(loading.getByRole("button", { name: "取消", exact: true })).toBeInViewport();
  expect(await loading.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("skill-store-loading-zh.png"), animations: "disabled" });
  await loading.getByRole("button", { name: "取消", exact: true }).click();
  await expect(loading).toBeHidden();
  await expect(page.getByRole("button", { name: "预览技能包", exact: true })).toBeEnabled();
  expect(await installs(page)).toBe(0);
});

for (const width of [820, 1440]) {
  test(`Skills toolbar groups actions without overflow at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await open(page, "", "zh");
    await page.keyboard.press("Escape");
    const pane = page.getByTestId("skills-pane");
    const search = pane.getByRole("textbox", { name: "搜索技能…" });
    await expect(search).toBeVisible();
    for (const selector of [".skills-toolbar", ".skills-primary-actions", ".skills-list-controls", ".skill-tags-filter"]) {
      expect(await pane.locator(selector).evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    }
    const controls = await pane.locator(".skills-list-controls").boundingBox();
    const toolbar = await pane.locator(".skills-toolbar").boundingBox();
    expect(controls!.y).toBeGreaterThanOrEqual(toolbar!.y + toolbar!.height);
    await pane.locator("summary").click();
    await page.keyboard.press("Escape");
    await expect(pane.locator("details")).not.toHaveAttribute("open", "");
    await expect(pane).toBeVisible();
    await pane.locator(".skill-tags-filter").getByRole("button", { name: "compute", exact: true }).click();
    await expect(pane.locator(".skill-catalog-row")).toHaveCount(1);
    await pane.getByRole("button", { name: "关闭当前筛选", exact: true }).click();
    await expect(pane.locator(".skill-catalog-row input[type=checkbox]")).not.toBeChecked();
    await pane.getByRole("button", { name: "启用当前筛选", exact: true }).click();
    await expect(pane.locator(".skill-catalog-row input[type=checkbox]")).toBeChecked();
    await pane.locator(".skill-tags-filter").getByRole("button", { name: "全部", exact: true }).click();
    await search.fill("paper-narrative");
    await expect(pane.locator(".skill-catalog-row")).toHaveCount(1);
    await search.clear();
    await page.screenshot({ path: testInfo.outputPath(`skills-toolbar-${width}.png`), animations: "disabled" });
  });
}

test("GitHub errors remain visible and failed directory refresh retains offline entries", async ({ page }) => {
  await open(page, "network");
  await page.getByRole("button", { name: "Preview package", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("404");
  await open(page, "offline");
  await page.getByRole("button", { name: "Refresh directory", exact: true }).click();
  await expect(page.getByTestId("skill-store")).toContainText("Showing the directory shipped");
  await expect(page.locator(".skill-store-card")).toHaveCount(1);
});

test("Skills filtering, detail, and file loading animate and respect reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await open(page, "motion");
  await page.keyboard.press("Escape");
  const pane = page.getByTestId("skills-pane");
  await pane.locator(".skill-tags-filter").getByRole("button", { name: "compute", exact: true }).click();
  await expect(pane.locator(".skill-catalog-row")).toHaveCount(1);
  await pane.locator(".skill-tags-filter").getByRole("button", { name: "All", exact: true }).click();
  const row = pane.locator('[data-skill-name="paper-narrative"]');
  await expect(row).toHaveCSS("animation-name", "skills-content-enter");
  await row.locator(".skill-row-open").click();
  const detail = page.getByTestId("skill-detail");
  await expect(detail).toHaveCSS("animation-name", "skills-detail-enter");
  await expect(detail.getByRole("status")).toContainText("Loading");
  await expect(detail.locator(".skills-loading-icon svg")).toHaveCSS("animation-name", "skill-store-loading-spin");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(detail).toHaveCSS("animation-name", "none");
  await expect(detail.locator(".skills-loading-icon svg")).toHaveCSS("animation-name", "none");
  await page.evaluate(() => (window as any).__resolveSkillFiles());
  await expect(detail.locator(".skill-file-loading")).toBeHidden();
  await expect(page.getByTestId("skill-file-preview")).toHaveCSS("animation-name", "none");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await detail.getByRole("button", { name: "Source", exact: true }).click();
  await expect(page.getByTestId("skill-file-source")).toHaveCSS("animation-name", "skills-content-enter");
  await page.keyboard.press("Escape");
  await expect(pane).toBeVisible();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(pane.locator(".skill-catalog-row").first()).toHaveCSS("animation-name", "none");
  await expect(pane.locator(".skill-tags-filter button").first()).toHaveCSS("transition-duration", "0s");
});

test("Skills reload shows busy feedback and recovers after both failure and success", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await open(page, "motion");
  await page.keyboard.press("Escape");
  const pane = page.getByTestId("skills-pane");
  const reload = pane.locator(".skills-reload");
  await reload.click();
  await expect(reload).toBeDisabled();
  await expect(reload).toHaveText("Reloading skills…");
  await expect(reload.locator("svg")).toHaveCSS("animation-name", "skill-store-loading-spin");
  await expect(pane.getByRole("status")).toContainText("Reloading skills");
  await page.evaluate(() => (window as any).__rejectSkillReload());
  await expect(pane).toContainText("Skill reload interrupted");
  await expect(reload).toBeEnabled();
  await reload.click();
  await expect(pane).not.toContainText("Skill reload interrupted");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(reload.locator("svg")).toHaveCSS("animation-name", "none");
  await page.evaluate(() => (window as any).__resolveSkillReload());
  await expect(reload).toBeEnabled();
  await expect(reload).toHaveText("Reload skills");
  await expect(pane).toContainText("Skills reloaded");
});

for (const [marketplace, repository, directory] of [
  ["OpenAI Skills", "openai/skills", "skills/.curated"],
  ["Anthropic Skills", "anthropics/skills", "skills"],
  ["BEAR Research Skills", "fei0810/bear-research-skills", "skills"],
]) {
  test(`${marketplace} is a default source with searchable packages and pinned single-package installation`, async ({ page }) => {
    await open(page, "multi");
    await expect(page.getByRole("group", { name: "Default Skill marketplaces" })).toBeVisible();
    await page.getByRole("button", { name: marketplace, exact: true }).click();
    await expect(page.locator(".skill-store-candidate")).toHaveCount(2);
    const calls = await page.evaluate(() => (window as any).__skillInvokeLog.filter((c: any) => c.cmd === "preview_github_skills").map((c: any) => c.args instanceof Map ? Object.fromEntries(c.args) : c.args));
    expect(calls.at(-1)).toEqual({ sourceUrl: `https://github.com/${repository}/tree/main/${directory}`, exactRef: "main" });
    await page.getByRole("searchbox", { name: "Search source packages" }).fill("second-skill");
    await expect(page.locator(".skill-store-candidate")).toHaveCount(1);
    await page.locator(".skill-store-candidate").click();
    const detail = page.getByTestId("skill-store-preview");
    await expect(detail.locator("h3")).toHaveText("second-skill");
    await expect(detail).toContainText(marketplace);
    await expect(detail).not.toContainText("User-added GitHub source");
    await page.getByRole("button", { name: "Review installation", exact: true }).click();
    const confirm = page.getByRole("dialog", { name: "Confirm Skill installation" });
    await expect(confirm).toContainText(`${directory}/second-skill`);
    await expect(confirm).toContainText("a".repeat(40));
    expect(await installs(page)).toBe(0);
    await confirm.getByRole("button", { name: "Confirm and install", exact: true }).click();
    await expect(confirm).toBeHidden();
    expect(await installs(page)).toBe(1);
    const source = await page.evaluate(() => (window as any).__skillStoreOrigins["second-skill"]);
    expect(source.repository).toBe(repository);
    expect(source.package_path).toBe(`${directory}/second-skill`);
    await page.keyboard.press("Escape");
    await expect(page.locator(".skill-store-candidate")).toHaveCount(1);
    await page.keyboard.press("Escape");
    await expect(page.locator(".skill-store-card")).toHaveCount(1);
    await expect(page.getByTestId("skill-store")).toBeVisible();
    await expect(page.locator(".skill-store-candidate")).toHaveCount(0);
    await page.getByRole("button", { name: "Preview package", exact: true }).click();
    await expect(page.getByRole("searchbox", { name: "Search source packages" })).toHaveValue("");
    await expect(page.locator(".skill-store-candidate")).toHaveCount(2);
  });
}

test("default marketplace cancellation ignores a late response and offers reload", async ({ page }) => {
  await open(page, "pending");
  await page.getByRole("button", { name: "OpenAI Skills", exact: true }).click();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("skill-store-loading")).toBeHidden();
  await expect(page.getByRole("button", { name: "Load / refresh source" })).toBeEnabled();
  await page.evaluate(() => (window as any).__resolveSkillPreview());
  await expect(page.locator(".skill-store-candidate")).toHaveCount(0);
  await page.getByRole("button", { name: "Load / refresh source" }).click();
  await expect(page.getByTestId("skill-store-loading")).toBeVisible();
  await page.evaluate(() => (window as any).__resolveSkillPreview());
  await expect(page.locator(".skill-store-candidate")).toHaveCount(1);
  expect(await installs(page)).toBe(0);
});

test("BEAR can be installed on demand as a global Skill", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "BEAR Research Skills", exact: true }).click();
  await page.locator(".skill-store-candidate").filter({ hasText: "bear-support" }).click();
  await page.getByRole("button", { name: "Review installation", exact: true }).click();
  const confirm = page.getByRole("dialog", { name: "Confirm Skill installation" });
  await expect(confirm).toContainText("~/.wisp/skills/bear-support");
  await confirm.getByRole("button", { name: "Confirm and install", exact: true }).click();
  await expect(confirm).toBeHidden();
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  const skill = page.locator('[data-skill-name="bear-support"]');
  await expect(skill).toContainText("Global");
  await expect(skill.locator('input[type="checkbox"]')).toBeChecked();
});

test("default marketplace failures keep source navigation and retry available", async ({ page }) => {
  await open(page, "network");
  await page.getByRole("button", { name: "Anthropic Skills", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("404");
  await expect(page.getByRole("button", { name: "Load / refresh source" })).toBeEnabled();
  await page.getByRole("button", { name: "Browse community Skills", exact: true }).click();
  await expect(page.locator(".skill-store-card")).toHaveCount(1);
  await expect(page.getByRole("alert")).toHaveCount(0);
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
