import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

function monitoredReportFixture(options: { active?: boolean; unowned?: boolean; review?: boolean } = {}) {
  const w = window as any;
  const base = w.__mockRuns.find((run: any) => run.id === "run-local-002");
  const runs = Array.from({ length: 10 }, (_, index) => ({
    ...base,
    id: `report-run-${index}`,
    frame_id: "monitored-report",
    title: `Report phase ${index + 1}`,
    kind: options.review && index === 0 ? "ssh_direct" : base.kind,
    status: options.active && index === 0 ? "running" : "succeeded",
    created_at: 1_790_606_441 + index * 60,
    started_at: 1_790_606_442 + index * 60,
    ended_at: options.active && index === 0 ? null : 1_790_606_451 + index * 60,
    stdout_tail: `Output for phase ${index + 1}`,
    stderr_tail: "",
    exit_code: options.active && index === 0 ? null : 0,
  }));
  w.__mockRuns.splice(0, w.__mockRuns.length, ...runs);
  if (options.review) w.__mockRunWorkspaceFiles["report-run-0"] = {
    "": [{ path: "report.tsv", kind: "file", size_bytes: 100, file_count: null }],
  };
  const invoke = w.__TAURI__.core.invoke;
  w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
    const id = args instanceof Map ? args.get("id") : args?.id;
    if (cmd === "load_session" && id === "monitored-report") return {
      items: [
        { role: "user", text: "Check the analysis and produce the final report" },
        ...runs.flatMap((run, index) => [
          { role: "tool", tool_name: "python", ok: true, input: `prepare(${index})`, text: "Prepared", duration_ms: 20 },
          ...(!(options.unowned && index === 0) ? [{
            role: "tool", tool_name: index % 2 ? "transfer_between_contexts" : "run_in_context",
            ok: true, input: `phase ${index + 1}`, text: JSON.stringify({ run_id: run.id, status: "submitted" }), duration_ms: 30,
          }] : []),
          // Nonempty progress before a monitor must fold with that monitor too.
          ...((options.active || options.unowned) && index === 0 ? [] : [
            { role: "assistant", text: `Checking run ${index + 1}` },
            { role: "reasoning", text: `Checking results for run ${index + 1}` },
          ]),
          { role: "tool", tool_name: index % 2 ? "wisp_monitor_run" : "monitor_run", ok: true,
            input: run.id, text: JSON.stringify(run), duration_ms: 50 },
          { role: "usage", text: JSON.stringify({ input: 100, output: 10 }) },
        ]),
        { role: "tool", tool_name: "update_plan", ok: true, input: "", text: JSON.stringify({ plan: [{ step: "Write report", status: "completed" }] }) },
        { role: "assistant", text: "## Monitored analysis report\n\nAll analysis phases are ready." },
      ],
      outline: [{ user_index: 0, seq: 1, text: "Check the analysis and produce the final report", sent_at: 1_790_606_441, response_at: 1_790_607_171 }],
      next_before_seq: null, user_offset: 0,
    };
    return invoke(cmd, args);
  };
}

async function openMonitoredReport(page: Page) {
  await expect.poll(() => page.evaluate(() => Boolean((window as any).__tauriListenerReady?.("open-session")))).toBe(true);
  await page.evaluate(() => (window as any).__tauriEmit("open-session", { projectId: "other", sessionId: "monitored-report" }));
  await expect(page.getByRole("heading", { name: "Monitored analysis report" })).toBeVisible();
}

