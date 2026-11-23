import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => { await page.addInitScript(tauriMock); });

async function open(page: Page, mode: "cards" | "new" | "draft", locale = "en") {
  await page.goto(`/?mockPublication=${mode === "cards" ? "frozen" : "draft"}&mockLocale=${locale}`);
  await page.locator(".proj-card-main").first().click();
  await page.evaluate((mode) => {
    const core = (window as any).__TAURI__.core;
    const invoke = core.invoke;
    let created = false;
    core.invoke = async (cmd: string, args: any) => {
      const value = await invoke(cmd, args);
      if (cmd === "create_publication_workspace") created = true;
      if (cmd !== "get_publication_workspace") return value;
      if (mode === "new" && !created) return { ...value, publications: [], publication: null, revisions: [], revision: null, items: [], bindings: [], lineage: [] };
      if (mode !== "cards") return value;
      return {
        ...value,
        bindings: Array.from({ length: 6 }, (_, i) => ({ ...value.bindings[0], id: `wide-evidence-${i}`, purpose: `Figure ${i + 1}: reviewed treatment comparison` })),
        lineage: Array.from({ length: 6 }, (_, i) => ({ ...value.lineage[0], binding_id: `wide-evidence-${i}`, source_label: `figure_${i + 1}.png` })),
      };
    };
  }, mode);
  await page.locator(".sidebar").getByRole("button", { name: /^(Publication|论文整理)$/ }).click();
  await expect(page.getByTestId("publication-workspace")).toBeVisible();
}

// The bind form is only reached from "Use in publication" on an artifact or a
// run. The right pane is driven at the default viewport; callers resize after.
async function useArtifactInPublication(page: Page) {
  await page.goto("/?mockPublication=draft&mockLocale=en");
  await page.locator(".proj-card-main").first().click();
  await page.locator('[data-session-id="publication-session"]').click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByTestId("right-launcher-artifacts").click();
  const artifact = page.locator('.rp-tile[data-artifact-name="plddt_profile.png"]');
  await artifact.getByRole("button", { name: "More" }).click();
  await page.locator(".rp-tile-menu").getByRole("button", { name: "Use in publication" }).click();
  const binding = page.getByTestId("publication-binding-dialog");
  await expect(binding).toBeVisible();
  return binding;
}

async function fits(page: Page) {
  expect(await page.locator(".publication-page").evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  const pane = (await page.locator(".publication-page").boundingBox())!;
  const content = (await page.getByTestId("publication-workspace").boundingBox())!;
  expect(content.width).toBeGreaterThanOrEqual(pane.width - 2);
  const heading = (await page.locator(".publication-workspace-head h2").boundingBox())!;
  expect(heading.x - pane.x).toBeLessThanOrEqual(65);
}

async function shot(page: Page, name: string) {
  if (!process.env.WISP_PUBLICATION_SHOTS) return;
  mkdirSync(process.env.WISP_PUBLICATION_SHOTS, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: resolve(process.env.WISP_PUBLICATION_SHOTS, `${name}.png`) });
}

for (const [width, height] of [[1920, 1080], [2560, 1440], [3840, 2160]]) {
  test(`publication uses a ${width}px display for readable evidence columns`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await open(page, "cards", "zh");
    const cards = page.getByTestId("publication-evidence-card");
    await expect(cards).toHaveCount(6);
    await fits(page);
    const outline = (await page.locator(".publication-manuscript").boundingBox())!;
    expect(outline.width).toBeLessThanOrEqual(361);
    const first = (await cards.nth(0).boundingBox())!;
    const second = (await cards.nth(1).boundingBox())!;
    expect(first.width).toBeGreaterThan(400);
    expect(second.x).toBeGreaterThan(first.x + first.width);
    expect(Math.abs(first.y - second.y)).toBeLessThan(2);
    const nextRow = (await cards.nth(width === 1920 ? 2 : 3).boundingBox())!;
    expect(nextRow.y).toBeGreaterThan(first.y + first.height);
    await shot(page, `evidence-${width}`);
    await page.evaluate(() => { document.documentElement.dataset.theme = "dark"; });
    await fits(page);
    await shot(page, `evidence-${width}-dark`);
  });
}

