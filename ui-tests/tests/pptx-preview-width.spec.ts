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
  await page.waitForTimeout(500);
  const a = await measure(page);
  await page.waitForTimeout(700);
  const b = await measure(page);

  // Container pinned to the figure (not runaway, not collapsed).
  expect(Math.abs(a.cw - a.figW)).toBeLessThan(30);
  // Stable over time — no feedback loop.
  expect(Math.abs(a.cw - b.cw)).toBeLessThan(5);
  // Slide actually rendered, not scaled to a near-zero sliver.
  expect(a.scale).not.toBeNull();
  expect(a.scale!).toBeGreaterThan(0.1);
});
