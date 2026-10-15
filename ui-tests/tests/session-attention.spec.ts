import { test, expect, type Locator, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.setViewportSize({ width: 1400, height: 1000 });
});

async function openProject(page: Page, locale = "en") {
  await page.goto(`/?mockNeedsYou=1&mockLocale=${locale}`);
  await page.locator(".proj-card-main").first().click();
  await expect(page.getByTestId("sidebar-needs-you")).toHaveText(
    locale === "zh" ? "4 个待处理" : "4 need you",
  );
}

async function expectLabelInsideRow(row: Locator) {
  const label = row.locator(".ses-status-label");
  await expect(label).toBeVisible();
  const geometry = await row.evaluate((element) => {
    const bounds = element.getBoundingClientRect();
    const label = element.querySelector<HTMLElement>(".ses-status-label")!;
    const title = element.querySelector<HTMLElement>(".ses-title")!;
    return {
      labelFits: label.getBoundingClientRect().right <= bounds.right - 4,
      labelUnclipped: label.scrollWidth <= label.clientWidth,
      titleDoesNotOverlap: title.getBoundingClientRect().right <= label.getBoundingClientRect().left,
    };
  });
  expect(geometry).toEqual({ labelFits: true, labelUnclipped: true, titleDoesNotOverlap: true });
}

for (const locale of ["en", "zh"]) {
  test(`project and sidebar show separate labeled activity at larger text sizes (${locale})`, async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem("sidebarWidth", "240"));
    await page.goto(`/?mockNeedsYou=1&mockLocale=${locale}`);
    const card = page.locator(".proj-card").filter({ has: page.locator(".proj-card-main") }).first();
    await expect(card.locator(".sess-status-needs-you")).toHaveText(locale === "zh" ? "4 个待处理" : "4 need you");
    await expect(card.locator(".sess-status-running")).toHaveText(locale === "zh" ? "1 个运行中" : "1 running");
    await expect(card.locator(".pc-dot")).toHaveCount(0);
    await expect(card.locator(".sess-status-needs-you")).toHaveAttribute("title", /未读回复|Unread replies/);
    await card.locator(".proj-card-main").click();
    const unread = page.locator('.sidebar .side-item.ses[data-session-id="session-002"]');
    const running = page.locator('.sidebar .side-item.ses[data-session-id="session-003"]');
    const awaiting = page.locator('.sidebar .side-item.ses[data-session-id="session-004"]');
    await expect(unread.locator(".ses-status-label")).toHaveText(locale === "zh" ? "待处理" : "Needs you");
    await expect(running.locator(".ses-status-label")).toHaveText(locale === "zh" ? "运行中" : "Running");
    await expect(awaiting.locator(".ses-attention")).toBeVisible();
    await expect(awaiting.locator(".ses-live")).toBeHidden();
    await page.evaluate(() => document.documentElement.style.setProperty("--ui-font-scale", "1.5"));
    for (const row of [unread, running, awaiting]) await expectLabelInsideRow(row);
    await expect(unread).toHaveAttribute("title", /Unread report in a collapsed group with a long conversation title/);
  });
}

test("Needs you finds folded and older conversations and clears only viewed replies", async ({ page }) => {
  await openProject(page);
  const folder = page.locator('.side-folder[data-folder-id="pending-folder"]');
  await folder.click();
  await expect(page.locator('.side-item.ses[data-session-id="session-002"]')).toBeHidden();
  const family = page.locator(".side-nest-group").filter({
    has: page.locator('.side-item.ses[data-session-id="session-001"]'),
  });
  await family.getByTestId("sidebar-nest-toggle").click();
  await expect(family).toHaveClass(/collapsed/);
  await expect(page.locator('.side-item.ses[data-session-id="session-101"]')).toHaveCount(0);

  await page.getByTestId("sidebar-needs-you").click();
  const pending = page.getByTestId("sidebar-attention-session");
  await expect(pending).toHaveCount(4);
  await expect(pending.filter({ hasText: "Unread report in a collapsed group" })).toBeVisible();
  await expect(pending.filter({ hasText: "Unread branch report" })).toBeVisible();
  await expect(pending.filter({ hasText: "Cross-project counts" })).toHaveCount(0);
  await expect(page.locator(".side-load-more")).toHaveCount(0);
  await pending.filter({ hasText: "Paged session 101" }).click();
  await expect(page.locator(".msg.assistant")).toContainText("Saved reply for session-101");
  await expect(page.getByTestId("sidebar-needs-you")).toHaveText("3 need you");
  await expect(pending).toHaveCount(3);

  // Viewing a conversation still waiting for approval does not resolve it.
  await pending.filter({ hasText: "Paged session 4" }).click();
  await expect(page.locator(".msg.assistant")).toContainText("Saved reply for session-004");
  await expect(page.getByTestId("sidebar-needs-you")).toHaveText("3 need you");
  await expect(pending.filter({ hasText: "Paged session 4" })).toBeVisible();
  await page.getByTestId("sidebar-all-sessions").click();
  await expect(page.locator('.side-item.ses[data-session-id="session-002"]')).toBeHidden();
  await expect(family).toHaveClass(/collapsed/);
  await expect(page.locator(".side-load-more")).toBeVisible();
});

test("a late inbox snapshot cannot restore a cleared pending marker", async ({ page }) => {
  await openProject(page);
  await page.evaluate(() => { (window as any).__holdNextNeedsYouRead = true; });
  await page.locator(".inbox-wrap > .icon-btn").click();
  await expect.poll(() => page.evaluate(() => typeof (window as any).__releaseNeedsYouRead)).toBe("function");
  // Escape must work immediately, without focusing inside the existing inbox.
  await page.keyboard.press("Escape");
  await expect(page.locator(".inbox-drop")).toHaveCount(0);
  await page.getByTestId("sidebar-needs-you").click();
  await page.getByTestId("sidebar-attention-session").filter({ hasText: "Paged session 101" }).click();
  await expect(page.getByTestId("sidebar-needs-you")).toHaveText("3 need you");
  await page.evaluate(async () => {
    (window as any).__releaseNeedsYouRead();
    // Flush the resumed invoke and Leptos's scheduled render before asserting.
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
  });
  await expect.poll(() => page.evaluate(() => (window as any).__heldNeedsYouReadReturned)).toBe(true);
  await expect(page.getByTestId("sidebar-needs-you")).toHaveText("3 need you");
  await expect(page.getByTestId("sidebar-attention-session").filter({ hasText: "Paged session 101" })).toHaveCount(0);
  await expect(page.locator(".inbox-wrap .inbox-badge")).toHaveText("3");
});
