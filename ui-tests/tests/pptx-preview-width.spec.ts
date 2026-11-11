import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { tauriMock } from "./mock-tauri";
const officeFixtures = {
  xlsxBase64: readFileSync(resolve(__dirname, "../fixtures/office-preview.xlsx")).toString("base64"),
  pptxBase64: readFileSync(resolve(__dirname, "../fixtures/office-preview.pptx")).toString("base64"),
};
// Regression for the PPTX preview feedback loop: the renderer sizes each slide to
// the container's width, so if the mount host isn't width-pinned the container grows
// (twitch) or collapses to a sliver (blank until resize). The container must stay
// pinned to the figure and stable over time. Run narrow, where the bug is worst.
test("PPTX preview width stays pinned in a narrow modal", async ({ page }) => {
  await page.setViewportSize({ width: 380, height: 800 });
  await page.addInitScript(tauriMock, officeFixtures);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('.fb-row[data-workspace-path="office-preview.pptx"]').click();
  await expect(page.locator(".artifact-modal .rp-pptx")).toBeVisible();

  const measure = (page: Page) => page.evaluate(() => {
    const c = document.querySelector(".rp-pptx") as HTMLElement;
    const fig = document.querySelector(".am-figure.office-preview") as HTMLElement;
    const content = document.querySelector('.rp-pptx [data-slide-index="0"] > div > *') as HTMLElement | null;
    const m = (content?.style.transform || "").match(/scale\(([\d.]+)\)/);
    return { cw: c.clientWidth, figW: fig.offsetWidth, scale: m ? parseFloat(m[1]) : null };
  });
  // Wait for the slide content to actually render (its scale transform is set
  // by the sizing pass) instead of sleeping and hoping it has.
  await expect.poll(async () => (await measure(page)).scale).not.toBeNull();
  const a = await measure(page);
  // Sample consecutive layout frames: ResizeObserver feedback has a chance
  // to run on each frame, without tying the regression to a wall-clock delay.
  const widths = await page.evaluate(async () => {
    const widths: number[] = [];
    for (let frame = 0; frame < 30; frame++) {
      await new Promise(requestAnimationFrame);
      widths.push((document.querySelector(".rp-pptx") as HTMLElement).clientWidth);
    }
    return widths;
  });

  // Container pinned to the figure (not runaway, not collapsed).
  expect(Math.abs(a.cw - a.figW)).toBeLessThan(30);
  // Stable over time — no feedback loop.
  expect(Math.max(a.cw, ...widths) - Math.min(a.cw, ...widths)).toBeLessThan(5);
  // Slide actually rendered, not scaled to a near-zero sliver.
  expect(a.scale).not.toBeNull();
  expect(a.scale!).toBeGreaterThan(0.1);
});
