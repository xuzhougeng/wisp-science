import { test, expect } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.describe("Files modification timestamps", () => {
  test.use({ timezoneId: "UTC" });

  for (const { scenario, now, expectedModified } of [
    { scenario: "same day", now: "2026-09-12T12:00:30Z", expectedModified: "11:59" },
    { scenario: "across midnight", now: "2026-09-12T00:00:30Z", expectedModified: "09-11 23:59" },
  ]) {
    test(`Files sorts by size and modified time and Escape closes only the sort menu (${scenario})`, async ({ page }) => {
      // The mock file is 90 seconds old. Freeze Date before navigation so both
      // the fixture and UI use the same clock, including at the UTC day boundary.
      // Register the clock before the mock init script captures FILE_NOW.
      await page.clock.setFixedTime(new Date(now));
      await page.addInitScript(tauriMock);
      await page.goto("/");
      expect(await page.evaluate(() => Date.now())).toBe(Date.parse(now));
      await page.locator(".proj-card-main").first().click();
      await expect(page.locator(".sidebar").getByRole("button", { name: "New session" })).toBeVisible();
      await page.getByRole("button", { name: "Files" }).click();
      const files = page.locator(".rp-files");
      const fileRows = files.locator(".fb-row:not(.dir)");

      await expect(files.locator('.fb-row[data-workspace-path="report.csv"] .fb-size')).toHaveText("4.0 KB");

      await files.getByTestId("files-sort").click();
      const sortMenu = files.locator(".fb-sort-menu");
      await expect(sortMenu).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(sortMenu).toHaveCount(0);
      await expect(files).toBeVisible();

      await files.getByTestId("files-sort").click();
      await sortMenu.getByRole("menuitem", { name: "Size" }).click();
      await expect(fileRows.first()).toHaveAttribute("data-workspace-path", "manuscript.docx");
      await expect(fileRows.first().locator(".fb-size")).toHaveText("11.1 KB");

      await files.getByTestId("files-sort").click();
      await sortMenu.getByRole("menuitem", { name: "Modified" }).click();
      await expect(fileRows.first()).toHaveAttribute("data-workspace-path", "report.csv");
      await expect(fileRows.first().locator(".fb-size")).toHaveText(expectedModified);
      await expect(files.locator('.fb-row.dir[data-workspace-path="DEG"] .fb-size')).toHaveText(/\d/);
    });
  }
});
