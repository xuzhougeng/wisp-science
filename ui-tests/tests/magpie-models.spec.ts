import { expect, test, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// While Magpie is connected it runs every built-in HTTP turn (src-tauri
// magpie.rs), so no picker may offer a personal profile: picking one would
// change nothing. ACP agents keep their own models and stay available.
const MAGPIE = "Magpie · fixture/reasoner";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function enterProject(page: Page, query = "?mockMagpie=1") {
  await page.goto(`/${query}`);
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator("#composer-input")).toBeVisible();
}

async function openAgentMenu(page: Page) {
  await page.getByRole("button", { name: "Agent options" }).click();
  return page.getByRole("menu", { name: "Agent options" });
}

function invokeCount(page: Page, cmd: string) {
  return page.evaluate((name) =>
    ((window as any).__skillInvokeLog ?? []).filter((call: any) => call.cmd === name).length,
  cmd);
}

test("the composer picker offers only the Magpie model besides ACP agents", async ({ page }) => {
  await enterProject(page, "?mockSessionModels=1&mockComposerCatalog=1&mockMagpie=1");
  // This conversation is bound to a personal profile; Magpie still runs it.
  await page.locator('[data-session-id="s-model-a"]').click();
  const trigger = page.getByTestId("composer-model-trigger");
  await expect(trigger).toHaveText(MAGPIE);
  // A Magpie turn sends no service tier, so Fast is not offered.
  await expect(page.getByTestId("composer-fast-toggle")).toHaveCount(0);

  await trigger.click();
  const menu = page.locator(".model-menu");
  const magpie = menu.getByTestId("composer-model-magpie");
  await expect(magpie).toHaveClass(/active/);
  await expect(magpie).toContainText(MAGPIE);
  await expect(magpie).toContainText("Switch models in Magpie");
  await expect(menu.getByText("deepseek-v4-pro")).toHaveCount(0);
  await expect(menu.getByText("opus-4.8")).toHaveCount(0);
  await expect(menu.getByRole("button", { name: /Test ACP Agent/ })).toBeVisible();

  await magpie.getByRole("button").click();
  await expect(menu).toHaveCount(0);
  expect(await invokeCount(page, "set_active_model")).toBe(0);
});

test("without Magpie the composer picker still lists personal profiles", async ({ page }) => {
  await enterProject(page, "?mockSessionModels=1&mockComposerCatalog=1");
  await page.getByTestId("composer-model-trigger").click();
  const menu = page.locator(".model-menu");
  await expect(menu.getByRole("button", { name: "opus-4.8" })).toBeVisible();
  await expect(menu.getByTestId("composer-model-magpie")).toHaveCount(0);
});

test("the Agent menu runs the reviewer and subagents on the Magpie model", async ({ page }) => {
  await enterProject(page);
  let menu = await openAgentMenu(page);
  await expect(menu.getByRole("button", { name: /^Reviewer model/ })).toContainText(MAGPIE);
  await expect(menu.getByTestId("agent-menu-subagent-model")).toContainText(MAGPIE);

  await menu.getByRole("button", { name: /^Reviewer model/ }).click();
  const reviewer = page.getByRole("menu", { name: "Reviewer model" });
  await expect(reviewer.locator(".agent-submenu-row")).toHaveText([
    MAGPIE,
    "Follow session backend",
    "Test ACP Agent · ACP",
  ]);
  // Re-picking Magpie keeps the stored personal reviewer for after a disconnect.
  await reviewer.getByRole("button", { name: MAGPIE }).click();
  await expect(menu).toHaveCount(0);
  expect(await invokeCount(page, "save_specialist_cmd")).toBe(0);

  menu = await openAgentMenu(page);
  await menu.getByTestId("agent-menu-subagent-model").click();
  const subagent = page.getByRole("menu", { name: "Subagent model" });
  await expect(subagent.locator(".agent-submenu-row")).toHaveText([MAGPIE]);
  await subagent.getByRole("button", { name: MAGPIE }).click();
  await expect(menu).toHaveCount(0);
  expect(await invokeCount(page, "set_session_subagent_model")).toBe(0);
});

test("side chat offers only the Magpie model besides ACP agents", async ({ page }) => {
  await enterProject(page);
  await page.locator("#composer-input").fill("/btw what did the main thread miss?");
  await page.locator("#composer-input").press("Enter");
  const panel = page.locator(".rightpane");
  const trigger = panel.getByTestId("sidechat-model-trigger");
  await expect(trigger).toHaveText(MAGPIE);

  await trigger.click();
  const menu = panel.locator(".sidechat-model-menu");
  await expect(menu.getByTestId("sidechat-model-magpie")).toHaveClass(/active/);
  await expect(menu.getByRole("button", { name: "deepseek-v4-pro" })).toHaveCount(0);
  await expect(menu.getByRole("button", { name: "opus-4.8" })).toHaveCount(0);
  await expect(menu.getByRole("button", { name: "Test ACP Agent" })).toBeVisible();

  await menu.getByTestId("sidechat-model-magpie").click();
  await expect(menu).toHaveCount(0);
  expect(await invokeCount(page, "set_active_model")).toBe(0);
});

test("Settings says personal models apply again after a Magpie disconnect", async ({ page }) => {
  await page.goto("/?mockMagpie=1");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByTestId("settings-nav-models").click();
  const notice = page.getByTestId("models-magpie-notice");
  await expect(notice).toContainText(MAGPIE);
  await expect(notice).toContainText("magpie wisp-science default");
  // The profiles themselves stay editable.
  await expect(page.locator(".settings-list-row").filter({ hasText: "opus-4.8" })).toBeVisible();
});

test("Settings shows no Magpie notice when Magpie is not connected", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByTestId("settings-nav-models").click();
  await expect(page.locator(".settings-list-row").filter({ hasText: "opus-4.8" })).toBeVisible();
  await expect(page.getByTestId("models-magpie-notice")).toHaveCount(0);
});
