import { test, expect, type Page } from "@playwright/test";
import { parallelMock } from "./mock-tauri";

async function enter(page: Page, query = "") {
  await page.addInitScript(parallelMock);
  await page.goto(`/${query}`);
  await page.locator(".proj-card-main").first().click();
  await page.locator("#composer-input").fill("actions-files");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.locator(".side-item.ses", { hasText: "actions-files" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Stop", exact: true })).toHaveCount(0);
}
async function openAction(page: Page, name: string) {
  await page.locator(".side-item.ses", { hasText: "actions-files" }).click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name, exact: true }).click();
}
async function argsFor(page: Page, command: string) {
  return page.evaluate(command => {
    const calls = ((window as any).__sendInvokeLog ?? []).filter((call: any) => call.cmd === command);
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : args;
  }, command);
}

test("delete files requires opt-in, shows retained inputs, and resets after dismissal", async ({ page }) => {
  await enter(page);
  await openAction(page, "Delete");
  let modal = page.locator(".confirm-modal");
  await expect(modal.getByRole("checkbox")).not.toBeChecked();
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  expect(await argsFor(page, "delete_session")).toBeUndefined();
  await openAction(page, "Delete");
  modal = page.locator(".confirm-modal");
  await modal.getByRole("checkbox").check();
  await expect(modal).toContainText("1 artifacts · 2 files to process");
  await expect(modal).toContainText("uploads/input.csv");
  await expect(modal).toContainText("shared or referenced");
  await page.keyboard.press("Escape");
  await openAction(page, "Delete");
  await expect(page.locator(".confirm-modal").getByRole("checkbox")).not.toBeChecked();
  await page.locator(".confirm-modal").getByRole("button", { name: "Delete", exact: true }).click();
  await expect.poll(() => argsFor(page, "delete_session")).toMatchObject({ includeArtifacts: false });
});

test("delete submits the reviewed fingerprint only after preview completes", async ({ page }) => {
  await enter(page, "?mockSessionArtifacts=slow");
  await openAction(page, "Delete");
  const modal = page.locator(".confirm-modal");
  await modal.getByRole("checkbox").check();
  await expect(modal.getByRole("button", { name: "Delete", exact: true })).toBeDisabled();
  await expect(modal).toContainText("1 artifacts · 2 files to process");
  await modal.getByRole("button", { name: "Delete", exact: true }).click();
  await expect.poll(() => argsFor(page, "delete_session")).toMatchObject({ includeArtifacts: true, artifactFingerprint: expect.stringMatching(/:delete$/) });
  await expect(page.locator(".side-item.ses", { hasText: "actions-files" })).toHaveCount(0);
});

test("move binds the preview to its target and invalidates it when the target changes", async ({ page }) => {
  await enter(page);
  await openAction(page, "Move to another project…");
  const modal = page.locator(".session-transfer-modal");
  await expect(modal.getByRole("checkbox")).not.toBeChecked();
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  await openAction(page, "Move to another project…");
  await modal.getByRole("checkbox").check();
  await expect(modal).toContainText("1 artifacts · 2 files to process");
  await modal.getByLabel("Target project").selectOption("archive");
  await expect(modal.getByRole("checkbox")).not.toBeChecked();
  await modal.getByRole("checkbox").check();
  await expect(modal).toContainText("1 artifacts · 2 files to process");
  await modal.screenshot({ path: test.info().outputPath("move-artifacts.png") });
  await modal.getByRole("button", { name: "Move", exact: true }).click();
  await expect.poll(() => argsFor(page, "transfer_session_to_project")).toMatchObject({ includeArtifacts: true, targetProjectId: "archive", artifactFingerprint: expect.stringMatching(/:archive$/) });
});

test("a failed preview prevents file operations and allows transcript-only move", async ({ page }) => {
  await enter(page, "?mockSessionArtifacts=error");
  await openAction(page, "Move to another project…");
  const modal = page.locator(".session-transfer-modal");
  await modal.getByRole("checkbox").check();
  await expect(modal.getByRole("alert")).toContainText("Target file already exists");
  await expect(modal.getByRole("button", { name: "Move", exact: true })).toBeDisabled();
  await modal.getByRole("checkbox").uncheck();
  await modal.getByRole("button", { name: "Move", exact: true }).click();
  await expect.poll(() => argsFor(page, "transfer_session_to_project")).toMatchObject({ includeArtifacts: false });
});

test("Escape closes the topmost palette while the artifact confirmation stays open", async ({ page }) => {
  await enter(page);
  await openAction(page, "Delete");
  const modal = page.locator(".confirm-modal");
  await page.keyboard.press("Control+p");
  await expect(page.locator(".action-palette")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".action-palette")).toHaveCount(0);
  await expect(modal).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
});


test("many artifacts remain scrollable with confirmation actions visible in a small window", async ({ page }) => {
  await enter(page, "?mockSessionArtifacts=many");
  await openAction(page, "Delete");
  const modal = page.locator(".confirm-modal");
  await modal.getByRole("checkbox").check();
  await expect(modal).toContainText("120 files to process");
  await page.setViewportSize({ width: 390, height: 560 });
  await expect.poll(() => modal.locator(".session-artifact-files").first().evaluate(el => el.scrollHeight > el.clientHeight)).toBe(true);
  const button = modal.getByRole("button", { name: "Delete", exact: true });
  const bounds = await button.boundingBox();
  expect(bounds).not.toBeNull();
  expect(bounds!.y).toBeGreaterThanOrEqual(0);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(560);
  await modal.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(modal).toHaveCount(0);
  expect(await argsFor(page, "delete_session")).toBeUndefined();
});
