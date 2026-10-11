import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.route(/^https:\/\//, route => route.abort());
});

for (const width of [1440, 320]) {
  test(`homepage milestones stay readable and translate at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/index.html?lang=zh");
    const milestones = page.getByRole("group", { name: /从 v0.1 首次发布至今/ });
    await expect(milestones).toBeVisible();
    await expect(milestones.locator("dd")).toHaveText(["19K", "50K+", "1,000+"]);
    await expect(milestones.locator("dt")).toHaveText([
      "累计下载", "相关教程与文章阅读", "GitHub Stars",
    ]);
    await expect(milestones.getByRole("link", { name: "GitHub Stars" })).toHaveAttribute(
      "href", "https://github.com/xuzhougeng/wisp-science/stargazers",
    );

    await page.getByRole("button", { name: "EN", exact: true }).click();
    const translated = page.getByRole("group", { name: /From the first v0.1 release to today/ });
    await expect(translated.locator("dt")).toHaveText([
      "Total downloads", "Reads of tutorials & articles", "GitHub Stars",
    ]);
    await expect(translated.locator("dd")).toHaveText(["19K", "50K+", "1,000+"]);
    for (const metric of await translated.locator("dt, dd").all()) {
      await expect(metric).toBeVisible();
      const bounds = await metric.boundingBox();
      expect(bounds).not.toBeNull();
      expect(bounds!.x).toBeGreaterThanOrEqual(0);
      expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
      expect(await metric.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
    }

    await page.getByRole("button", { name: "中文", exact: true }).click();
    await expect(milestones.locator("dt")).toHaveText([
      "累计下载", "相关教程与文章阅读", "GitHub Stars",
    ]);
  });
}

test.describe("without JavaScript", () => {
  test.use({ javaScriptEnabled: false });

  test("homepage milestones include readable fallback content", async ({ page }) => {
    await page.goto("/index.html");
    const milestones = page.getByRole("group", { name: /从 v0.1 首次发布至今/ });
    await expect(milestones).toBeVisible();
    await expect(milestones.locator("dt")).toHaveText([
      "累计下载", "相关教程与文章阅读", "GitHub Stars",
    ]);
    await expect(milestones.locator("dd")).toHaveText(["19K", "50K+", "1,000+"]);
  });
});
