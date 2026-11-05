import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

async function enterApp(page: Page, locale = "en") {
  await page.addInitScript(tauriMock);
  await page.goto(`/?mockLocale=${locale}`);
  await page.locator(".proj-card-main").first().click();
}

test("the right pane opens a grouped tool home and returns to it from any panel", async ({ page }, testInfo) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  const panel = page.locator(".rightpane");
  const launcher = panel.getByTestId("right-pane-launcher");

  await expect(launcher).toBeVisible();
  await expect(panel.locator(".rp-tabs")).toHaveCount(0);
  for (const id of ["files", "agents", "artifacts", "hosts"]) {
    await expect(panel.getByTestId(`right-launcher-${id}`)).toBeVisible();
  }
  await expect(panel.getByTestId("right-launcher-notebook")).toBeHidden();
  const more = panel.getByTestId("right-launcher-more");
  await more.locator("summary").click();
  await expect(more).toHaveAttribute("open", "");
  for (const id of ["runs", "notebook", "highlights", "provenance", "side-chat", "memos"]) {
    await expect(panel.getByTestId(`right-launcher-${id}`)).toBeVisible();
  }
  await page.screenshot({ path: testInfo.outputPath("right-pane-tool-home.png"), animations: "disabled" });

  await panel.getByTestId("right-launcher-notebook").click();
  await expect(launcher).toHaveCount(0);
  await expect(panel.locator(".rp-tab")).toHaveCount(1);
  await expect(panel.locator(".rp-tab.active")).toHaveText("Notebook");
  await panel.getByTestId("right-pane-home").click();
  await expect(launcher).toBeVisible();

  await panel.getByTestId("right-launcher-agents").click();
  await expect(panel.locator(".rp-tab")).toHaveCount(2);
  await expect(panel.getByTestId("agent-workflows")).toBeVisible();
  await panel.getByTestId("right-pane-home").click();
  await panel.getByRole("button", { name: "Close panel" }).click();
  await expect(panel).toHaveCount(0);
  // Tabs are still open, so reopening returns to them instead of the home page.
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await expect(launcher).toHaveCount(0);
  await expect(panel.locator(".rp-tab")).toHaveText(["Notebook", "Agents"]);
  await expect(panel.locator(".rp-tab.active")).toHaveText("Agents");
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
});

test("reopening the panel restores only the tools the user opened", async ({ page }) => {
  await enterApp(page);
  const toggle = page.getByRole("button", { name: "Toggle panel" });
  const panel = page.locator(".rightpane");
  await toggle.click();
  await panel.getByTestId("right-launcher-agents").click();
  await expect(panel.locator(".rp-tab")).toHaveText(["Agents"]);

  await toggle.click();
  await expect(panel).toHaveCount(0);
  await toggle.click();
  await expect(panel.getByTestId("right-pane-launcher")).toHaveCount(0);
  await expect(panel.locator(".rp-tab")).toHaveText(["Agents"]);
  await expect(panel.getByTestId("agent-workflows")).toBeVisible();

  await panel.getByTestId("right-pane-home").click();
  await panel.getByTestId("right-launcher-hosts").click();
  await expect(panel.locator(".rp-tab")).toHaveText(["Agents", "Environment"]);
  await panel.getByRole("button", { name: "Close panel" }).click();
  await toggle.click();
  await expect(panel.locator(".rp-tab")).toHaveText(["Agents", "Environment"]);
  await expect(panel.locator(".rp-tab.active")).toHaveText("Environment");
});

test("direct tool links bypass the launcher without losing the way back", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files", exact: true }).click();
  const panel = page.locator(".rightpane");
  await expect(panel).toBeVisible();
  await expect(panel.getByTestId("right-pane-launcher")).toHaveCount(0);
  await expect(panel.locator(".rp-tab")).toHaveCount(1);
  await expect(panel.locator(".rp-tab.active")).toHaveText("Files");

  await panel.getByTestId("right-pane-home").click();
  await expect(panel.getByTestId("right-pane-launcher")).toBeVisible();
  await panel.getByTestId("right-launcher-artifacts").click();
  await expect(panel.locator(".rp-tab")).toHaveCount(2);
  await expect(panel.locator(".rp-tab.active")).toHaveText("Artifacts");
});

test("reopening after the last tab closes does not restore default tabs", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  const panel = page.locator(".rightpane");
  await panel.getByTestId("right-launcher-hosts").click();
  await expect(panel.locator(".rp-tab")).toHaveCount(1);
  await panel.locator(".rp-tab-close").click();
  await expect(panel).toHaveCount(0);

  await page.getByRole("button", { name: "Toggle panel" }).click();
  await expect(panel.getByTestId("right-pane-launcher")).toBeVisible();
  await panel.getByTestId("right-launcher-artifacts").click();
  await expect(panel.locator(".rp-tab")).toHaveCount(1);
  await expect(panel.locator(".rp-tab.active")).toHaveText("Artifacts");
});

test("Chinese tool home fits a narrow drawer", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 640, height: 900 });
  await enterApp(page, "zh");
  await page.getByRole("button", { name: "切换面板" }).click();
  const panel = page.locator(".rightpane");
  await expect(panel.getByTestId("right-pane-launcher")).toContainText("工作区工具");
  await expect(panel.getByTestId("right-launcher-files")).toContainText("工作区文件");
  await panel.getByTestId("right-launcher-more").locator("summary").click();
  await expect(panel.getByTestId("right-launcher-runs")).toBeVisible();
  expect(await panel.evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("right-pane-tool-home-zh.png"), animations: "disabled" });
});

test("each conversation keeps its own right pane and a new one starts closed", async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto("/?mockSessionModels=1");
  await page.locator(".proj-card-main").first().click();
  const toggle = page.getByRole("button", { name: "Toggle panel" });
  const panel = page.locator(".rightpane");
  const conversationA = page.locator('[data-session-id="s-model-a"]');
  const conversationB = page.locator('[data-session-id="s-model-b"]');

  await conversationA.click();
  await toggle.click();
  await panel.getByTestId("right-launcher-agents").click();
  await expect(panel.locator(".rp-tab")).toHaveText(["Agents"]);

  await page.getByRole("button", { name: "New session", exact: true }).click();
  await expect(panel).toHaveCount(0);

  await conversationB.click();
  await expect(panel).toHaveCount(0);
  await toggle.click();
  await expect(panel.getByTestId("right-pane-launcher")).toBeVisible();
  await panel.getByTestId("right-launcher-hosts").click();
  await expect(panel.locator(".rp-tab")).toHaveText(["Environment"]);

  await conversationA.click();
  await expect(panel.locator(".rp-tab")).toHaveText(["Agents"]);
  await expect(panel.locator(".rp-tab.active")).toHaveText("Agents");
  await toggle.click();
  await expect(panel).toHaveCount(0);

  await conversationB.click();
  await expect(panel.locator(".rp-tab")).toHaveText(["Environment"]);
  await conversationA.click();
  await expect(panel).toHaveCount(0);
});
