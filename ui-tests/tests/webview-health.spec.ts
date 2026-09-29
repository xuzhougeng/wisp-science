import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

async function lastArgs(page: Page, command: string) {
  return page.evaluate((cmd) => {
    const plain = (v: any): any => v instanceof Map ? Object.fromEntries([...v].map(([k, v]) => [k, plain(v)]))
      : Array.isArray(v) ? v.map(plain) : v && typeof v === "object"
        ? Object.fromEntries(Object.entries(v).map(([k, v]) => [k, plain(v)])) : v;
    return plain((window as any).__skillInvokeLog?.filter((c: any) => c.cmd === cmd).at(-1)?.args);
  }, command);
}

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
  await expect(page.frameLocator('iframe[title="Candidates A"]').locator("article")).toHaveCount(96);
  await tabs.filter({ hasText: "Candidates B" }).click();
  const candidates = page.frameLocator('iframe[title="Candidates B"]');
  await candidates.getByRole("button", { name: "Select 0", exact: true }).click({ timeout: 2000 });
  await expect(candidates.getByRole("button", { name: "Selected 0", exact: true })).toBeVisible();
  await expect.poll(async () => (await lastArgs(page, "ui_heartbeat"))?.snapshot, { timeout: 10_000 })
    .toMatchObject({ activeApps: 1, parkedApps: 1, dragOverlays: 0 });
  expect((await lastArgs(page, "ui_heartbeat")).snapshot.appMessages).toBeGreaterThan(0);
  await page.screenshot({ path: test.info().outputPath("combined-load.png") });
  // Assert during the stream, not only a rAF after completion.
  await page.getByRole("button", { name: "Stop", exact: true }).click({ timeout: 2000 });
  await expect.poll(() => lastArgs(page, "stop_agent")).toMatchObject({ sessionId: frameId });
  await tabs.filter({ hasText: "Candidates B" }).locator("..").locator(".center-tab-close").click({ timeout: 2000 });
  await expect(tabs).toHaveCount(1);
  await page.locator(".sidebar").getByRole("button", { name: "New session", exact: true }).click({ timeout: 2000 });
  await page.locator(".composer-inner textarea").first().fill("host still accepts input");
  await expect(page.locator(".composer-inner textarea").first()).toHaveValue("host still accepts input");
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
    .toMatchObject({ scriptErrors: 1, dragOverlays: 1 });
  const snapshot = (await lastArgs(page, "ui_heartbeat")).snapshot;
  expect(Object.values(snapshot).every((value) => typeof value === "number")).toBe(true);
  expect(JSON.stringify(snapshot)).not.toContain("private prompt");
});
