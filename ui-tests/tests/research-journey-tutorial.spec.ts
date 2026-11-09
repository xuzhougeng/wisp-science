import { test, expect } from "@playwright/test";
import { mkdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { tauriMock } from "./mock-tauri";

test.use({ timezoneId: "Asia/Shanghai", viewport: { width: 1440, height: 1000 } });
for (const locale of ["zh", "en"]) {
  test(`research journey tutorial (${locale}) follows calendar, sources, notes and publication`, async ({ page }, info) => {
    const tr = (zh: string, en: string) => locale === "zh" ? zh : en;
    await page.clock.setFixedTime(new Date("2026-09-09T08:00:00Z"));
    await page.addInitScript(tauriMock, {
      researchImageBase64: readFileSync(resolve(__dirname, "../fixtures/research-comparison.png")).toString("base64"),
    });
    if (locale === "en") {
      await page.addInitScript(() => {
        const core = (window as any).__TAURI__.core;
        const invoke = core.invoke;
        core.invoke = async (cmd: string, args: any) => {
          const result = await invoke(cmd, args);
          if (cmd === "list_recent_sessions") {
            return result.map((row: any) => ({ ...row, title: row.id === "s-needs-you" ? "Find a single-cell paper" : row.title }));
          }
          return result;
        };
      });
    }
    async function shot(name: string) {
      await page.evaluate(() => document.fonts.ready);
      if (locale === "en") {
        expect(await page.locator("body").innerText()).not.toMatch(/[\p{Script=Han}]/u);
        for (const field of await page.locator("input:visible, textarea:visible").all()) {
          expect(await field.inputValue()).not.toMatch(/[\p{Script=Han}]/u);
        }
      }
      const relative = `${locale === "en" ? "en/" : ""}research-journey/${name}.png`;
      const path = process.env.WISP_TUTORIAL_SHOTS
        ? resolve(process.env.WISP_TUTORIAL_SHOTS, relative) : info.outputPath(relative);
      mkdirSync(dirname(path), { recursive: true });
      await page.screenshot({ path, animations: "disabled" });
    }
    await page.goto(`/?mockLocale=${locale}&mockJourney=${locale === "zh" ? "design" : "default"}&mockPublication=frozen`);
    await page.getByTestId("open-research-calendar").click();
    const calendar = page.getByTestId("home-research-calendar");
    await expect(page.getByTestId("home-calendar-details")).toContainText("normalized_counts.csv");
    await shot("01-calendar");
    await calendar.locator(".calendar-project-link").first().click();
    const journey = page.getByTestId("research-journey");
    await expect(journey).toBeVisible();
    await journey.getByRole("button", { name: tr("显示整月", "Show full month"), exact: true }).click();
    await expect(journey.locator(".journey-day")).toHaveCount(3);
    await shot("02-journey");
    await journey.getByRole("button", { name: "normalized_counts.csv", exact: true }).click();
    const source = journey.getByTestId("journey-source");
    await expect(source).toContainText("counts_matrix.csv");
    await shot("03-source");
    await journey.getByRole("button", { name: tr("补充记录", "Add entry"), exact: true }).click();
    // A newly opened editor must close before its parent, without moving focus.
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(journey).toBeVisible();
    await journey.getByRole("button", { name: tr("补充记录", "Add entry"), exact: true }).click();
    const editor = page.getByRole("dialog", { name: tr("补充研究记录", "Add research entry"), exact: true });
    await editor.getByLabel(tr("研究日期", "Research date")).fill("2026-09-08");
    await editor.getByRole("combobox").selectOption("finding");
    const title = tr("小样本比较：方案 B 波动较小，仍需完整数据验证", "Small-sample comparison: B varies less; full-data validation pending");
    await editor.getByLabel(tr("标题", "Title"), { exact: true }).fill(title);
    await editor.getByLabel(tr("详情与依据", "Details and evidence")).fill(tr(
      "教学示例。依据：normalization_comparison.png v1。当前只比较了小样本；保留方案 A 作为基线，下一步在完整数据集上复核。",
      "Teaching example. Evidence: normalization_comparison.png v1. Only a small sample was compared. Keep A as the baseline and validate on the full dataset next.",
    ));
    await shot("04-entry");
    await editor.getByRole("button", { name: tr("保存记录", "Save entry"), exact: true }).click();
    await expect(editor).toHaveCount(0);
    await expect(journey).toContainText(title);
    await page.keyboard.press("Escape");
    await page.locator(".sidebar").getByRole("button", { name: tr("论文整理", "Publication"), exact: true }).click();
    await expect(page.getByTestId("publication-evidence-card")).toBeVisible();
    await shot("05-publication");
  });
}
