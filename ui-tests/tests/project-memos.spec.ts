import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";
import { openSidebarEntry } from "./sidebar-nav";

// Project memos live in the right pane. They are the researcher's own notes:
// the mock agent never receives them, and Insert only fills the composer.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function openMemos(page: Page, query = "") {
  await page.goto(`/${query}`);
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator("#composer-input")).toBeVisible();
  await openSidebarEntry(page, /^(Memos|备忘录)$/);
  const pane = page.getByTestId("memos-pane");
  await expect(pane).toBeVisible();
  return pane;
}

function invokes(page: Page, cmd: string) {
  return page.evaluate((name) => ((window as any).__skillInvokeLog as any[]).filter((c) => c.cmd === name).length, cmd);
}

test("a new memo is private to the pane and Insert fills then appends to the composer", async ({ page }) => {
  const pane = await openMemos(page);
  await expect(pane.getByTestId("memos-private")).toContainText(/not visible to the AI|AI 不可见/);
  await expect(pane.getByTestId("memos-empty")).toBeVisible();
  await expect(page.locator(".rightpane .rp-tab.active")).toHaveText(/^Memos|备忘录$/);
  await expect(page.locator(".sidebar .nav .side-btn.active")).toHaveText(/Memos|备忘录/);

  await pane.getByTestId("memos-new").click();
  const editor = pane.getByTestId("memo-editor");
  await expect(editor).toBeVisible();
  await expect(pane.getByTestId("memo-save")).toBeDisabled();
  await editor.getByTestId("memo-title-input").fill("Server habits");
  await editor.getByTestId("memo-body-input").fill("ssh gpu02 first\nthen conda activate wisp");
  await editor.getByTestId("memo-body-input").press("ControlOrMeta+Enter");
  await expect(editor).toHaveCount(0);
  const card = pane.getByTestId("memo-card");
  await expect(card).toHaveCount(1);
  await expect(card.locator(".memo-title")).toHaveText("Server habits");
  await expect(pane.getByTestId("memos-empty")).toHaveCount(0);

  // Insert never sends: the composer receives the body only, the memo stays.
  const composer = page.locator("#composer-input");
  await card.hover();
  await card.getByTestId("memo-insert").click();
  await expect(composer).toHaveValue("ssh gpu02 first\nthen conda activate wisp");
  await expect(composer).toBeFocused();
  await expect(card.getByTestId("memo-insert")).toHaveText(/Inserted|已插入/);
  await expect(pane).toBeVisible();
  await card.getByTestId("memo-insert").click();
  await expect(composer).toHaveValue("ssh gpu02 first\nthen conda activate wisp\n\nssh gpu02 first\nthen conda activate wisp");
  expect(await invokes(page, "send_message")).toBe(0);
  expect(await invokes(page, "add_project_memo")).toBe(1);
});

