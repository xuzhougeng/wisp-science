import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";
import { clickInChat, revealInChat } from "./chat-scroll";

type Options = { status?: string; monitor?: boolean; fallback?: boolean; foreign?: boolean };

function completedRunsFixture(options: Options) {
  const w = window as any;
  const now = Math.floor(Date.now() / 1000);
  const base = w.__mockRuns.find((run: any) => run.id === "run-local-002");
  const runs = ["first", "second"].map((id, index) => ({
    ...base, id, title: `Analysis ${id}`, command: `python ${id}.py`,
    status: index === 0 ? options.status ?? "succeeded" : "succeeded",
    created_at: now - 120, started_at: now - 119,
    ended_at: options.status === "running" && index === 0 ? null : now - 90,
    stdout_tail: `Result from ${id}`, stderr_tail: "", exit_code: 0,
  }));
  w.__mockRuns.splice(0, w.__mockRuns.length, ...runs);
  // Freeze tool output as persisted at submission; subsequent polling changes
  // the Run record, not the transcript. Cover both supported result shapes.
  const outputs = [JSON.stringify({ run_id: "first" }), JSON.stringify(runs[1])];
  if (options.fallback) outputs[0] = JSON.stringify(runs[0]);
  if (options.foreign) runs[0].frame_id = "another-session";
  const invoke = w.__TAURI__.core.invoke;
  w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
    const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
    if (cmd === "load_session" && arg("id") === "s-complete") return {
      items: [
        { role: "user", text: "Run two analyses" },
        ...runs.map((run, i) => ({ role: "tool", tool_name: i ? "wisp_run_in_context" : "run_in_context",
          input: run.command, text: outputs[i], ok: true })),
        ...(options.monitor ? [{ role: "tool", tool_name: "monitor_run", input: "first",
          text: outputs[0], ok: true }] : []),
        { role: "assistant", text: "Analysis summary" },
      ], next_before_seq: null, user_offset: 0,
    };
    if (options.fallback && cmd === "list_runs") return [];
    return invoke(cmd, args);
  };
}

async function setup(page: Page, options: Options = {}) {
  await page.addInitScript({ content: `(${tauriMock.toString()})(); (${completedRunsFixture.toString()})(${JSON.stringify(options)});` });
  await page.goto("/");
  await page.getByTestId("recent-session-card").nth(1).click();
  await expect(page.getByText("Analysis summary", { exact: true })).toBeVisible();
}

async function openRunStep(page: Page, index: number) {
  const group = page.locator(".steps").filter({ has: page.locator(".steps-head") }).first();
  // Heads sit above the fold once a run card expands below them; see chat-scroll.ts.
  if (await group.locator(".steps-head").getAttribute("aria-expanded") !== "true") {
    await clickInChat(group.locator(".steps-head"));
  }
  const step = group.locator(".step").nth(index);
  await expect(step.locator(".step-head")).toHaveAttribute("aria-expanded", "false");
  await clickInChat(step.locator(".step-head"));
  return step;
}

