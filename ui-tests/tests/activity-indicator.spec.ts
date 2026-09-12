import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function start(page: Page, locale = "en") {
  await page.goto(`/?mockLocale=${locale}`);
  await page.locator(".proj-card-main").first().click();
  await page.locator("#composer-input").fill("INDICATORDEMO · 对比论文中的关键发现");
  await page.getByRole("button", { name: locale === "zh" ? "发送" : "Send", exact: true }).click();
  await expect(page.getByTestId("steps-indicator")).toHaveAttribute("data-state", "running");
  return page.evaluate(() => {
    const calls = (window as any).__skillInvokeLog as { cmd: string; args: any }[];
    const args = calls.filter(call => call.cmd === "send_message").at(-1)!.args;
    return (args instanceof Map ? args.get("sessionId") : args.sessionId) as string;
  });
}

async function emit(page: Page, frameId: string, event: Record<string, unknown>) {
  await page.evaluate(payload => (window as any).__tauriEmit("agent", payload), { frame_id: frameId, ...event });
}

for (const locale of ["en", "zh"]) {
  test(`activity indicator combines breathing and orbit with a quiet waiting subtitle (${locale})`, async ({ page }) => {
    const frameId = await start(page, locale);
    const indicator = page.getByTestId("steps-indicator");
    const head = page.locator(".steps-head");
    const title = head.locator(".steps-title");
    await expect(title).toHaveText(locale === "zh" ? "执行中…" : "Working…");
    await expect(page.getByTestId("steps-wait-note")).toHaveCount(0);
    const arc = indicator.locator("svg > path");
    const dot = indicator.locator("svg > circle").last();
    await expect(arc).toHaveCSS("animation-name", "steps-orbit");
    await expect(dot).toHaveCSS("animation-name", "steps-breathe");
    // Check actual motion, not just a declared keyframe name.
    const transform = await arc.evaluate(el => getComputedStyle(el).transform);
    await expect.poll(() => arc.evaluate(el => getComputedStyle(el).transform)).not.toBe(transform);
    const opacity = await dot.evaluate(el => getComputedStyle(el).opacity);
    await expect.poll(() => dot.evaluate(el => getComputedStyle(el).opacity)).not.toBe(opacity);
    await expect(head.locator('[aria-valuenow]')).toHaveCount(0);

    await emit(page, frameId, { kind: "ToolResult", name: "pdf-explore", ok: true, content: "Paper findings extracted." });
    await expect(title).toHaveText(locale === "zh" ? "等待模型继续…" : "Waiting for the model…");
    await expect(page.getByTestId("steps-wait-note")).toBeVisible();
    await head.click();
    await expect(head).toHaveAttribute("aria-expanded", "false");
    await expect(indicator).toHaveAttribute("data-state", "running");
    await head.focus();
    await page.keyboard.press("Enter");
    await expect(head).toHaveAttribute("aria-expanded", "true");
    await page.locator("#composer-input").click();

    await page.evaluate(async () => { await document.fonts.ready; });
    await page.locator(".steps").screenshot({ path: test.info().outputPath(`waiting-${locale}.png`), animations: "disabled" });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
    const bounds = await head.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
    await expect.poll(() => head.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
    await page.locator(".steps").screenshot({ path: test.info().outputPath(`waiting-${locale}-dark-narrow.png`), animations: "disabled" });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await expect(arc).toHaveCSS("animation-name", "none");
    await expect(dot).toHaveCSS("animation-name", "none");
    await expect(indicator).toBeVisible();

    await emit(page, frameId, { kind: "ToolCall", name: "python", preview: "compare_findings()" });
    await expect(title).toHaveText(locale === "zh" ? "执行中…" : "Working…");
    await expect(page.getByTestId("steps-wait-note")).toHaveCount(0);
    await emit(page, frameId, { kind: "ToolResult", name: "python", ok: true, content: "Comparison ready." });
    await page.evaluate(() => (window as any).__finishIndicatorTurn());
    await expect(indicator).toHaveAttribute("data-state", "complete");
    await expect(page.getByTestId("steps-wait-note")).toHaveCount(0);
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await expect(indicator.locator("svg > path")).toHaveCSS("animation-name", "none");
    await expect(page.locator(".steps")).toHaveClass(/activity-summary/);
    await page.locator(".steps").screenshot({ path: test.info().outputPath(`complete-${locale}.png`), animations: "disabled" });
  });
}

test("failed tools settle without a success check or a running animation", async ({ page }) => {
  const frameId = await start(page);
  await emit(page, frameId, { kind: "ToolResult", name: "pdf-explore", ok: false, content: "Could not read the PDF." });
  await page.evaluate(() => (window as any).__finishIndicatorTurn());
  const indicator = page.getByTestId("steps-indicator");
  await expect(indicator).toHaveAttribute("data-state", "stopped");
  await expect(indicator.locator("svg > path")).toHaveCSS("animation-name", "none");
  await expect(page.getByTestId("steps-wait-note")).toHaveCount(0);
});
