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
  await expect(panel.locator(".rp-tab.active")).toHaveText("Notebook");
  await panel.getByTestId("right-pane-home").click();
  await expect(launcher).toBeVisible();

  await panel.getByTestId("right-launcher-agents").click();
  await expect(panel.getByTestId("agent-workflows")).toBeVisible();
  await panel.getByTestId("right-pane-home").click();
  await panel.getByRole("button", { name: "Close panel" }).click();
  await expect(panel).toHaveCount(0);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await expect(launcher).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
});

test("direct tool links bypass the launcher without losing the way back", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files", exact: true }).click();
  const panel = page.locator(".rightpane");
  await expect(panel).toBeVisible();
  await expect(panel.getByTestId("right-pane-launcher")).toHaveCount(0);
  await expect(panel.locator(".rp-tab.active")).toHaveText("Files");

  await panel.getByTestId("right-pane-home").click();
  await expect(panel.getByTestId("right-pane-launcher")).toBeVisible();
  await panel.getByTestId("right-launcher-artifacts").click();
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