test("completed Runs are collapsed under their exact submission rows and survive reload", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await setup(page);
  await expect(page.getByTestId("auto-run-monitor")).toHaveCount(0);
  await expect(page.getByTestId("run-monitor-card")).toHaveCount(0);
  const first = await openRunStep(page, 0);
  await expect(first.getByTestId("run-monitor-card")).toHaveAttribute("data-run-id", "first");
  await expect(first).toContainText("Result from first");
  await expect(first.getByRole("button", { name: "Dismiss completed run card" })).toHaveCount(0);
  const second = await openRunStep(page, 1);
  await expect(second.getByTestId("run-monitor-card")).toHaveAttribute("data-run-id", "second");
  await expect(second).toContainText("Result from second");
  // Identical polling must preserve an expanded card and its DOM.
  await first.getByTestId("run-monitor-card").evaluate(el => (el as any).__stableProbe = true);
  const before = await page.evaluate(() => (window as any).__skillInvokeLog.filter((call: any) => call.cmd === "list_runs").length);
  await expect.poll(() => page.evaluate(() => (window as any).__skillInvokeLog.filter((call: any) => call.cmd === "list_runs").length)).toBeGreaterThan(before + 1);
  expect(await first.getByTestId("run-monitor-card").evaluate(el => (el as any).__stableProbe)).toBe(true);
  await clickInChat(second.locator(".step-head"));
  // A plain scrollIntoView is undone the same way; park the first card for the screenshot.
  await revealInChat(first.locator(".step-head"));
  await page.screenshot({ path: "test-results/completed-runs-expanded.png", fullPage: true, animations: "disabled" });
  await page.reload();
  await page.getByTestId("recent-session-card").nth(1).click();
  await expect(page.getByText("Analysis summary", { exact: true })).toBeVisible();
  await expect(page.getByTestId("run-monitor-card")).toHaveCount(0);
  await expect(page.getByTestId("auto-run-monitor")).toHaveCount(0);
  await page.screenshot({ path: "test-results/completed-runs-collapsed.png", fullPage: true, animations: "disabled" });
  await expect((await openRunStep(page, 0)).getByTestId("run-monitor-card")).toHaveAttribute("data-run-id", "first");
});

for (const status of ["succeeded", "failed", "cancelled", "timed_out", "lost"]) {
  test(`active Run folds into its submission on ${status}`, async ({ page }) => {
    await setup(page, { status: "running" });
    await expect(page.getByTestId("auto-run-monitor")).toContainText("Analysis first");
    await page.evaluate(status => {
      Object.assign((window as any).__mockRuns[0], { status, ended_at: Math.floor(Date.now() / 1000) });
    }, status);
    await expect(page.getByTestId("auto-run-monitor")).toHaveCount(0);
    await expect(page.getByTestId("run-monitor-card")).toHaveCount(0);
    const step = await openRunStep(page, 0);
    await expect(step.locator(`.run-status.${status}`)).toBeVisible();
    await expect(step).toContainText("Result from first");
  });
}

test("a completed foreground monitor does not duplicate the folded Run", async ({ page }) => {
  await setup(page, { monitor: true });
  await expect(page.getByTestId("run-monitor-card")).toHaveCount(0);
  const step = await openRunStep(page, 0);
  await expect(step.getByTestId("run-monitor-card")).toBeVisible();
  await expect(page.getByTestId("run-monitor-card")).toHaveCount(1);
});

test("historical full Run results remain available without the recent Runs list", async ({ page }) => {
  await setup(page, { fallback: true });
  const step = await openRunStep(page, 0);
  await expect(step.getByTestId("run-monitor-card")).toBeVisible();
  await expect(step).toContainText("Result from first");
});

test("a Run from another session is not attached to a submission row", async ({ page }) => {
  await setup(page, { foreign: true });
  const step = await openRunStep(page, 0);
  await expect(step.locator(".tool-output")).toContainText("first");
  await expect(step.getByTestId("run-monitor-card")).toHaveCount(0);
  await expect((await openRunStep(page, 1)).getByTestId("run-monitor-card")).toHaveAttribute("data-run-id", "second");
});

function transferRunsFixture(options: { status?: string } = {}) {
  const w = window as any;
  const now = Math.floor(Date.now() / 1000);
  const base = w.__mockRuns.find((run: any) => run.id === "run-local-002");
  const run = {
    ...base,
    id: "xfer-1",
    title: "Download compare_fixed_cluster_nesting.png from CPU3",
    command: "download ssh:CPU3:/data/compare_fixed_cluster_nesting.png -> local:/tmp/compare_fixed_cluster_nesting.png",
    kind: "file_transfer",
    context_id: "ssh:CPU3",
    status: options.status ?? "succeeded",
    created_at: now - 120,
    started_at: now - 119,
    ended_at: options.status === "running" ? null : now - 90,
    stdout_tail: "downloaded compare_fixed_cluster_nesting.png",
    stderr_tail: "",
    exit_code: options.status === "running" ? null : 0,
    remote_workdir: "E:\\\\cross-species-root\\\\analysis\\\\compare-fixed",
  };
  w.__mockRuns.splice(0, w.__mockRuns.length, run);
  const output = JSON.stringify({
    run_id: "xfer-1",
    status: "submitted",
    route: "local",
    transport: "scp",
    destination_path: "/tmp/compare_fixed_cluster_nesting.png",
  });
  const invoke = w.__TAURI__.core.invoke;
  w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
    const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
    if (cmd === "load_session" && arg("id") === "s-complete") return {
      items: [
        { role: "user", text: "Download the comparison figures" },
        { role: "tool", tool_name: "transfer_between_contexts",
          input: "ssh:CPU3:/data/compare_fixed_cluster_nesting.png", text: output, ok: true },
        { role: "assistant", text: "Transfer submitted" },
      ], next_before_seq: null, user_offset: 0,
    };
    return invoke(cmd, args);
  };
}

