import { test, expect } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

for (const variant of [
  { name: "default", ui: 14, code: 12, font: "" },
  { name: "custom", ui: 20, code: 16, font: "Courier New" },
]) {
  test(`source editor caret and highlight share text metrics (${variant.name})`, async ({ page }, testInfo) => {
    await page.addInitScript(tauriMock);
    await page.addInitScript(({ ui, code, font }) => {
      localStorage.setItem("wisp-ui-font-size", String(ui));
      localStorage.setItem("wisp-code-font-size", String(code));
      localStorage.setItem("wisp-font-mono", font);
    }, variant);
    await page.goto("/");
    await page.locator(".proj-card-main").first().click();
    await page.getByRole("button", { name: "Files", exact: true }).click();
    await page.locator('[data-workspace-path="analysis.R"]').click({ button: "right" });
    await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
    const editor = page.getByRole("textbox", { name: "Source editor" });
    const source = 'plot(1:10)\nkjhk\n\tplot(2:11)\nplot(3:12)\nplot(4:13)\n';
    await expect(editor).toBeVisible();
    await editor.fill(source);
    const mirror = page.locator(".rp-code-edit-stack .rp-code-body code");
    await expect(mirror).toHaveAttribute("data-hl", "1");
    await expect(mirror).toHaveText(source + "\n", { useInnerText: false });
    await page.evaluate(() => document.fonts.ready);
    await editor.press("ArrowLeft");
    await page.locator(".rp-code-editor").screenshot({ path: testInfo.outputPath("editor.png"), caret: "initial" });

    const metrics = await page.locator(".rp-code-editor").evaluate(root => {
      const read = (selector: string) => {
        const style = getComputedStyle(root.querySelector(selector)!);
        return { font: style.fontFamily, size: style.fontSize, line: style.lineHeight,
          spacing: style.letterSpacing, tab: style.tabSize };
      };
      return {
        input: read(".rp-code-input"), body: read(".rp-code-body"),
        code: read(".rp-code-body code"), gutter: read(".rp-code-gutter"),
        selection: read(".rp-code-selection-layer"),
      };
    });
    await testInfo.attach("text-metrics", { body: JSON.stringify(metrics), contentType: "application/json" });
    expect(metrics.input.size).toBe(`${variant.code}px`);
    for (const [layer, style] of Object.entries(metrics)) {
      expect(style, layer).toEqual(metrics.input);
    }
    // The toolbar follows the UI size (its 12px design scaled by ui/14), not the code size.
    const runSize = await page.locator("[data-editor-run]").evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
    expect(runSize).toBeCloseTo((12 * variant.ui) / 14, 1);

    // Clicking glyphs in the visible mirror must place the native textarea
    // caret at the corresponding offset, including a tab and later lines.
    for (const offset of [4, source.indexOf("kjhk") + 3, source.indexOf("plot(2") + 4, source.indexOf("plot(4") + 4]) {
      const point = await mirror.evaluate((root, offset) => {
        const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
        let remaining = offset;
        while (walker.nextNode()) {
          const node = walker.currentNode as Text;
          if (remaining >= node.length) { remaining -= node.length; continue; }
          const range = document.createRange();
          range.setStart(node, remaining);
          range.setEnd(node, remaining + 1);
          const rect = range.getBoundingClientRect();
          return { x: rect.left + rect.width * 0.2, y: rect.top + rect.height / 2 };
        }
        throw new Error("Missing source glyph");
      }, offset);
      await page.mouse.click(point.x, point.y);
      expect(await editor.evaluate(el => (el as HTMLTextAreaElement).selectionStart)).toBe(offset);
    }
  });
}

// #1301: the mirror, the selection layer and the textarea must all be as wide
// as the widest line, so `.rp-code` is the only horizontal scroller. When they
// collapse to the pane width the textarea becomes a nested `overflow: hidden`
// scroller sitting on top, and horizontal panning stops working entirely.
test("long source lines pan horizontally in a single scroller", async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await page.getByRole("button", { name: "Files", exact: true }).click();
  await page.locator('[data-workspace-path="analysis.R"]').click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  const editor = page.getByRole("textbox", { name: "Source editor" });
  await expect(editor).toBeVisible();
  await editor.fill(`make_figure(${'"n_samples_COPD_PH", '.repeat(24)}1)\nshort\n`);

  const metrics = await page.locator(".rp-code-editor").evaluate(root => {
    const scroller = root.querySelector(".rp-code") as HTMLElement;
    const input = root.querySelector(".rp-code-input") as HTMLElement;
    const layer = root.querySelector(".rp-code-selection-layer") as HTMLElement;
    scroller.scrollLeft = scroller.scrollWidth;
    return {
      pan: scroller.scrollLeft,
      overflow: scroller.scrollWidth - scroller.clientWidth,
      inputClipped: input.scrollWidth - input.clientWidth,
      layerClipped: layer.scrollWidth - layer.clientWidth,
    };
  });
  expect(metrics.overflow).toBeGreaterThan(0);
  expect(metrics.pan).toBe(metrics.overflow);
  expect(metrics.inputClipped).toBe(0);
  expect(metrics.layerClipped).toBe(0);
});
