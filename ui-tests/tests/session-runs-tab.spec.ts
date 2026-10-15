import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// Runs for two conversations plus one without a frame. The Runs tab must show
// only the active conversation's rows, working ones first.
function sessionRunsFixture() {
  const w = window as any;
  const now = Math.floor(Date.now() / 1000);
  const base = w.__mockRuns.find((run: any) => run.id === "run-local-002");
  const make = (overrides: Record<string, unknown>) => ({
    ...base, stdout_tail: "", stderr_tail: "", exit_code: null, ended_at: null, ...overrides,
  });
  const runs = [
    make({ id: "a-running", frame_id: "s-model-a", title: "Alignment batch", status: "running",
      created_at: now - 300, started_at: now - 299 }),
    make({ id: "a-transfer", frame_id: "s-model-a", title: "Upload reads", kind: "file_transfer",
      status: "submitted", created_at: now - 60, started_at: now - 59 }),
    make({ id: "a-done", frame_id: "s-model-a", title: "QC report", status: "succeeded",
      created_at: now - 900, started_at: now - 899, ended_at: now - 800, exit_code: 0, stdout_tail: "qc table written" }),
    make({ id: "b-running", frame_id: "s-model-b", title: "Other session job", status: "running",
      created_at: now - 100, started_at: now - 99 }),
    make({ id: "no-frame", frame_id: null, title: "Environment job", status: "running",
      created_at: now - 50, started_at: now - 49 }),
  ];
  w.__mockRuns.splice(0, w.__mockRuns.length, ...runs);
}

async function setup(page: Page) {
  await page.addInitScript({ content: `(${tauriMock.toString()})(); (${sessionRunsFixture.toString()})();` });
  await page.goto("/?mockSessionModels=1");
  await page.locator(".proj-card-main").first().click();
  await page.locator('[data-session-id="s-model-a"]').click();
  await expect(page.locator("#composer-input")).toBeVisible();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  const panel = page.locator(".rightpane");
  await panel.getByRole("button", { name: "Add panel" }).click();
  await panel.locator(".rp-tab-add-menu").getByRole("button", { name: /^Runs/ }).click();
  await expect(panel.locator(".rp-tab-add-menu")).toHaveCount(0);
  return panel;
}

const cardIds = (scope: ReturnType<Page["locator"]>) =>
  scope.locator('[data-testid="run-monitor-card"]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-run-id")));

test("the Runs tab lists only the current session's runs, active first, with a badge count", async ({ page }) => {
  const panel = await setup(page);
  await expect(panel.locator(".rp-tab.active")).toHaveText("Runs (2)");

  const active = panel.getByTestId("session-runs-active");
  await expect(active.getByTestId("session-runs-active-count")).toHaveText("2");
  await expect.poll(() => cardIds(active)).toEqual(["a-transfer", "a-running"]);
  await expect(panel.locator('[data-run-id="b-running"]')).toHaveCount(0);
  await expect(panel.locator('[data-run-id="no-frame"]')).toHaveCount(0);
  // Embedded cards never offer the transcript's dismiss action.
  await expect(panel.getByRole("button", { name: "Dismiss completed run card" })).toHaveCount(0);

  const completed = panel.getByTestId("session-runs-completed");
  const toggle = completed.getByRole("button", { name: "Completed (1)" });
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(completed.locator('[data-testid="run-monitor-card"]')).toHaveCount(0);
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  const done = completed.locator('[data-run-id="a-done"]');
  await expect(done).toBeVisible();
  await expect(done).toContainText("qc table written");
});

test("a run that finishes moves to the completed group and the badge follows", async ({ page }) => {
  const panel = await setup(page);
  await expect(panel.locator(".rp-tab.active")).toHaveText("Runs (2)");
  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "a-running");
    Object.assign(run, { status: "succeeded", exit_code: 0, ended_at: Math.floor(Date.now() / 1000) });
  });
  // The idle poll runs every five seconds.
  await expect(panel.locator(".rp-tab.active")).toHaveText("Runs (1)", { timeout: 15_000 });
  const active = panel.getByTestId("session-runs-active");
  await expect.poll(() => cardIds(active)).toEqual(["a-transfer"]);
  const toggle = panel.getByTestId("session-runs-completed").getByRole("button", { name: "Completed (2)" });
  await toggle.click();
  await expect.poll(() => cardIds(panel.getByTestId("session-runs-completed"))).toEqual(["a-running", "a-done"]);
});

test("switching sessions re-filters the Runs tab and resets the completed group", async ({ page }) => {
  const panel = await setup(page);
  await panel.getByTestId("session-runs-completed").getByRole("button", { name: "Completed (1)" }).click();
  await expect(panel.locator('[data-run-id="a-done"]')).toBeVisible();

  // Loading a session returns the pane to Artifacts; the Runs tab stays open.
  await page.locator('[data-session-id="s-model-b"]').click();
  await expect(page.locator("#composer-input")).toBeVisible();
  await panel.locator(".rp-tab", { hasText: /^Runs/ }).click();
  await expect(panel.locator(".rp-tab.active")).toHaveText("Runs (1)");
  await expect.poll(() => cardIds(panel.getByTestId("session-runs-active"))).toEqual(["b-running"]);
  await expect(panel.getByTestId("session-runs-completed")).toHaveCount(0);

  await page.locator('[data-session-id="s-model-a"]').click();
  await panel.locator(".rp-tab", { hasText: /^Runs/ }).click();
  await expect(panel.locator(".rp-tab.active")).toHaveText("Runs (2)");
  const toggle = panel.getByTestId("session-runs-completed").getByRole("button", { name: "Completed (1)" });
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
});

test("a session without runs shows the empty state and an unbadged tab", async ({ page }) => {
  await page.addInitScript({ content: `(${tauriMock.toString()})(); window.__mockRuns.splice(0);` });
  await page.goto("/?mockSessionModels=1");
  await page.locator(".proj-card-main").first().click();
  await page.locator('[data-session-id="s-model-a"]').click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  const panel = page.locator(".rightpane");
  await panel.getByRole("button", { name: "Add panel" }).click();
  await panel.locator(".rp-tab-add-menu").getByRole("button", { name: "Runs", exact: true }).click();
  await expect(panel.locator(".rp-tab.active")).toHaveText("Runs");
  await expect(panel.getByTestId("session-runs-empty")).toContainText("No runs in this session");
});
