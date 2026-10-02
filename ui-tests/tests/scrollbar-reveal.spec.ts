import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const revealSource = readFileSync(resolve(__dirname, "../../ui/src/scrollbars.js"), "utf8");
const baseCss = readFileSync(resolve(__dirname, "../../ui/src/styles/base.css"), "utf8");

async function install(page: import("@playwright/test").Page) {
  await page.evaluate(async (source) => {
    const reveal = await import(URL.createObjectURL(new Blob([source], { type: "text/javascript" })));
    reveal.install_scrollbar_reveal();
  }, revealSource);
}

const thumbBackground = (el: HTMLElement) =>
  getComputedStyle(el, "::-webkit-scrollbar-thumb").backgroundColor;

test("a container shows its scrollbar thumb only while it scrolls", async ({ page }) => {
  await page.route("**/scrollbar-reveal", (route) => route.fulfill({
    contentType: "text/html",
    body: '<div id="box" style="height:200px;overflow:auto"><div style="height:1600px"></div></div>',
  }));
  await page.goto("/scrollbar-reveal");
  await install(page);

  const box = page.locator("#box");
  // base.css keeps the thumb transparent until the container scrolls or the
  // pointer reaches the strip — the paint side of the reveal contract.
  await page.evaluate((css) => {
    const style = document.createElement("style");
    style.textContent = css;
    document.head.appendChild(style);
  }, baseCss);
  await expect.poll(() => box.evaluate(thumbBackground)).toBe("rgba(0, 0, 0, 0)");

  await expect(box).not.toHaveClass(/is-scrolling/);
  await box.evaluate((el) => { el.scrollTop = 300; });
  await expect(box).toHaveClass(/is-scrolling/);
  await expect.poll(() => box.evaluate(thumbBackground)).toBe("rgba(138, 135, 127, 0.45)");
  await expect(box).toHaveClass(/is-scrolling/);

  // The thumb fades out again after the idle window elapses.
  await page.waitForTimeout(1_200);
  await expect(box).not.toHaveClass(/is-scrolling/);
  await expect.poll(() => box.evaluate(thumbBackground)).toBe("rgba(0, 0, 0, 0)");

  // Independent containers keep their own reveal timers: scrolling one leaves
  // the other untouched.
  await page.evaluate(() => {
    const box = document.getElementById("box")!;
    const second = box.cloneNode() as HTMLElement;
    second.id = "box-2";
    second.innerHTML = box.innerHTML;
    box.parentElement!.appendChild(second);
  });
  await page.locator("#box-2").evaluate((el) => { el.scrollTop = 100; });
  await expect(page.locator("#box-2")).toHaveClass(/is-scrolling/);
  await expect(page.locator("#box")).not.toHaveClass(/is-scrolling/);
});

test("a scrolling document marks the scrolling element", async ({ page }) => {
  await page.route("**/scrollbar-reveal-root", (route) => route.fulfill({
    contentType: "text/html",
    body: '<div style="height:3000px"></div>',
  }));
  await page.goto("/scrollbar-reveal-root");
  await install(page);

  await page.evaluate(() => { window.scrollTo(0, 500); });
  // The document's scroll event fires on the next rendering frame.
  await expect.poll(() =>
    page.evaluate(() => document.scrollingElement?.classList.contains("is-scrolling")),
  ).toBe(true);
});
