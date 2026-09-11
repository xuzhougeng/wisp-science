import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto("/?mockLongPages=8");
  await page.locator(".proj-card-main").first().click();
  await expect(page.getByText(/Window page 0 row 19/)).toBeVisible();
  await page.evaluate(async () => { await document.fonts.ready; });
});

async function activeMatch(page: Page) {
  return page.evaluate(() => {
    const range = [...((CSS as any).highlights.get("wisp-find-active") ?? [])][0] as Range | undefined;
    if (!range) return null;
    const viewport = document.getElementById("chat-scroller")!.getBoundingClientRect();
    const rect = range.getBoundingClientRect();
    return {
      text: range.toString(),
      body: range.startContainer.parentElement?.closest(".body")?.textContent,
      visible: rect.top >= viewport.top && rect.bottom <= viewport.bottom,
    };
  });
}

for (const modifier of ["Control", "Meta"]) {
  test(`${modifier}+F counts only conversation bodies and stays on backward results (#1212)`, async ({ page }) => {
    await page.locator("#composer-input").fill("Window page outside the transcript");
    await page.keyboard.press(`${modifier}+f`);
    const input = page.locator("#chat-find-input");
    await expect(input).toBeFocused();
    await input.fill("Window page");
    const count = page.locator(".chat-find-count");
    await expect(count).toHaveText("1 / 20");
    await expect.poll(() => activeMatch(page)).toMatchObject({ body: expect.stringContaining("row 0 "), visible: true });
    await input.press("Enter");
    await expect(count).toHaveText("2 / 20");
    await expect.poll(() => activeMatch(page)).toMatchObject({ body: expect.stringContaining("row 1 "), visible: true });
    await input.press("Shift+Enter");
    await input.press("Shift+Enter");
    await expect(count).toHaveText("20 / 20");
    await expect.poll(() => activeMatch(page)).toMatchObject({ body: expect.stringContaining("row 19 "), visible: true });
    await page.locator(".msg.assistant .body").last().evaluate(body => {
      body.appendChild(document.createTextNode(" streamed output".repeat(500)));
    });
    await page.waitForTimeout(250);
    await expect(count).toHaveText("20 / 20");
    await expect.poll(() => activeMatch(page)).toMatchObject({ body: expect.stringContaining("row 19 "), visible: true });
    await page.getByRole("button", { name: "Previous match", exact: true }).click();
    await expect(count).toHaveText("19 / 20");
    // Outlive the scroll gesture guard and pending observer/follow callbacks.
    await page.waitForTimeout(750);
    await expect.poll(() => activeMatch(page)).toMatchObject({ body: expect.stringContaining("row 18 "), visible: true });
    await page.locator(".msg.assistant .body").last().evaluate(body => {
      body.appendChild(document.createTextNode(" streamed output".repeat(500)));
    });
    await page.waitForTimeout(250);
    await expect(count).toHaveText("19 / 20");
    await expect.poll(() => activeMatch(page)).toMatchObject({ body: expect.stringContaining("row 18 "), visible: true });
    const scroller = page.locator("#chat-scroller");
    const top = await scroller.evaluate(el => el.scrollTop);
    await page.keyboard.press("Escape");
    await expect(input).toHaveCount(0);
    await expect.poll(() => scroller.evaluate(el => el.scrollTop)).toBeCloseTo(top, 0);
    await expect.poll(() => page.evaluate(() => (CSS as any).highlights.has("wisp-find-active"))).toBe(false);
    await page.locator("#chat-jump-pill").click();
    await expect.poll(() => scroller.evaluate(el => el.scrollHeight - el.clientHeight - el.scrollTop)).toBeLessThan(8);
  });
}

test("find uses literal Unicode ranges across inline formatting and refreshes changed content", async ({ page }) => {
  // Bounded rendered-DOM fixture: exercise inline splits, hidden content and
  // the same subtree replacement/append notifications used by streaming rows.
  await page.locator(".msg.assistant .body").first().evaluate(body => {
    body.innerHTML = '<p>你好 <strong>please</strong> [a.b] 你好</p><p hidden>你好</p><button>你好</button>';
  });
  await page.keyboard.press("Control+f");
  const input = page.locator("#chat-find-input");
  const count = page.locator(".chat-find-count");
  await input.fill("你好 PLEASE");
  await expect(count).toHaveText("1 / 1");
  expect((await activeMatch(page))?.text).toBe("你好 please");
  await input.fill("[a.b]");
  await expect(count).toHaveText("1 / 1");
  await input.fill("你好");
  await expect(count).toHaveText("1 / 2");
  await input.press("Enter");
  await expect(count).toHaveText("2 / 2");
  await page.locator(".msg.assistant .body").first().evaluate(body => {
    body.innerHTML = '<p>你好 <strong>please</strong> [a.b] 你好 你好</p>';
  });
  await expect(count).toHaveText("2 / 3");
  await page.getByRole("button", { name: "Next match", exact: true }).click();
  await expect(count).toHaveText("3 / 3");
  await input.fill("missing literal");
  await expect(count).toHaveText("0 / 0");
  await expect(page.getByRole("button", { name: "Next match", exact: true })).toBeDisabled();
  await input.fill("");
  await expect(count).toHaveText("0 / 0");
});

test("Escape closes only the top layer and session changes clear find", async ({ page }) => {
  await page.keyboard.press("Control+f");
  // No focus preparation before Escape.
  await page.keyboard.press("Escape");
  await expect(page.locator(".chat-find")).toHaveCount(0);
  await page.keyboard.press("Control+f");
  await page.locator("#chat-find-input").fill("Window page");
  await page.keyboard.press("Control+f");
  await expect(page.locator("#chat-find-input")).toHaveValue("Window page");
  await page.keyboard.press("Control+p");
  await expect(page.locator(".action-palette")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".action-palette")).not.toBeVisible();
  await expect(page.locator(".chat-find")).toBeVisible();
  await page.getByRole("button", { name: "New session", exact: true }).click();
  await expect(page.locator(".chat-find")).toHaveCount(0);
  expect(await page.evaluate(() => (CSS as any).highlights.has("wisp-find"))).toBe(false);
});

test("find bar fits a narrow conversation viewport", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 700 });
  await page.keyboard.press("Control+f");
  await page.locator("#chat-find-input").fill("Window page");
  const bar = page.locator(".chat-find");
  await expect(bar).toBeVisible();
  expect(await bar.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("conversation-find-narrow.png"), animations: "disabled" });
});