async function setupTransfer(page: Page, options: { status?: string } = {}) {
  await page.addInitScript({ content: `(${tauriMock.toString()})(); (${transferRunsFixture.toString()})(${JSON.stringify(options)});` });
  await page.goto("/");
  await page.getByTestId("recent-session-card").nth(1).click();
  await expect(page.getByText("Transfer submitted", { exact: true })).toBeVisible();
}

test("completed file transfers fold into transfer_between_contexts rows", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await setupTransfer(page);
  await expect(page.getByTestId("auto-run-monitor")).toHaveCount(0);
  await expect(page.getByTestId("run-monitor-card")).toHaveCount(0);
  const step = await openRunStep(page, 0);
  await expect(step.getByTestId("run-monitor-card")).toHaveAttribute("data-run-id", "xfer-1");
  await expect(step).toContainText("Download compare_fixed_cluster_nesting.png from CPU3");
  await expect(step.getByRole("button", { name: "Dismiss completed run card" })).toHaveCount(0);
});

test("active file transfer folds into its submission when it settles", async ({ page }) => {
  await setupTransfer(page, { status: "running" });
  await expect(page.getByTestId("auto-run-monitor")).toContainText("Download compare_fixed_cluster_nesting.png from CPU3");
  await page.evaluate(() => {
    Object.assign((window as any).__mockRuns[0], {
      status: "succeeded",
      ended_at: Math.floor(Date.now() / 1000),
      exit_code: 0,
    });
  });
  await expect(page.getByTestId("auto-run-monitor")).toHaveCount(0);
  await expect(page.getByTestId("run-monitor-card")).toHaveCount(0);
  const step = await openRunStep(page, 0);
  await expect(step.locator(".run-status.succeeded")).toBeVisible();
  await expect(step).toContainText("downloaded compare_fixed_cluster_nesting.png");
});

test("a live wait-for-completion result folds before the next Run poll", async ({ page }) => {
  await setup(page);
  await expect.poll(() => page.evaluate(() => Boolean((window as any).__tauriListenerReady?.("agent")))).toBe(true);
  await page.evaluate(() => {
    const w = window as any;
    const emit = (event: any) => w.__tauriEmit("agent", { frame_id: "s-complete", ...event });
    // A complete tool result is enough; this Run is deliberately not in list_runs.
    const run = { ...w.__mockRuns[0], id: "live-result", title: "Live analysis", stdout_tail: "Live output" };
    emit({ kind: "User", text: "Run one more analysis" });
    emit({ kind: "ToolCall", name: "run_in_context", preview: "python live.py" });
    emit({ kind: "ToolResult", name: "run_in_context", ok: true, content: JSON.stringify(run) });
    emit({ kind: "Text", delta: "Live analysis finished" });
    emit({ kind: "Done", stop_reason: "end_turn" });
  });
  await expect(page.getByText("Live analysis finished", { exact: true })).toBeVisible();
  await expect(page.getByTestId("run-monitor-card")).toHaveCount(0);
  const group = page.locator(".steps").last();
  await group.locator(".steps-head").click();
  const step = group.locator(".step").filter({ hasText: "python live.py" });
  await step.locator(".step-head").click();
  await expect(step.getByTestId("run-monitor-card")).toHaveAttribute("data-run-id", "live-result");
  await expect(step).toContainText("Live output");
});