test("new publication form stays readable on wide displays and stacks in smaller panes", async ({ page }) => {
  await page.setViewportSize({ width: 3840, height: 2160 });
  await open(page, "new", "zh");
  for (const width of [3840, 1920, 1280, 900, 600]) {
    await page.setViewportSize({ width, height: width > 1920 ? 2160 : 1080 });
    await fits(page);
    const intro = (await page.locator(".publication-create-intro").boundingBox())!;
    const form = (await page.locator(".publication-create-fields").boundingBox())!;
    expect(form.width).toBeLessThanOrEqual(761);
    if (width >= 1920) {
      expect(form.x).toBeGreaterThan(intro.x + intro.width);
      expect(form.width).toBeGreaterThan(600);
    } else {
      expect(form.y).toBeGreaterThan(intro.y + intro.height);
    }
    await shot(page, `new-publication-${width}`);
  }
  await page.getByTestId("publication-new-title").fill("根系生长研究");
  await page.locator(".publication-create-fields button").click();
  await expect(page.locator(".publication-create")).toHaveCount(0);
  await expect(page.getByTestId("publication-evidence-hint")).toContainText("用于论文");
  await expect(page.getByTestId("add-publication-evidence")).toHaveCount(0);
});

test("a draft revision explains how evidence is attached instead of offering a source picker", async ({ page }) => {
  await page.setViewportSize({ width: 2560, height: 1440 });
  await open(page, "draft");
  await shot(page, "empty-evidence-2560");
  const workspace = page.getByTestId("publication-workspace");
  await expect(workspace.locator(".publication-page-tabs button.active")).toHaveText("Evidence");
  const hint = workspace.getByTestId("publication-evidence-hint");
  await expect(hint).toHaveText("Ask the assistant in a conversation to set a file as evidence for this paper, or choose “Use in publication” on an artifact or a run.");
  await expect(hint).toBeInViewport({ ratio: 1 });
  await expect(workspace.getByRole("button", { name: "Add evidence" })).toHaveCount(0);
  await expect(workspace.getByTestId("add-publication-evidence")).toHaveCount(0);
  await expect(workspace.getByTestId("publication-source-picker")).toHaveCount(0);
  for (const width of [2560, 1280, 600]) {
    await page.setViewportSize({ width, height: 900 });
    await fits(page);
    await expect(hint).toBeVisible();
  }
});

test("the binding form stays readable on wide displays and narrow layouts remain usable", async ({ page }) => {
  const binding = await useArtifactInPublication(page);
  await page.setViewportSize({ width: 2560, height: 1440 });
  await fits(page);
  expect((await page.locator(".publication-binding-step").boundingBox())!.width).toBeLessThanOrEqual(1121);
  await shot(page, "binding-form-2560");
  for (const width of [1280, 900, 600]) {
    await page.setViewportSize({ width, height: 900 });
    await fits(page);
    await expect(page.getByRole("button", { name: "Bind exact evidence" })).toBeVisible();
  }
  // Cancel returns to the evidence page the form was opened over.
  await binding.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(binding).toHaveCount(0);
  await expect(page.locator(".publication-evidence")).toBeVisible();
  await expect(page.getByTestId("publication-evidence-hint")).toBeVisible();
  await page.getByRole("button", { name: "Finalization check", exact: true }).click();
  await fits(page);
  await page.getByRole("button", { name: "Version history", exact: true }).click();
  await fits(page);
});

test("Escape right after opening the binding form closes only the form and returns to the evidence page", async ({ page }) => {
  const binding = await useArtifactInPublication(page);
  // Escape right after opening closes the form alone; the evidence page it covered is back.
  await page.keyboard.press("Escape");
  await expect(binding).toHaveCount(0);
  await expect(page.getByTestId("publication-workspace")).toBeVisible();
  await expect(page.locator(".publication-manuscript")).toBeVisible();
  await expect(page.getByTestId("publication-evidence-hint")).toBeVisible();
  // The outline editor starts on its real default.
  await page.locator(".publication-manuscript .publication-pane-head button").click();
  await expect(page.locator(".publication-item-editor select").first()).toHaveValue("figure");
});

test("the binding form replaces the evidence content in view and shows the values it will save", async ({ page }) => {
  const binding = await useArtifactInPublication(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  // Binding is a step of the Evidence tab: the form sits under the tabs, not below a full-height page.
  await expect(page.locator(".publication-page-tabs button.active")).toHaveText("Evidence");
  await expect(page.locator(".publication-workspace-grid")).toBeHidden();
  await expect(binding).toContainText("plddt_profile.png");
  await expect(binding).toBeInViewport({ ratio: 1 });
  await expect(page.getByRole("button", { name: "Bind exact evidence" })).toBeInViewport({ ratio: 1 });
  await fits(page);
  await shot(page, "binding-form-1440");
  // Dropdowns show the values that will be saved, not their first option.
  const selects = binding.locator("select");
  await expect(selects.nth(0)).toHaveValue("publication-revision-1");
  await expect(selects.nth(3)).toHaveValue("selected");
  await expect(selects.nth(4)).toHaveValue("private");
});
