import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// `/memo` turns the composer's `/` menu into a picker over the researcher's
// private memos. Selecting one only pastes its body; nothing is sent.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function openProject(page: Page, query = "") {
  await page.goto(`/${query}`);
  await page.locator(".proj-card-main").first().click();
  const composer = page.locator("#composer-input");
  await expect(composer).toBeVisible();
  await composer.click();
  return composer;
}

function sent(page: Page) {
  return page.evaluate(() => ((window as any).__skillInvokeLog as any[]).filter((c) => c.cmd === "send_message").length);
}

test("/memo lists memos, filters as you type and pastes the chosen body without sending", async ({ page }) => {
  const composer = await openProject(page, "?mockMemos=seeded");
  const menu = page.locator(".mention-menu");
  await composer.pressSequentially("/mem");
  await expect(menu.locator(".mention-item.active .mention-item-name")).toHaveText("/memo");
  await composer.press("Enter");
  await expect(composer).toHaveValue("/memo ");
  await expect(menu.locator(".mention-group-label")).toHaveText(/Memos · local only|备忘录 · 仅本机/);
  const rows = menu.locator(".mention-item");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).locator(".mention-item-name")).toHaveText("Server habits");
  await expect(rows.nth(1).locator(".mention-item-name")).toHaveText("You are a careful analyst. Start from the QC table.");

  await composer.pressSequentially("qc");
  await expect(composer).toHaveValue("/memo qc");
  await expect(rows).toHaveCount(1);
  await composer.press("Enter");
  await expect(menu).toHaveCount(0);
  await expect(composer).toHaveValue("You are a careful analyst. Start from the QC table.");
  await expect(composer).toBeFocused();
  expect(await sent(page)).toBe(0);
});

test("a typed /memo appends after the draft, and Escape removes only the token", async ({ page }) => {
  const composer = await openProject(page, "?mockMemos=seeded");
  const menu = page.locator(".mention-menu");
  await composer.pressSequentially("Compare methods /memo ");
  await expect(menu.locator(".mention-item")).toHaveCount(2);
  // Escape right after opening: the picker closes, the draft stays, the token goes.
  await composer.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(composer).toHaveValue("Compare methods ");
  await expect(composer).toBeFocused();

  await composer.pressSequentially("/memo gpu");
  await expect(menu.locator(".mention-item")).toHaveCount(1);
  await composer.press("Enter");
  await expect(composer).toHaveValue("Compare methods\n\nssh gpu02 first\nthen conda activate wisp");
  expect(await sent(page)).toBe(0);
});

test("/memo sent as a message opens the picker instead of reaching the model", async ({ page }) => {
  const composer = await openProject(page, "?mockMemos=seeded");
  // `fill` bypasses the `/` menu, so Enter takes the send path.
  await composer.fill("/memo");
  await composer.press("Enter");
  const menu = page.locator(".mention-menu");
  await expect(menu.locator(".mention-item")).toHaveCount(2);
  await expect(composer).toHaveValue("/memo ");
  expect(await sent(page)).toBe(0);
});

test("without memos /memo offers to open the Memos pane", async ({ page }) => {
  const composer = await openProject(page);
  const menu = page.locator(".mention-menu");
  await composer.pressSequentially("/memo ");
  const row = menu.locator(".mention-item");
  await expect(row).toHaveCount(1);
  await expect(row.locator(".mention-item-name")).toHaveText(/no memos yet|还没有备忘录/);
  await composer.press("Enter");
  await expect(menu).toHaveCount(0);
  await expect(page.getByTestId("memos-pane")).toBeVisible();
  await expect(composer).toHaveValue("");
  expect(await sent(page)).toBe(0);
});
