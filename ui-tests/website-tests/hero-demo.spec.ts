import { expect, test, type Locator, type Page } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.route(/^https:\/\//, route => route.abort());
});

// The demo's clock only runs while the mock is on screen, so scroll to it and
// drive Playwright's fake timers until the given condition holds. The site
// scrolls smoothly; jump instead, so the mock is fully in view before the
// clock moves and stays there.
// A click may scroll its target to the viewport edge, so re-center after
// clicks that are followed by running the clock.
const centerDemo = async (demo: Locator) => {
  await demo.evaluate((el) => {
    document.documentElement.style.scrollBehavior = "auto";
    el.scrollIntoView({ block: "center" });
  });
  await expect(demo).toBeInViewport({ ratio: 0.9 });
};
const openDemo = async (page: Page, lang: "zh" | "en" = "zh") => {
  await page.clock.install();
  await page.goto(`/index.html?lang=${lang}`);
  const demo = page.locator("[data-hero-demo]");
  await centerDemo(demo);
  return demo;
};
const runUntil = async (page: Page, check: () => Promise<boolean>) => {
  await expect.poll(async () => {
    if (await check()) return true;
    await page.clock.runFor(500);
    return check();
  }, { intervals: [50], timeout: 20_000 }).toBe(true);
};

test("hero demo plays a conversation once it is on screen, then moves to the next", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const demo = await openDemo(page);
  await expect(demo).toHaveAttribute("role", "region");
  await expect(demo).not.toHaveAttribute("aria-hidden");
  const status = demo.locator(".mock-status");
  const nav = demo.getByRole("navigation", { name: "示例对话" });
  await expect(nav.getByRole("button", { name: "CRISPR 筛选" })).toHaveAttribute("aria-pressed", "true");

  await runUntil(page, async () => (await demo.locator(".mock-composer").getAttribute("class"))!.includes("is-typing"));
  await runUntil(page, async () => (await demo.locator(".mock-step").count()) > 0);
  await expect(status).toContainText("运行中");
  await expect(demo.locator(".mock-msg.user")).toHaveText("检索 PubMed 上 CRISPR 筛选的最新方法，并画一张流程图。");

  await runUntil(page, async () => (await status.textContent())!.startsWith("已完成"));
  await expect(status).toContainText("3 个产物");
  await expect(demo.locator(".mock-step")).toHaveCount(8);
  await expect(demo.locator('.mock-step:not([data-state="done"])')).toHaveCount(0);
  await expect(demo.locator(".mock-artifact-list .mock-chip")).toHaveCount(3);
  await expect(demo.locator(".mock-msg.assistant")).toContainText("绘图代码与流程图已保存到项目中。");

  await runUntil(page, async () =>
    (await nav.getByRole("button", { name: "酶动力学" }).getAttribute("aria-pressed")) === "true");
  await expect(demo.locator(".mock-chat-title")).toHaveText("酶动力学");
});

test("choosing a conversation takes over and artifacts open a preview that Escape closes", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const demo = await openDemo(page);
  await demo.getByRole("button", { name: "PBMC 3k 质控" }).click();
  await expect(demo.getByRole("button", { name: "PBMC 3k 质控" })).toHaveAttribute("aria-pressed", "true");
  await centerDemo(demo);
  const status = demo.locator(".mock-status");
  await runUntil(page, async () => (await status.textContent())!.startsWith("已完成"));

  const chip = demo.locator('.mock-artifact-list [data-artifact="figure"]');
  await chip.click();
  const preview = demo.locator(".mock-preview");
  await expect(preview).toBeVisible();
  await expect(preview.locator("strong").first()).toHaveText("Figure 2");
  await expect(preview.locator("svg.mock-figure")).toBeVisible();
  await expect(demo.locator('.mock-step[data-artifact="figure"] .mock-step-link')).toHaveClass(/is-linked/);
  await page.keyboard.press("Escape");
  await expect(preview).toBeHidden();
  await expect(demo).toBeVisible();
  await expect(chip).toBeFocused();

  // The stream's artifact rows open the same preview.
  await demo.locator('.mock-step-link[data-artifact="table"]').click();
  await expect(preview.locator(".mock-table tbody tr")).toHaveCount(4);
  await expect(preview).toContainText("2,638 / 2,700");
  await preview.getByRole("button", { name: "关闭预览" }).click();
  await expect(preview).toBeHidden();

  // Rotation stopped once the visitor took over.
  await centerDemo(demo);
  await page.clock.runFor(20_000);
  await expect(demo.getByRole("button", { name: "PBMC 3k 质控" })).toHaveAttribute("aria-pressed", "true");
});