for (const width of [1280, 540]) {
  test(`completed monitored report has one disclosure and one turn clock at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.addInitScript(tauriMock);
    await page.addInitScript(monitoredReportFixture);
    await page.goto("/");
    await openMonitoredReport(page);
    const activity = page.locator(".activity-summary");
    await expect(activity).toHaveCount(1);
    await expect(activity.locator(".steps-head")).toHaveAttribute("aria-expanded", "false");
    await expect(activity.locator(".steps-meta")).toHaveText("12m 10s");
    await expect(page.getByTestId("run-monitor-card")).toHaveCount(0);
    await expect(page.locator(".run-monitor-wrap")).toHaveCount(0);
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: `test-results/completed-monitored-report-${width}.png`, fullPage: true, animations: "disabled" });
    await activity.locator(".steps-head").click();
    await expect(activity.locator(".step-progress")).toHaveCount(10);
    await expect(activity.locator(".step-think")).toHaveCount(10);
    const submission = activity.locator(".step").filter({ has: page.locator(".step-name", { hasText: /^run_in_context$/ }) }).first();
    await submission.locator(".step-head").click();
    await expect(submission.getByTestId("run-monitor-card")).toHaveAttribute("data-run-id", "report-run-0");
    await expect(submission).toContainText("Output for phase 1");
    await activity.locator(".steps-head").click();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await page.reload();
    await openMonitoredReport(page);
    await expect(activity).toHaveCount(1);
    await expect(activity.locator(".steps-head")).toHaveAttribute("aria-expanded", "false");
    await expect(activity.locator(".steps-meta")).toHaveText("12m 10s");
  });
}

for (const mode of ["active", "unowned"] as const) {
  test(`a ${mode} Run remains visible without repeating the turn clock`, async ({ page }) => {
    await page.addInitScript(tauriMock);
    await page.addInitScript(monitoredReportFixture, { [mode]: true });
    await page.goto("/");
    await openMonitoredReport(page);
    await expect(page.getByTestId("run-monitor-card")).toHaveAttribute("data-run-id", "report-run-0");
    await expect(page.locator(".activity-summary")).toHaveCount(2);
    await expect(page.locator(".activity-summary .steps-meta")).toHaveText(["12m 10s"]);
    if (mode === "active") {
      await page.evaluate(() => Object.assign((window as any).__mockRuns[0], {
        status: "succeeded", ended_at: 1_790_606_451, exit_code: 0,
      }));
      await expect(page.getByTestId("run-monitor-card")).toHaveCount(0);
      await expect(page.locator(".activity-summary")).toHaveCount(1);
      await expect(page.locator(".activity-summary .steps-meta")).toHaveText(["12m 10s"]);
    }
  });
}

test("folding a newly completed monitored Run preserves its results-review prompt", async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.addInitScript(monitoredReportFixture, { active: true, review: true });
  await page.goto("/");
  await openMonitoredReport(page);
  await expect(page.getByTestId("run-monitor-card")).toHaveAttribute("data-run-id", "report-run-0");
  await page.evaluate(() => Object.assign((window as any).__mockRuns[0], {
    status: "succeeded", ended_at: 1_790_606_451, exit_code: 0,
  }));
  await expect(page.locator(".activity-summary")).toHaveCount(1);
  await expect(page.getByTestId("run-review-modal")).toBeVisible();
  await expect(page.getByTestId("run-review-modal")).toContainText("report.tsv");
});

for (const width of [1280, 540]) {
  test(`completed report folds recorded phases across usage and compaction at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.addInitScript(tauriMock);
    await page.goto("/");
    await expect.poll(() => page.evaluate(() => Boolean((window as any).__tauriListenerReady?.("open-session")))).toBe(true);
    await page.evaluate(() => {
      const w = window as any;
      const invoke = w.__TAURI__.core.invoke;
      const usage = { role: "usage", text: JSON.stringify({ input: 100, output: 10 }) };
      w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
        const id = args instanceof Map ? args.get("id") : args?.id;
        if (cmd === "load_session" && id === "phase-report") return {
          items: [
            { role: "user", text: "Analyze the trajectory and write a report" },
            ...Array.from({ length: 6 }, (_, phase) => [
              { role: "assistant", text: `Checking phase ${phase + 1}` },
              { role: "assistant", text: "" },
              { role: "reasoning", text: `Reasoning for phase ${phase + 1}` },
              { role: "tool", tool_name: "python", ok: true, input: "analyze()", text: `Phase ${phase + 1} results`, duration_ms: 50 },
              { role: "file_changed", text: `results/phase_${phase + 1}.csv` },
              usage,
              { role: "app_context", text: JSON.stringify({ contextId: "plot", appName: "plot", state: "ready", summary: "", structuredPreview: null }) },
              ...(phase === 2 ? [{ role: "compaction", text: JSON.stringify({ before: 1000, after: 500, strategy: "auto" }) }] : []),
            ]).flat(),
            { role: "tool", tool_name: "update_plan", ok: true, input: "", text: JSON.stringify({ plan: [{ step: "Write report", status: "completed" }] }) },
            { role: "assistant", text: "## Final trajectory report\n\nAnalysis complete. The figures and methods are ready." },
            usage,
          ], next_before_seq: null, user_offset: 0,
        };
        return invoke(cmd, args);
      };
      w.__tauriEmit("open-session", { projectId: "other", sessionId: "phase-report" });
    });
    const report = page.getByRole("heading", { name: "Final trajectory report" });
    await expect(report).toBeVisible();
    const activity = page.locator(".activity-summary");
    await expect(activity).toHaveCount(1);
    const head = activity.locator(".steps-head");
    await expect(head).toHaveAttribute("aria-expanded", "false");
    await expect(activity.locator(".steps-body")).toHaveCount(0);
    await expect(page.locator(".thread > .usage-row")).toHaveCount(1);
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: `test-results/completed-report-${width}.png`, fullPage: true, animations: "disabled" });
    await head.click();
    await expect(activity.locator(".step-progress")).toHaveCount(6);
    await expect(activity.locator(".usage-row")).toHaveCount(6);
    await expect(activity.getByTestId("context-compaction-flag")).toBeVisible();
    await expect(activity.locator(".execution-plan")).toBeVisible();
    await expect(activity.locator(".step-name")).toContainText([
      "progress", "thinking", "python", "progress", "thinking", "python",
      "progress", "thinking", "python", "progress", "thinking", "python",
      "progress", "thinking", "python", "progress", "thinking", "python",
    ]);
    await head.click();
    await expect(report).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  });
}
