import { test, expect, type FrameLocator, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// A Playwright click is about ten sequential renderer round trips (locator,
// two stability frames, scroll, geometry, hit target, three input acks). On a
// CI runner shared by four workers each of those waits behind other processes,
// so this bound is liveness only: a frozen host still fails here, but host
// responsiveness is measured inside the page (see startHostStallProbe).
const HOST_ACTION_TIMEOUT_MS = 10_000;
// Longest single main-thread task, and longest 50 ms timer lag, the host may
// show while two apps and two streams are live and its controls are used.
// The stalls this guards were multi-second; a gap this long is already a
// visible freeze. Measured on a 4-worker M-series Mac: tasks up to ~90 ms,
// timer lag up to ~230 ms (end-of-turn tasks queue back to back); CI ran the
// same clicks over 4x slower, so this keeps about a 3x margin. Twice the
// single-frame bound of long-session-stress.spec.ts for the same reason.
const HOST_STALL_LIMIT_MS = 3_000;

async function lastArgs(page: Page, command: string) {
  return page.evaluate((cmd) => {
    const plain = (v: any): any => v instanceof Map ? Object.fromEntries([...v].map(([k, v]) => [k, plain(v)]))
      : Array.isArray(v) ? v.map(plain) : v && typeof v === "object"
        ? Object.fromEntries(Object.entries(v).map(([k, v]) => [k, plain(v)])) : v;
    return plain((window as any).__skillInvokeLog?.filter((c: any) => c.cmd === cmd).at(-1)?.args);
  }, command);
}

/** The app finished its initial render: all 96 cards parsed and every
 * thumbnail decoded. A click into a still-loading sandboxed frame measures
 * its image decoding, not the host. */
async function expectAppRendered(app: FrameLocator) {
  await expect(app.locator("article")).toHaveCount(96);
  await expect.poll(() => app.locator("img").evaluateAll((images) =>
    images.every((image) => (image as HTMLImageElement).complete && (image as HTMLImageElement).naturalWidth > 0)))
    .toBe(true);
}

/** Host-side responsiveness: the longest main-thread task and the longest
 * 50 ms timer lag, measured by the page's own clock so Playwright's protocol
 * latency on a busy runner does not count against the host. */
async function startHostStallProbe(page: Page) {
  await page.evaluate(() => {
    const probe = {
      supported: PerformanceObserver.supportedEntryTypes.includes("longtask"),
      longestTaskMs: 0,
      timerLagMs: 0,
      stop: () => {},
    };
    const record = (entries: PerformanceEntryList) => {
      for (const entry of entries) probe.longestTaskMs = Math.max(probe.longestTaskMs, entry.duration);
    };
    const observer = new PerformanceObserver((list) => record(list.getEntries()));
    observer.observe({ type: "longtask" });
    let last = performance.now();
    const timer = setInterval(() => {
      const now = performance.now();
      probe.timerLagMs = Math.max(probe.timerLagMs, now - last - 50);
      last = now;
    }, 50);
    probe.stop = () => {
      clearInterval(timer);
      record(observer.takeRecords());
      observer.disconnect();
    };
    (window as any).__hostStallProbe = probe;
  });
}

async function stopHostStallProbe(page: Page) {
  return page.evaluate(() => {
    const probe = (window as any).__hostStallProbe;
    probe.stop();
    return { supported: probe.supported as boolean, longestTaskMs: probe.longestTaskMs as number, timerLagMs: probe.timerLagMs as number };
  });
}

const stressTicks = (page: Page) => page.evaluate(() => (window as any).__healthStressTicks as number);

test.beforeEach(async ({ page }) => { await page.addInitScript(tauriMock); });

test("two candidate apps and concurrent streams leave host controls responsive", async ({ page }) => {
  await page.goto("/?mockLongPages=8&mockLongRows=40&mockLongRowBytes=2048");
  await page.locator(".proj-card-main").first().click();
  await expect(page.getByText(/Window page 0 row 39/)).toBeVisible();
  await page.locator(".composer-inner textarea").first().fill("HEALTHSTRESS");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect.poll(() => lastArgs(page, "send_message")).toBeTruthy();
  const frameId = String((await lastArgs(page, "send_message")).sessionId);
  await page.evaluate(({ frameId }) => {
    // Local SVG thumbnail images; no service, real MCP or network required.
    const svg = encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><path d="M10 160L80 100L170 130L300 10" fill="none" stroke="green"/></svg>');
    const html = `<!doctype html><body>${Array.from({ length: 96 }, (_, i) =>
      `<article><img width="320" height="180" src="data:image/svg+xml,${svg}"><button onclick="this.textContent='Selected ${i}'">Select ${i}</button></article>`).join("")}
      <script>setInterval(()=>parent.postMessage({jsonrpc:'2.0',method:'ping',id:Date.now()},'*'),100);</script></body>`;
    for (const [suffix, title] of [["a", "Candidates A"], ["b", "Candidates B"]]) {
      (window as any).__tauriEmit("agent", { kind: "ToolPresentation", frame_id: frameId,
        presentation_kind: "mcp_app", payload: { tool: { name: "figure_library_search", title },
          arguments: {}, result: { content: [] },
          resource: { uri: `ui://figure-library/candidates-${suffix}.html`, text: html, _meta: {} } } });
    }
  }, { frameId });
  const tabs = page.locator('.center-tab[data-center-path^="mcp-app:"]');
  await expect(tabs).toHaveCount(2);
  // Mount each instance, leaving A alive in the parking root while B is visible.
  await tabs.filter({ hasText: "Candidates A" }).click();
  await expectAppRendered(page.frameLocator('iframe[title="Candidates A"]'));
  await tabs.filter({ hasText: "Candidates B" }).click();
  const candidates = page.frameLocator('iframe[title="Candidates B"]');
  await expectAppRendered(candidates);
  await candidates.getByRole("button", { name: "Select 0", exact: true }).click({ timeout: HOST_ACTION_TIMEOUT_MS });
  await expect(candidates.getByRole("button", { name: "Selected 0", exact: true })).toBeVisible();
  await expect.poll(async () => (await lastArgs(page, "ui_heartbeat"))?.snapshot, { timeout: 10_000 })
    .toMatchObject({ activeApps: 1, parkedApps: 1, dragOverlays: 0 });
  expect((await lastArgs(page, "ui_heartbeat")).snapshot.appMessages).toBeGreaterThan(0);
  await page.screenshot({ path: test.info().outputPath("combined-load.png") });
  // Assert during the stream, not only a rAF after completion: keep both apps
  // and both streams live under the probe for ten more stream ticks (the
  // host's own 50 ms timer has to keep firing), then use the host controls.
  await startHostStallProbe(page);
  const ticksAtStart = await stressTicks(page);
  await expect.poll(() => stressTicks(page), { timeout: 10_000 }).toBeGreaterThanOrEqual(ticksAtStart + 10);
  await page.getByRole("button", { name: "Stop", exact: true }).click({ timeout: HOST_ACTION_TIMEOUT_MS });
  await expect.poll(() => lastArgs(page, "stop_agent")).toMatchObject({ sessionId: frameId });
  await tabs.filter({ hasText: "Candidates B" }).locator("..").locator(".center-tab-close").click({ timeout: HOST_ACTION_TIMEOUT_MS });
  await expect(tabs).toHaveCount(1);
  await page.locator(".sidebar").getByRole("button", { name: "New session", exact: true }).click({ timeout: HOST_ACTION_TIMEOUT_MS });
  await page.locator(".composer-inner textarea").first().fill("host still accepts input");
  await expect(page.locator(".composer-inner textarea").first()).toHaveValue("host still accepts input");
  const stall = await stopHostStallProbe(page);
  expect(stall.supported).toBe(true);
  expect(stall.longestTaskMs).toBeLessThan(HOST_STALL_LIMIT_MS);
  expect(stall.timerLagMs).toBeLessThan(HOST_STALL_LIMIT_MS);
});

test("a same-tick burst of turn events does not trap the wasm renderer", async ({ page }) => {
  // Disposed-owner effects used to panic here. Each panic is a wasm trap that
  // leaks shadow stack until the window dies (vendor/leptos_reactive/WISP-PATCH.md).
  const traps: string[] = [];
  page.on("pageerror", (error) => traps.push(error.message));
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await page.locator(".composer-inner textarea").first().fill("ARTIFACTATTRIBUTION");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByText(/I inspected/).first()).toBeVisible();
  await page.evaluate(() => new Promise(requestAnimationFrame));
  expect(traps).toEqual([]);
});