test("pausing the demo freezes the stream until it resumes", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const demo = await openDemo(page);
  await runUntil(page, async () => (await demo.locator(".mock-step").count()) >= 2);
  const toggle = demo.locator(".mock-toggle");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await centerDemo(demo);
  const steps = await demo.locator(".mock-step").count();
  const status = await demo.locator(".mock-status").textContent();
  await page.clock.runFor(5_000);
  await expect(demo.locator(".mock-step")).toHaveCount(steps);
  await expect(demo.locator(".mock-status")).toHaveText(status!);

  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await centerDemo(demo);
  await runUntil(page, async () => (await demo.locator(".mock-step").count()) > steps);
});

test("reduced motion shows each conversation finished and still switches", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/index.html?lang=en");
  const demo = page.locator("[data-hero-demo]");
  await expect(demo.locator(".mock-status")).toContainText("Done");
  await expect(demo.locator(".mock-toggle")).toBeHidden();
  await expect(demo.locator(".mock-msg.user")).toHaveText("Search PubMed for recent CRISPR screen methods and draft a flowchart.");
  await expect(demo.locator(".mock-artifact-list .mock-chip")).toHaveCount(3);

  await demo.getByRole("button", { name: "Enzyme kinetics" }).click();
  await expect(demo.locator(".mock-msg.assistant")).toContainText("3.1×");
  await expect(demo.locator(".mock-artifact-list .mock-chip")).toHaveCount(3);
  expect(await demo.evaluate(el => el.getAnimations({ subtree: true }).length)).toBe(0);

  await page.getByRole("button", { name: "中文", exact: true }).click();
  await expect(demo.locator(".mock-msg.user")).toHaveText("拟合 96 孔板中各突变体的 Michaelis–Menten 动力学，比较 kcat/Km。");
  await expect(demo.getByRole("button", { name: "酶动力学" })).toHaveAttribute("aria-pressed", "true");
});

test("on a phone the conversations become pills and artifacts open from the stream", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/index.html?lang=zh");
  const demo = page.locator("[data-hero-demo]");
  await demo.scrollIntoViewIfNeeded();
  await expect(demo.getByRole("button", { name: "酶动力学" })).toBeVisible();
  await expect(demo.locator(".mock-artifacts")).toBeHidden();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);

  await demo.locator('.mock-step-link[data-artifact="code"]').click();
  const preview = demo.locator(".mock-preview");
  await expect(preview).toContainText("figure_flow.py");
  await expect(preview.locator(".mock-code")).toContainText("import matplotlib.pyplot as plt");
  await page.keyboard.press("Escape");
  await expect(preview).toBeHidden();
});

test.describe("without JavaScript", () => {
  test.use({ javaScriptEnabled: false });

  test("the static mock stays as a decorative fallback", async ({ page }) => {
    await page.goto("/index.html");
    const visual = page.locator(".hero-visual");
    await expect(visual).toHaveAttribute("aria-hidden", "true");
    await expect(visual.locator(".mock-bubble.user")).toHaveText("检索 PubMed 上 CRISPR 筛选的最新方法，并画一张流程图。");
    await expect(visual.locator(".mock-chip")).toHaveCount(3);
  });
});
