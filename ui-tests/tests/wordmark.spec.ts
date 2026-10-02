import { expect, test, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function selectTheme(page: Page, theme: "light" | "dark" | "system") {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Appearance", exact: true }).click();
  await page.getByTestId(`theme-mode-${theme}`).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await page.getByRole("button", { name: "Back to app", exact: true }).click();
}

for (const surface of ["projects", "chat"] as const) {
  test(`${surface} wordmark follows system appearance and explicit app overrides`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: "dark" });
    await page.goto("/");
    if (surface === "chat") {
      await page.locator(".proj-card-main").first().click();
      await page.locator(".sidebar").getByRole("button", { name: "New session" }).click();
    }
    const wordmark = page.locator(surface === "projects" ? ".projects-brand-mark" : ".empty-logo");
    await expect(wordmark).toHaveAccessibleName("Wisp Science");
    await expect(wordmark).toBeVisible();

    const expectVariant = async (theme: "light" | "dark") => {
      await expect(wordmark).toHaveCSS("background-image", new RegExp(`/wordmark-${theme}\\.svg`));
      // Decode the selected asset: a correct CSS URL alone would miss broken
      // Trunk copy paths or invalid SVGs in the packaged frontend.
      const ratio = await wordmark.evaluate(async (el) => {
        const img = new Image();
        img.src = getComputedStyle(el).backgroundImage.slice(5, -2);
        await img.decode();
        return img.naturalWidth / img.naturalHeight;
      });
      expect(ratio).toBeCloseTo(520 / 344, 1);
    };

    await expect(page.locator("html")).toHaveAttribute("data-theme", "system");
    await expectVariant("dark");
    await selectTheme(page, "light");
    await expectVariant("light");
    await page.emulateMedia({ colorScheme: "light" });
    await selectTheme(page, "dark");
    await expectVariant("dark");
    await selectTheme(page, "system");
    await expectVariant("light");
    await page.emulateMedia({ colorScheme: "dark" });
    await expectVariant("dark");
  });
}

for (const locale of ["en", "zh"]) {
for (const width of [390, 800, 900, 1000, 1100, 1600]) {
  test(`projects wordmark, tagline and actions fit a ${width}px window (${locale})`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 800 });
    await page.goto(`/?mockLocale=${locale}`);
    const wordmark = page.getByRole("heading", { name: "Wisp Science", exact: true });
    const brand = page.locator(".projects-brand");
    const tagline = page.locator(".projects-tagline");
    const actions = page.locator(".projects-actions");
    await expect(wordmark).toBeVisible();
    await expect(tagline).toHaveText(locale === "zh"
      ? "严谨做科研， Wisp Science 在身边。"
      : "Let rigor be your guide, with Wisp Science by your side.", { useInnerText: true });
    await expect(tagline).toBeInViewport();
    expect(await tagline.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
    await expect(actions.getByRole("button", { name: locale === "zh" ? "新建项目" : "New project" })).toBeVisible();
    const brandBox = (await brand.boundingBox())!;
    const actionsBox = (await actions.boundingBox())!;
    expect(brandBox.x).toBeGreaterThanOrEqual(0);
    expect(brandBox.x + brandBox.width).toBeLessThanOrEqual(width);
    expect(actionsBox.x).toBeGreaterThanOrEqual(0);
    expect(actionsBox.x + actionsBox.width).toBeLessThanOrEqual(width);
    expect(actionsBox.x >= brandBox.x + brandBox.width || actionsBox.y >= brandBox.y + brandBox.height).toBe(true);
    const quickBox = (await actions.locator(".projects-quick-actions").boundingBox())!;
    const mainBox = (await actions.locator(".projects-main-actions").boundingBox())!;
    if (width === 1600 || width === 800) {
      expect(Math.abs(quickBox.y + quickBox.height / 2 - mainBox.y - mainBox.height / 2)).toBeLessThan(2);
    } else {
      expect(mainBox.y).toBeGreaterThanOrEqual(quickBox.y + quickBox.height);
      expect(Math.abs(quickBox.x + quickBox.width - mainBox.x - mainBox.width)).toBeLessThan(2);
    }
    if (width > 820) {
      // Narrow desktop windows keep both action rows beside the brand.
      expect(actionsBox.x).toBeGreaterThanOrEqual(brandBox.x + brandBox.width);
    }
    const buttons = actions.getByRole("button");
    await expect(buttons).toHaveCount(9);
    for (const button of await buttons.all()) {
      await expect(button).toBeInViewport();
      expect(await button.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
    }
    const screen = page.locator(".projects-screen");
    expect(await screen.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
    await page.screenshot({ path: testInfo.outputPath(`projects-${locale}-${width}.png`), animations: "disabled" });
  });
}
}
