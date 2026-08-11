import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// Structured tool details: a tool's final `details` payload rides the
// persisted ToolResultDetails patch and lands on the tool row. For
// monitor_run the card prefers the structured RunRecord over scraping text,
// so a run the poll list does not know still renders its real status.

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function enterApp(page: Page) {
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator(".composer-inner").first()).toBeVisible();
}

async function emitAgent(page: Page, payload: unknown) {
  await expect.poll(() => page.evaluate(() =>
    Boolean((window as any).__tauriListenerReady?.("agent"))
  )).toBe(true);
  await page.evaluate((value) => {
    (window as any).__tauriEmit("agent", value);
  }, payload);
}

// End the open run the way the real backend does: Done event first, then the
// pending send_message invoke resolves.
async function finishRun(page: Page, frameId: string) {
  await emitAgent(page, { kind: "Done", frame_id: frameId });
  await page.evaluate((fid) => {
    (window as any).__draftRunResolvers?.[fid]?.(fid);
  }, frameId);
}

async function startOpenRun(page: Page, message: string) {
  await page.locator("#composer-input").fill(message);
  await page.getByRole("button", { name: "Send" }).click();
  const sent = await page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "send_message");
    return calls.at(-1)?.args ?? null;
  });
  const frameId = String(sent?.sessionId ?? sent?.session_id ?? "");
  expect(frameId).not.toBe("");
  await expect(page.locator(".msg.assistant").first()).toBeVisible();
  return frameId;
}

test("monitor card renders structured status from final details", async ({ page }) => {
  await enterApp(page);
  const frameId = await startOpenRun(page, "DETAILS monitor card");

  // A run the mocked `list_runs` poll does NOT know: only the details patch
  // can supply the record, so the card proves it consumed structured details.
  const record = {
    id: "run-details-777",
    frame_id: frameId,
    context_id: "local",
    title: "Details-driven run",
    kind: "local",
    status: "succeeded",
    command: "echo done",
    created_at: 2000,
    started_at: 2000,
    ended_at: 2001,
    exit_code: 0,
    stdout_tail: "done",
    stderr_tail: "",
    remote_workdir: null,
    timeout_secs: null,
    last_polled_at: 2001,
    last_poll_error: null,
    progress_json: "{}",
    env_snapshot_json: "{}",
  };

  await emitAgent(page, {
    kind: "ToolExecutionStarted", frame_id: frameId, call_key: "1:0", name: "monitor_run",
  });
  await emitAgent(page, {
    kind: "ToolCall", frame_id: frameId, name: "monitor_run", preview: "run-details-777",
  });
  // Live-only progress: updates the row but is never the persisted shape.
  await emitAgent(page, {
    kind: "ToolProgress", frame_id: frameId, call_key: "1:0",
    details: { ...record, status: "running", ended_at: null, exit_code: null },
  });
  await emitAgent(page, {
    kind: "ToolResult", frame_id: frameId, name: "monitor_run", ok: true,
    content: "Run run-details-777 (\"Details-driven run\") finished with status succeeded (exit code 0).",
    duration_ms: 5,
  });
  await emitAgent(page, {
    kind: "ToolResultDetails", frame_id: frameId, call_key: "1:0", name: "monitor_run",
    details: record,
  });

  const card = page.locator('[data-testid="run-monitor-card"][data-run-id="run-details-777"]');
  await expect(card).toBeVisible();
  await expect(card.locator(".run-status.succeeded").first()).toBeVisible();
  await expect(card.locator(".run-monitor-title")).toContainText("Details-driven run");

  await finishRun(page, frameId);
});
