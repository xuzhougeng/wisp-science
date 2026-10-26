import { expect, test, type Locator } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.route(/^https:\/\//, route => route.abort());
});

// Script-driven spins (Web Animations) have no CSS animation name.
const scriptedSpins = (locator: Locator) =>
  locator.evaluateAll(els => els.map(el =>
    el.getAnimations().filter(a => !(a as CSSAnimation).animationName && !(a as CSSTransition).transitionProperty).length));

test("hero title bounces in per character and keeps its text after a language switch", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto("/index.html?lang=zh");
  const title = page.getByRole("heading", { level: 1, name: "严谨做科研， Wisp Science 在身边。" });
  await expect(title).toHaveClass(/is-split/);
  await expect(title).toHaveText("严谨做科研， Wisp Science 在身边。", { useInnerText: true });
  await expect(title.locator(".hero-char")).toHaveCount(21);
  // Words never break mid-word, so each stays on one line box.
  for (const word of await title.locator(".hero-word").all()) {
    expect(await word.evaluate(el => el.getClientRects().length)).toBe(1);
  }
  await expect(title).toHaveClass(/is-settled/);

  await page.getByRole("button", { name: "EN", exact: true }).click();
  const english = page.getByRole("heading", { level: 1, name: "Let rigor be your guide, with Wisp Science by your side." });
  await expect(english).toHaveClass(/is-split/);
  await expect(english).not.toHaveClass(/is-settled/);
  await expect(english).toHaveText("Let rigor be your guide, with Wisp Science by your side.", { useInnerText: true });
  await expect(english).toHaveClass(/is-settled/);

  const char = english.locator(".hero-char").first();
  await char.hover();
  await expect(char).toHaveClass(/is-hopping/);
  await expect(char).not.toHaveClass(/is-hopping/);
});

test("clicking a swirl spins it", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto("/index.html?lang=zh");
  const wordmark = page.getByRole("img", { name: "Wisp Science", exact: true });
  const wisps = wordmark.locator(".wm-orbit");
  await expect(wisps).toHaveCount(3);
  await expect.poll(() => wisps.evaluateAll(els => els.every(el =>
    el.getAnimations().some(a => (a as CSSAnimation).animationName === "wm-orbit")))).toBe(true);

  await wordmark.dispatchEvent("click");
  expect(await scriptedSpins(wisps)).toEqual([1, 1, 1]);

  const navLogo = page.locator(".site-nav .brand img");
  await navLogo.dispatchEvent("click");
  expect(await scriptedSpins(navLogo)).toEqual([1]);
});

test("reduced motion keeps the hero still", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/index.html?lang=zh");
  const hero = page.locator(".hero");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("严谨做科研， Wisp Science 在身边。", { useInnerText: true });
  expect(await hero.evaluate(el => el.getAnimations({ subtree: true }).length)).toBe(0);

  await page.getByRole("img", { name: "Wisp Science", exact: true }).dispatchEvent("click");
  await page.locator(".site-nav .brand img").dispatchEvent("click");
  await page.locator(".hero-char").first().hover();
  expect(await hero.evaluate(el => el.getAnimations({ subtree: true }).length)).toBe(0);
  expect(await scriptedSpins(page.locator(".site-nav .brand img"))).toEqual([0]);
});

test.describe("without JavaScript", () => {
  test.use({ javaScriptEnabled: false });

  test("hero title and wordmark render whole", async ({ page }) => {
    await page.goto("/index.html");
    const title = page.getByRole("heading", { level: 1 });
    await expect(title).toHaveText("严谨做科研， Wisp Science 在身边。", { useInnerText: true });
    await expect(title.locator(".hero-char")).toHaveCount(0);
    await expect(page.getByRole("img", { name: "Wisp Science", exact: true })).toBeVisible();
  });
});
