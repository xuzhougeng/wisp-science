import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { tauriMock } from "./mock-tauri";

// Synthetic variant of office-preview.xlsx: C2 references an external workbook
// with a cached value of 84. No user workbook or private paths are included.
test("XLSX external references display cached cells without fetching the workbook", async ({ page }) => {
  await page.addInitScript(tauriMock, {
    xlsxBase64: readFileSync(resolve(__dirname, "../fixtures/office-external-reference.xlsx")).toString("base64"),
  });
  const externalRequests: string[] = [];
  await page.context().route("https://example.invalid/**", async (route) => {
    externalRequests.push(route.request().url());
    await route.abort();
  });
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('.fb-row[data-workspace-path="office-preview.xlsx"]').click();
  const workbook = page.locator(".artifact-modal .rp-xlsx");
  await expect(workbook).toContainText("FOXA2");
  await workbook.locator(".rp-xlsx-cell", { hasText: "84" }).click();
  await expect(workbook.locator(".rp-xlsx-formula-value")).toHaveText("='[1]Sheet1'!A1");
  expect(externalRequests).toEqual([]);
});
