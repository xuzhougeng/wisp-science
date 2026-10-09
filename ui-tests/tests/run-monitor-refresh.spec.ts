import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

async function setup(page: Page) {
  await page.addInitScript(tauriMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator(".sidebar").getByRole("button", { name: "New session" })).toBeVisible();
  await page.evaluate(() => {
    const w = window as any;
    const original = w.__TAURI__.core.invoke;
    const state = w.__runMonitorRefresh = {
      listError: "", detailError: "", holdList: false, holdDetail: false,
      listCalls: 0, detailCalls: 0, listActive: 0, detailActive: 0,
      listPeak: 0, detailPeak: 0, releaseList: null, releaseDetail: null,
    };
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      if (cmd !== "list_runs" && cmd !== "get_run_detail") return original(cmd, args);
      const list = cmd === "list_runs";
      const prefix = list ? "list" : "detail";
      state[`${prefix}Calls`] += 1;
      state[`${prefix}Active`] += 1;
      state[`${prefix}Peak`] = Math.max(state[`${prefix}Peak`], state[`${prefix}Active`]);
      // Snapshot the response before blocking, as a real DB read may already
      // have observed a lifecycle that changes while IPC is still pending.
      try {
        const response = structuredClone(await original(cmd, args));
        if (state[list ? "holdList" : "holdDetail"]) {
          await new Promise<void>((resolve) => {
            state[list ? "releaseList" : "releaseDetail"] = resolve;
          });
        }
        const error = state[`${prefix}Error`];
        if (error) throw new Error(error);
        return response;
      } finally {
        state[`${prefix}Active`] -= 1;
      }
    };
  });
}

async function startMonitor(page: Page) {
  await page.locator("#composer-input").fill("MONITORRUN");
  await page.getByRole("button", { name: "Send" }).click();
  const card = page.locator('[data-testid="run-monitor-card"][data-run-id="run-local-002"]');
  await expect(card).toBeVisible();
  return card;
}

test("run monitoring reports failed summary refreshes and recovers without failing the Run", async ({ page }) => {
  await setup(page);
  await page.evaluate(() => { (window as any).__runMonitorRefresh.listError = "pool timed out waiting for a connection"; });
  const card = await startMonitor(page);
  await expect(card.getByTestId("run-monitor-refresh-error")).toContainText("pool timed out");
  await expect(card.locator(".run-status")).toHaveText("Running");
  await expect(card.locator(".run-status.failed")).toHaveCount(0);
  await page.evaluate(() => {
    const w = window as any;
    w.__runMonitorRefresh.listError = "";
    w.__mockRuns.find((run: any) => run.id === "run-local-002").stdout_tail = "Transfer is making progress";
  });
  await expect(card.getByTestId("run-monitor-refresh-error")).toHaveCount(0);
  await expect(card).toContainText("Transfer is making progress");
});

test("run monitoring coalesces slow summary polls and shows delayed status feedback", async ({ page }) => {
  await setup(page);
  await page.evaluate(() => { (window as any).__runMonitorRefresh.holdList = true; });
  const card = await startMonitor(page);
  await expect(card.getByTestId("run-monitor-refresh-error")).toContainText("taking longer", { timeout: 15_000 });
  const pending = await page.evaluate(() => {
    const state = (window as any).__runMonitorRefresh;
    return { calls: state.listCalls, peak: state.listPeak, active: state.listActive };
  });
  expect(pending).toEqual({ calls: 1, peak: 1, active: 1 });
  await page.evaluate(() => {
    const state = (window as any).__runMonitorRefresh;
    state.holdList = false;
    state.releaseList();
  });
  await expect(card.getByTestId("run-monitor-refresh-error")).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => (window as any).__runMonitorRefresh.listCalls)).toBeGreaterThan(1);
  expect(await page.evaluate(() => (window as any).__runMonitorRefresh.listPeak)).toBe(1);
});

