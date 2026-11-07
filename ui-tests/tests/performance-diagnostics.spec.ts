import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

test("health reports interval stalls and switch counters without transcript content", async ({ page }) => {
  await page.route("**/__performance-test/**", async route => {
    const file = new URL(route.request().url()).pathname.split("/").at(-1)!;
    await route.fulfill(file === "index.html"
      ? { contentType: "text/html", body: "<!doctype html><body></body>" }
      : { contentType: "text/javascript", body: readFileSync(resolve(__dirname, "../../ui/src", file), "utf8") });
  });
  await page.goto("/__performance-test/index.html");
  await page.evaluate(async () => {
    const w = window as any;
    w.snapshots = [];
    w.__TAURI__ = { core: { invoke: async (_cmd: string, args: any) => { w.snapshots.push(args.snapshot); } } };
    w.PerformanceObserver = class {
      static supportedEntryTypes = ["longtask"];
      constructor(callback: any) { w.longTask = (duration: number) => callback({ getEntries: () => [{ duration }] }); }
      observe() {}
    };
    const moduleUrl = "/__performance-test/api.js";
    w.diagnostics = await import(moduleUrl);
    w.diagnostics.start_ui_health();
  });
  await page.evaluate(() => {
    const w = window as any;
    w.longTask(900);
    w.diagnostics.report_ui_health();
  });
  await expect.poll(() => page.evaluate(() => (window as any).snapshots.at(-1).recentLongestTaskMs)).toBe(900);
  await page.evaluate(() => (window as any).diagnostics.report_ui_health());
  const next = await page.evaluate(() => (window as any).snapshots.at(-1));
  expect(next.recentLongestTaskMs).toBe(0);
  expect(next.longestTaskMs).toBe(900);
  await page.evaluate(() => {
    const api = (window as any).diagnostics;
    const stale = api.begin_transcript_switch();
    const token = api.begin_transcript_switch();
    api.record_transcript_projection(50);
    api.finish_transcript_switch(stale, 99, 999);
    api.finish_transcript_switch(token, 1, 3700);
  });
  await expect.poll(() => page.evaluate(() => (window as any).snapshots.at(-1).transcriptItems)).toBe(3700);
  const snapshot = await page.evaluate(() => (window as any).snapshots.at(-1));
  expect(snapshot).toMatchObject({ transcriptRebuilds: 1, transcriptRetries: 1, transcriptRows: 50 });
  expect(snapshot.transcriptSwitchMs).toBeGreaterThanOrEqual(0);
  const summary = await page.evaluate(() => {
    const api = (window as any).diagnostics;
    api.record_ui_fault("a private fault must not enter feedback");
    return JSON.parse(api.ui_performance_summary());
  });
  expect(Object.values(summary).every(value => typeof value === "number")).toBe(true);
  expect(summary).toMatchObject({ transcriptItems: 3700, transcriptRetries: 1 });
});