test("Escape closes the topmost memo layer only: menu, then discard prompt, then editor", async ({ page }) => {
  const pane = await openMemos(page, "?mockMemos=seeded");
  const cards = pane.getByTestId("memo-card");
  await expect(cards).toHaveCount(2);
  // Pinned first; an untitled memo shows its first body line.
  await expect(cards.nth(0).locator(".memo-title")).toHaveText("Server habits");
  await expect(cards.nth(0).locator(".memo-pin")).toBeVisible();
  await expect(cards.nth(1).locator(".memo-title")).toHaveText("You are a careful analyst. Start from the QC table.");

  // Card menu: one Escape right after opening closes only the menu.
  await cards.nth(1).hover();
  await cards.nth(1).getByTestId("memo-more").click();
  await expect(pane.getByTestId("memo-menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(pane.getByTestId("memo-menu")).toHaveCount(0);
  await expect(pane).toBeVisible();

  // A clean editor closes on the first Escape, without moving focus into it.
  await cards.nth(1).getByTestId("memo-edit").click();
  const editor = pane.getByTestId("memo-editor");
  await expect(editor).toBeVisible();
  await expect(editor.getByTestId("memo-title-input")).toHaveValue("");
  await page.keyboard.press("Escape");
  await expect(editor).toHaveCount(0);
  await expect(pane).toBeVisible();

  // A dirty editor asks first; the next Escape returns to editing; Discard closes.
  await pane.getByTestId("memos-new").click();
  await editor.getByTestId("memo-body-input").fill("half a thought");
  await page.keyboard.press("Escape");
  await expect(pane.getByTestId("memo-discard-confirm")).toBeVisible();
  await expect(editor).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(pane.getByTestId("memo-discard-confirm")).toHaveCount(0);
  await expect(editor.getByTestId("memo-body-input")).toHaveValue("half a thought");
  await page.keyboard.press("Escape");
  await pane.getByTestId("memo-discard-confirm").getByRole("button", { name: /^(Discard|放弃)$/ }).click();
  await expect(editor).toHaveCount(0);
  await expect(cards).toHaveCount(2);
  expect(await invokes(page, "add_project_memo")).toBe(0);
});

test("pin, edit and delete update the list in place and search narrows it", async ({ page }) => {
  const pane = await openMemos(page, "?mockMemos=seeded");
  const cards = pane.getByTestId("memo-card");
  await cards.nth(1).hover();
  await cards.nth(1).getByTestId("memo-more").click();
  await pane.getByTestId("memo-menu").getByRole("menuitem", { name: /^(Pin|置顶)$/ }).click();
  await expect(cards.nth(0).locator(".memo-title")).toHaveText("You are a careful analyst. Start from the QC table.");
  await expect(cards.nth(0).locator(".memo-pin")).toBeVisible();

  await cards.nth(0).locator(".memo-heading").dblclick();
  const editor = pane.getByTestId("memo-editor");
  await editor.getByTestId("memo-title-input").fill("Opening prompt");
  await pane.getByTestId("memo-save").click();
  await expect(editor).toHaveCount(0);
  await expect(cards.nth(0).locator(".memo-title")).toHaveText("Opening prompt");

  await pane.getByTestId("memos-search").fill("gpu");
  await expect(cards).toHaveCount(1);
  await expect(cards.first().locator(".memo-title")).toHaveText("Server habits");
  await pane.getByTestId("memos-search").fill("nothing here");
  await expect(cards).toHaveCount(0);
  await expect(pane.locator(".memos-empty.muted")).toBeVisible();
  await pane.getByTestId("memos-search").fill("");

  await cards.nth(1).hover();
  await cards.nth(1).getByTestId("memo-more").click();
  await pane.getByTestId("memo-menu").getByRole("menuitem", { name: /^(Delete|删除)$/ }).click();
  await expect(cards).toHaveCount(1);
  await expect(cards.first().locator(".memo-title")).toHaveText("Opening prompt");
  expect(await invokes(page, "delete_project_memo")).toBe(1);
});

test("a failed save keeps the editor and its text, and closing the tab keeps the draft", async ({ page }) => {
  const pane = await openMemos(page);
  await pane.getByTestId("memos-new").click();
  const editor = pane.getByTestId("memo-editor");
  await editor.getByTestId("memo-body-input").fill("keep me");
  await page.evaluate(() => { (window as any).__memoSaveError = true; });
  await pane.getByTestId("memo-save").click();
  await expect(editor.getByRole("alert")).toContainText("Memo store unavailable");
  await expect(editor.getByTestId("memo-body-input")).toHaveValue("keep me");
  await expect(pane.getByTestId("memo-card")).toHaveCount(0);

  // The draft belongs to the project, not to the pane: closing and reopening
  // the Memos tab brings it back unchanged.
  await page.locator(".rightpane .rp-tab-wrap", { hasText: /Memos|备忘录/ }).locator(".rp-tab-close").click();
  await expect(pane).toHaveCount(0);
  await openSidebarEntry(page, /^(Memos|备忘录)$/);
  await expect(page.getByTestId("memo-editor").getByTestId("memo-body-input")).toHaveValue("keep me");
  await page.getByTestId("memo-save").click();
  await expect(page.getByTestId("memo-card")).toHaveCount(1);
});

test("at drawer widths Escape closes the Memos pane once nothing inside it is open", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 800 });
  const pane = await openMemos(page);
  await page.keyboard.press("Escape");
  await expect(pane).toHaveCount(0);
  await expect(page.locator("#composer-input")).toBeVisible();
});
