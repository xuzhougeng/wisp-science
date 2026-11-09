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
  await expect(page.getByTestId("add-publication-evidence")).toBeVisible();
});

test("wide source previews use height while editing and narrow layouts remain usable", async ({ page }) => {
  await page.setViewportSize({ width: 2560, height: 1440 });
  await open(page, "draft");
  await shot(page, "empty-evidence-2560");
  await page.getByTestId("add-publication-evidence").click();
  await page.getByRole("button", { name: "Research conversations", exact: true }).click();
  await page.locator(".publication-source-choice").first().click();
  const preview = page.getByRole("textbox", { name: "Persisted message text" });
  expect((await preview.boundingBox())!.height).toBeGreaterThan(450);
  await fits(page);
  await shot(page, "source-preview-2560");
  await page.getByTestId("publication-source-continue").click();
  expect((await page.locator(".publication-binding-step").boundingBox())!.width).toBeLessThanOrEqual(1121);
  for (const width of [1280, 900, 600]) {
    await page.setViewportSize({ width, height: 900 });
    await fits(page);
    await expect(page.getByRole("button", { name: "Bind exact evidence" })).toBeVisible();
  }
  await page.getByRole("button", { name: "Finalization check", exact: true }).click();
  await fits(page);
  await page.getByRole("button", { name: "Version history", exact: true }).click();
  await fits(page);
});

test("source paging stays in view and the binding form opens under the tabs with its real values", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await open(page, "draft");
  await page.evaluate(() => {
    const core = (window as any).__TAURI__.core;
    const invoke = core.invoke;
    core.invoke = async (cmd: string, args: any) => {
      // serde_wasm_bindgen hands JSON objects to the bridge as a Map.
      const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
      if (cmd !== "list_publication_sources" || arg("kind") !== "messages") return invoke(cmd, args);
      const offset = Number(arg("offset") ?? 0);
      const sources = Array.from({ length: offset === 0 ? 50 : 7 }, (_, i) => ({
        kind: "message_span", id: `message-${offset + i}`, title: "figureS2C", detail: "assistant",
        text: `**Checked** \`go_complex_heatmap.r\` part ${offset + i + 1}`,
        frame_id: "publication-session", message_seq: offset + i + 1,
      }));
      return { sources, has_more: offset === 0 };
    };
  });
  await page.getByTestId("add-publication-evidence").click();
  // Adding a source is a step of the Evidence tab; one page of sources needs no pager.
  await expect(page.locator(".publication-page-tabs button.active")).toHaveText("Evidence");
  await expect(page.getByTestId("publication-source-pagination")).toBeHidden();
  await page.getByRole("button", { name: "Research conversations", exact: true }).click();
  const choices = page.locator(".publication-source-choice");
  await expect(choices).toHaveCount(50);
  await expect(choices.first()).toContainText("Assistant");
  await expect(choices.first().locator("p")).toHaveText("Checked go_complex_heatmap.r part 1");

  // The list overflows, yet paging is reachable without scrolling it or the page.
  const list = page.locator(".publication-source-list");
  expect(await list.evaluate(el => el.scrollHeight > el.clientHeight && el.scrollTop === 0)).toBe(true);
  expect(await page.locator(".publication-page").evaluate(el => el.scrollTop)).toBe(0);
  const pager = page.getByTestId("publication-source-pagination");
  await expect(pager).toBeInViewport({ ratio: 1 });
  await expect(pager).toContainText("1–50");
  await expect(page.getByTestId("publication-source-previous")).toBeDisabled();
  await fits(page);
  await shot(page, "source-paging-1440");
  await page.getByTestId("publication-source-next").click();
  await expect(choices).toHaveCount(7);
  await expect(pager).toContainText("51–57");
  await expect(page.getByTestId("publication-source-next")).toBeDisabled();
  await page.getByTestId("publication-source-previous").click();
  await expect(choices).toHaveCount(50);

  // "Explain its use" opens the form in view, not below a full-height page.
  await choices.first().click();
  await page.getByTestId("publication-source-continue").click();
  const binding = page.getByTestId("publication-binding-dialog");
  await expect(binding).toBeInViewport({ ratio: 1 });
  await expect(page.getByRole("button", { name: "Bind exact evidence" })).toBeInViewport({ ratio: 1 });
  // Dropdowns show the values that will be saved, not their first option.
  const selects = binding.locator("select");
  await expect(selects.nth(0)).toHaveValue("publication-revision-1");
  await expect(selects.nth(3)).toHaveValue("selected");
  await expect(selects.nth(4)).toHaveValue("private");
  await page.keyboard.press("Escape");
  await expect(binding).toHaveCount(0);
  await expect(page.getByTestId("publication-source-picker")).toBeVisible();

  // Cancel leaves the picker; the outline editor also starts on its real default.
  await page.getByTestId("publication-source-cancel").click();
  await expect(page.getByTestId("publication-source-picker")).toHaveCount(0);
  await expect(page.locator(".publication-manuscript")).toBeVisible();
  await page.locator(".publication-manuscript .publication-pane-head button").click();
  await expect(page.locator(".publication-item-editor select").first()).toHaveValue("figure");
});
