import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// Tool-call argument draft streaming: while the model is still streaming a
// call's arguments, the host emits live-only ToolCallDraft events and the UI
// shows an in-progress step row with the tool's safe preview. Drafts are keyed
// by call_key (`{round}:{index}`), superseded by the real ToolCall, and dropped
// at turn boundaries so no ghost rows remain.

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

// DRAFTSTREAM starts the mocked turn but never emits Done, so the test drives
// draft / clear / Done events deterministically while the run stays live.
async function startOpenRun(page: Page, message: string) {
  await page.locator("#composer-input").fill(message);
  await page.getByRole("button", { name: "Send" }).click();
  const sent = await page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "send_message");
    return calls.at(-1)?.args ?? null;
  });
  const frameId = String(sent?.sessionId ?? sent?.session_id ?? "");
  expect(frameId).not.toBe("");
  // The turn is underway once the user echo and the first assistant line render.
  await expect(page.locator(".msg.assistant").first()).toBeVisible();
  return frameId;
}

test("draft row with safe preview is visible before the run completes", async ({ page }) => {
  await enterApp(page);
  const frameId = await startOpenRun(page, "DRAFTSTREAM draft row");

  await emitAgent(page, {
    kind: "ToolCallDraft", frame_id: frameId, call_key: "1:0",
    name: "read", preview: "data/counts.tsv",
  });
  const draftRow = page.locator(".step", { hasText: "data/counts.tsv" });
  await expect(draftRow).toBeVisible();
  // An in-progress row, not a finished one, and no Done has arrived.
  await expect(draftRow.locator(".run-dot")).toBeVisible();

  // A later snapshot for the same call_key updates the row in place.
  await emitAgent(page, {
    kind: "ToolCallDraft", frame_id: frameId, call_key: "1:0",
    name: "read", preview: "data/counts-final.tsv",
  });
  await expect(page.locator(".step", { hasText: "data/counts-final.tsv" })).toBeVisible();
  await expect(page.locator(".step", { hasText: "data/counts.tsv" })).toHaveCount(0);

  await finishRun(page, frameId);
});

test("two drafts of the same tool render as two rows keyed separately", async ({ page }) => {
  await enterApp(page);
  const frameId = await startOpenRun(page, "DRAFTSTREAM two same-name drafts");

  await emitAgent(page, {
    kind: "ToolCallDraft", frame_id: frameId, call_key: "1:0",
    name: "shell", preview: "make first",
  });
  await emitAgent(page, {
    kind: "ToolCallDraft", frame_id: frameId, call_key: "1:1",
    name: "shell", preview: "make second",
  });
  await expect(page.locator(".step", { hasText: "make first" })).toBeVisible();
  await expect(page.locator(".step", { hasText: "make second" })).toBeVisible();

  // Updating one key must not touch the other row.
  await emitAgent(page, {
    kind: "ToolCallDraft", frame_id: frameId, call_key: "1:0",
    name: "shell", preview: "make first v2",
  });
  await expect(page.locator(".step", { hasText: "make first v2" })).toBeVisible();
  await expect(page.locator(".step", { hasText: "make second" })).toBeVisible();

  // The real call for 1:0 supersedes its draft; the sibling draft survives.
  await emitAgent(page, { kind: "ToolCallDraftClear", frame_id: frameId, call_key: "1:0" });
  await emitAgent(page, {
    kind: "ToolCall", frame_id: frameId, name: "shell", preview: "make first v2",
  });
  await expect(page.locator(".step", { hasText: "make first v2" })).toHaveCount(1);
  await expect(page.locator(".step", { hasText: "make second" })).toBeVisible();

  await finishRun(page, frameId);
});

test("drafts disappear when the run ends without the tool executing", async ({ page }) => {
  await enterApp(page);
  const frameId = await startOpenRun(page, "DRAFTSTREAM ghost drafts");

  // Mid-run round boundary (host sends call_key: null): both drafts go.
  await emitAgent(page, {
    kind: "ToolCallDraft", frame_id: frameId, call_key: "1:0",
    name: "read", preview: "ghost/one.txt",
  });
  await emitAgent(page, {
    kind: "ToolCallDraft", frame_id: frameId, call_key: "1:1",
    name: "read", preview: "ghost/two.txt",
  });
  await expect(page.locator(".step", { hasText: "ghost/one.txt" })).toBeVisible();
  await emitAgent(page, { kind: "ToolCallDraftClear", frame_id: frameId, call_key: null });
  await expect(page.locator(".step", { hasText: "ghost/one.txt" })).toHaveCount(0);
  await expect(page.locator(".step", { hasText: "ghost/two.txt" })).toHaveCount(0);

  // A draft that never becomes a real call leaves no row once the run ends.
  await emitAgent(page, {
    kind: "ToolCallDraft", frame_id: frameId, call_key: "2:0",
    name: "read", preview: "ghost/pending.txt",
  });
  await expect(page.locator(".step", { hasText: "ghost/pending.txt" })).toBeVisible();
  await finishRun(page, frameId);
  // The steps panel collapses into the activity summary once the turn settles;
  // expand whatever remains and prove no ghost row survived.
  const panel = page.locator(".steps.activity-summary").last();
  if (await panel.count()) {
    await panel.locator(".steps-head").click();
  }
  await expect(page.locator(".step", { hasText: "ghost/pending.txt" })).toHaveCount(0);
});