test("heartbeat diagnoses script failures and input blockers without exporting content", async ({ page }) => {
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await page.evaluate(() => {
    window.dispatchEvent(new ErrorEvent("error", { message: "private prompt must not be logged" }));
    const overlay = document.createElement("div");
    overlay.className = "drag-overlay";
    document.body.appendChild(overlay);
  });
  await expect.poll(async () => (await lastArgs(page, "ui_heartbeat"))?.snapshot, { timeout: 10_000 })
    .toMatchObject({ scriptErrors: 1, dragOverlays: 1, firstFault: "" });
  const snapshot = (await lastArgs(page, "ui_heartbeat")).snapshot;
  expect(Object.entries(snapshot).every(([key, value]) =>
    key === "firstFault" ? typeof value === "string" : typeof value === "number")).toBe(true);
  expect(JSON.stringify(snapshot)).not.toContain("private prompt");
  // Only a wasm trap (or a Rust panic, through the panic hook) is quoted: the
  // first one wins and only its first line, so a stack or a later trap cannot
  // grow the log.
  await page.evaluate(() => {
    window.dispatchEvent(new ErrorEvent("error", { message: "trap", error: new WebAssembly.RuntimeError("unreachable\nStack: private frames") }));
    window.dispatchEvent(new ErrorEvent("error", { message: "trap", error: new WebAssembly.RuntimeError("second trap") }));
  });
  await expect.poll(async () => (await lastArgs(page, "ui_heartbeat"))?.snapshot.firstFault, { timeout: 10_000 })
    .toBe("RuntimeError: unreachable");
  expect(JSON.stringify((await lastArgs(page, "ui_heartbeat")).snapshot)).not.toContain("private");
});