test("run monitoring retries unchanged details and coalesces summary changes during a pending read", async ({ page }) => {
  await setup(page);
  await page.evaluate(() => { (window as any).__runMonitorRefresh.detailError = "database temporarily unavailable"; });
  const card = await startMonitor(page);
  await expect(card.getByTestId("run-monitor-detail-error")).toContainText("database temporarily unavailable");
  // The first summary refresh also records the newly submitted frame_id.
  // Let that metadata change settle before testing retries with no changes.
  await expect.poll(() => page.evaluate(() => (window as any).__runMonitorRefresh.listCalls)).toBeGreaterThanOrEqual(2);
  const initialDetails = await page.evaluate(() => (window as any).__runMonitorRefresh.detailCalls);
  await page.evaluate(() => {
    const state = (window as any).__runMonitorRefresh;
    state.detailError = "";
    state.holdDetail = true;
    state.detailPeak = 0;
  });
  // No summary change triggers this second request: it must come from retry.
  await expect.poll(() => page.evaluate(() => (window as any).__runMonitorRefresh.detailCalls)).toBe(initialDetails + 1);
  const polls = await page.evaluate(() => (window as any).__runMonitorRefresh.listCalls);
  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, { status: "succeeded", exit_code: 0, ended_at: Math.floor(Date.now() / 1000), stdout_tail: "Final transfer output" });
  });
  await expect(card.locator(".run-status")).toHaveText("Succeeded");
  await expect.poll(() => page.evaluate(() => (window as any).__runMonitorRefresh.listCalls)).toBeGreaterThanOrEqual(polls + 2);
  expect(await page.evaluate(() => (window as any).__runMonitorRefresh.detailCalls)).toBe(initialDetails + 1);
  await page.evaluate(() => {
    const state = (window as any).__runMonitorRefresh;
    state.holdDetail = false;
    state.releaseDetail();
  });
  await expect(card.getByTestId("run-monitor-detail-error")).toHaveCount(0);
  await expect(card).toContainText("Final transfer output");
  expect(await page.evaluate(() => (window as any).__runMonitorRefresh.detailPeak)).toBe(1);
});

test("a failed monitor lookup without a Run record shows status unavailable", async ({ page }) => {
  await setup(page);
  const card = await startMonitor(page);
  await page.evaluate(() => {
    const w = window as any;
    const index = w.__mockRuns.findIndex((run: any) => run.id === "run-local-002");
    const run = w.__mockRuns[index];
    w.__tauriEmit("agent", { kind: "ToolResult", frame_id: run.frame_id, name: "monitor_run", ok: false,
      content: "pool timed out while checking status" });
    w.__mockRuns.splice(index, 1);
  });
  await expect(card.locator(".run-status")).toHaveText("Status unavailable");
  await expect(card).toContainText("pool timed out while checking status");
  await expect(card.locator(".run-status.failed")).toHaveCount(0);
});

test("a failed monitor query preserves a known Run status and displays its error", async ({ page }) => {
  await setup(page);
  const card = await startMonitor(page);
  await page.evaluate(() => {
    const w = window as any;
    const run = w.__mockRuns.find((item: any) => item.id === "run-local-002");
    w.__tauriEmit("agent", { kind: "ToolResult", frame_id: run.frame_id, name: "monitor_run", ok: false,
      content: "pool timed out while checking status" });
  });
  await expect(card.locator(".run-status")).toHaveText("Running");
  await expect(card.getByTestId("run-monitor-tool-error")).toContainText("pool timed out while checking status");
  await expect(card.locator(".run-status.failed")).toHaveCount(0);
});

test("a terminal failed Run record is not reported as a monitor query failure", async ({ page }) => {
  await setup(page);
  const card = await startMonitor(page);
  await page.evaluate(() => {
    const w = window as any;
    const run = w.__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, { status: "failed", exit_code: 1, ended_at: Math.floor(Date.now() / 1000), stderr_tail: "rsync exited with code 1" });
    w.__tauriEmit("agent", { kind: "ToolResult", frame_id: run.frame_id, name: "monitor_run", ok: false, content: JSON.stringify(run) });
  });
  await expect(card.locator(".run-status")).toHaveText("Failed");
  await expect(card.getByTestId("run-monitor-tool-error")).toHaveCount(0);
});
