import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

const sidebarRow = (page: Page, id = "session-001") => page.locator(`.sidebar .ses[data-session-id='${id}']`);
const shelvedDialog = (page: Page) => page.getByRole("dialog", { name: "Shelved conversations", exact: true });

async function enter(page: Page, extra = "") {
  await page.addInitScript(tauriMock);
  await page.goto(`/?mockManySessions=1${extra}`);
  await page.locator(".proj-card-main").first().click();
}
async function shelve(page: Page) {
  await sidebarRow(page).click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Shelve conversation", exact: true }).click();
  await expect(sidebarRow(page)).toHaveCount(0);
}
async function openShelved(page: Page) {
  await page.getByRole("button", { name: "Shelved conversations", exact: true }).click();
  await expect(shelvedDialog(page)).toBeVisible();
}

test("right-click shelving persists, excludes search and #, and restores the active conversation", async ({ page }) => {
  await enter(page);
  await sidebarRow(page).click();
  await shelve(page);
  await expect(page.locator(".center-tabs > .center-tab")).toContainText("Paged session 1");
  await page.keyboard.press("Control+k");
  await page.locator("#command-palette-input").fill("Paged session 1");
  await expect(page.locator(".project-search-title").getByText("Paged session 1", { exact: true })).toHaveCount(0);
  await expect(page.locator(".project-search-row", { hasText: "Paged session 10" }).first()).toBeVisible();
  await page.keyboard.press("Escape");
  await page.locator("#composer-input").pressSequentially("#Paged");
  await expect(page.locator(".mention-menu")).toBeVisible();
  await expect(page.locator(".mention-menu").getByText("Paged session 2", { exact: true })).toBeVisible();
  await expect(page.locator(".mention-menu").getByText("Paged session 1", { exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.reload();
  await page.locator(".proj-card-main").first().click();
  await expect(sidebarRow(page, "session-002")).toBeVisible();
  await expect(sidebarRow(page)).toHaveCount(0);
  await openShelved(page);
  const row = shelvedDialog(page).locator(".shelved-session-row");
  await expect(row).toHaveCount(1);
  await row.getByRole("button", { name: "Paged session 1", exact: true }).click();
  await expect(shelvedDialog(page)).toHaveCount(0);
  await expect(page.locator(".center-tabs > .center-tab")).toContainText("Paged session 1");
  await expect(sidebarRow(page)).toHaveCount(0);
  await openShelved(page);
  await shelvedDialog(page).getByRole("button", { name: "Restore to main list", exact: true }).click();
  await expect(sidebarRow(page)).toBeVisible();
  await expect(shelvedDialog(page).locator(".shelved-session-row")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.keyboard.press("Control+k");
  await page.locator("#command-palette-input").fill("Paged session 1");
  await expect(page.locator(".project-search-title").getByText("Paged session 1", { exact: true })).toBeVisible();
});

test("Escape closes only the topmost surface without preparing focus", async ({ page }) => {
  await enter(page);
  await shelve(page);
  await openShelved(page);
  await page.keyboard.press("Escape");
  await expect(shelvedDialog(page)).toHaveCount(0);
  await openShelved(page);
  await shelvedDialog(page).locator(".shelved-session-row").click({ button: "right" });
  await expect(page.locator(".ctx-menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".ctx-menu")).toHaveCount(0);
  await expect(shelvedDialog(page)).toBeVisible();
  await page.keyboard.press("Control+p");
  await expect(page.locator(".action-palette")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".action-palette")).toHaveCount(0);
  await expect(shelvedDialog(page)).toBeVisible();
  await shelvedDialog(page).locator(".shelved-session-row").click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Restore to main list", exact: true }).click();
  await expect(sidebarRow(page)).toBeVisible();
});

test("shelved collection searches beyond its first page and loads more", async ({ page }) => {
  await enter(page, "&mockShelvedPages=1");
  await openShelved(page);
  await expect(shelvedDialog(page).locator(".shelved-session-row")).toHaveCount(100);
  await shelvedDialog(page).getByRole("button", { name: "Load earlier sessions" }).click();
  await expect(shelvedDialog(page).locator(".shelved-session-row")).toHaveCount(101);
  await shelvedDialog(page).getByRole("searchbox").fill("Paged session 101");
  await expect(shelvedDialog(page).locator(".shelved-session-row")).toHaveCount(1);
  await expect(shelvedDialog(page).getByRole("button", { name: "Paged session 101", exact: true })).toBeVisible();
  await shelvedDialog(page).getByRole("searchbox").fill("no match");
  await expect(shelvedDialog(page).getByText("No shelved conversations found.")).toBeVisible();
});

test("failed shelving keeps the conversation and reports the error", async ({ page }) => {
  await enter(page);
  await page.evaluate(() => { (window as any).__failShelve = true; });
  await sidebarRow(page).click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Shelve conversation", exact: true }).click();
  await expect(page.getByText("Could not save conversation visibility", { exact: true })).toBeVisible();
  await expect(sidebarRow(page)).toBeVisible();
});

test("Chinese labels distinguish shelving from research archiving", async ({ page }) => {
  await enter(page, "&mockLocale=zh");
  await sidebarRow(page).click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "收起对话", exact: true }).click();
  await expect(sidebarRow(page)).toHaveCount(0);
  await page.getByRole("button", { name: "已收起的对话", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "已收起的对话", exact: true });
  await expect(dialog.getByRole("searchbox", { name: "搜索已收起的对话" })).toBeVisible();
  await dialog.getByRole("button", { name: "恢复到主列表", exact: true }).click();
  await expect(sidebarRow(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});

test("switching projects closes the old project's shelved collection", async ({ page }) => {
  await enter(page);
  await openShelved(page);
  await page.keyboard.press("Control+k");
  await page.locator("#command-palette-input").fill("Other project");
  await page.locator(".project-search-row").filter({ hasText: "Other project" }).click();
  await expect(shelvedDialog(page)).toHaveCount(0);
});

test("shelving the active branch preserves its title and branch controls", async ({ page }) => {
  await enter(page, "&mockBranches=1");
  const branch = sidebarRow(page, "conversation-branch");
  await branch.click();
  await expect(page.locator(".center-tabs > .center-tab")).toContainText("Branch: alternate analysis");
  await branch.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Shelve conversation", exact: true }).click();
  await expect(branch).toHaveCount(0);
  await expect(page.locator(".center-tabs > .center-tab")).toContainText("Branch: alternate analysis");
  await page.locator(".send-menu-toggle").click();
  await expect(page.locator(".send-mode-menu")).toBeVisible();
  await expect(page.getByRole("button", { name: "Branch in new session", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await sidebarRow(page, "conversation-main").click();
  await openShelved(page);
  await shelvedDialog(page).getByRole("button", { name: "Branch: alternate analysis", exact: true }).click();
  await expect(page.locator(".center-tabs > .center-tab")).toContainText("Branch: alternate analysis");
  await page.locator(".send-menu-toggle").click();
  await expect(page.getByRole("button", { name: "Branch in new session", exact: true })).toHaveCount(0);
});
