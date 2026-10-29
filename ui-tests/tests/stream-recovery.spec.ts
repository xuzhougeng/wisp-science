import { expect, test, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// In-turn model request recovery (#1641): the backend reports a cut stream as
// a `StreamRecovery` event instead of ending the turn. The UI must drop the
// failed attempt's partial reply, show a live, non-terminal notice, and clear
// it once the reconnected request produces output or the turn ends.

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

function composer(page: Page) {
  return page.locator("#composer-input");
}

async function emitTauriEvent(page: Page, event: string, payload: unknown) {
  await expect.poll(() => page.evaluate((name) =>
    Boolean((window as any).__tauriListenerReady?.(name)), event
  )).toBe(true);
  await page.evaluate(({ name, value }) => {
    (window as any).__tauriEmit(name, value);
  }, { name: event, value: payload });
}

async function lastInvokeArgs(page: Page, cmd: string) {
  return page.evaluate((name) => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") {
        return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      }
      return value;
    };
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === name);
    return plain(calls.at(-1)?.args ?? null);
  }, cmd);
}

/// Opens a project, sends one message and returns the frame id the mock
/// replied on. The mock's turn ends with "Hello from mock wisp-science.".
async function startConversation(page: Page, locale: "en" | "zh") {
  await page.goto(`/?mockLocale=${locale}`);
  await page.locator(".proj-card-main").first().click();
  await expect(composer(page)).toBeVisible();
  await composer(page).fill("Run the analysis on the flaky relay");
  await page.getByRole("button", { name: locale === "zh" ? "发送" : "Send", exact: true }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  return String((await lastInvokeArgs(page, "send_message")).sessionId);
}

function recovery(frameId: string, phase: string, overrides: Record<string, unknown> = {}) {
  return {
    kind: "StreamRecovery",
    frame_id: frameId,
    phase,
    cause: "stream",
    attempt: 1,
    max_attempts: 5,
    retry_in_ms: phase === "waiting" ? 1000 : 0,
    error: phase === "waiting" ? "stream ended without completion" : "",
    discarded_text_bytes: 0,
    discarded_reasoning_bytes: 0,
    ...overrides,
  };
}

test("a cut stream drops the partial reply, shows a live notice and recovers in place", async ({ page }) => {
  const frameId = await startConversation(page, "en");
  const partial = "The partial ans";

  await emitTauriEvent(page, "agent", { kind: "Reasoning", frame_id: frameId, delta: "deciding" });
  await emitTauriEvent(page, "agent", { kind: "Text", frame_id: frameId, delta: partial });
  await expect(page.getByText(partial)).toBeVisible();

  await emitTauriEvent(page, "agent", recovery(frameId, "waiting", {
    discarded_text_bytes: partial.length,
    discarded_reasoning_bytes: "deciding".length,
  }));
  const live = page.getByTestId("stream-recovery-live");
  await expect(live).toBeVisible();
  await expect(live).toHaveAttribute("role", "status");
  await expect(live).toHaveAttribute("data-phase", "waiting");
  await expect(live).toContainText("Model response stream interrupted");
  await expect(live).toContainText("(1/5)");
  await expect(live).toContainText("Retrying in 1 s");
  await expect(live).toContainText("Completed tool results are kept");
  await expect(live).toContainText("stream ended without completion");
  // The failed attempt is gone; the completed reply and tool row stay.
  await expect(page.getByText(partial)).toHaveCount(0);
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  // No terminal error card: the turn is still running.
  await expect(page.locator(".finding.err")).toHaveCount(0);

  await emitTauriEvent(page, "agent", recovery(frameId, "reconnecting"));
  await expect(live).toHaveAttribute("data-phase", "reconnecting");
  await expect(live).toContainText("Reconnecting to the model");

  await emitTauriEvent(page, "agent", { kind: "Text", frame_id: frameId, delta: "The full answer." });
  await expect(page.getByTestId("stream-recovery-live")).toHaveCount(0);
  await expect(page.getByText("The full answer.")).toBeVisible();
  await expect(page.getByText(partial)).toHaveCount(0);

  await emitTauriEvent(page, "agent", recovery(frameId, "recovered"));
  await expect(page.locator(".topbar")).toContainText("Model connection recovered after 1 retry");
  await emitTauriEvent(page, "agent", { kind: "Done", frame_id: frameId });
  await expect(page.getByTestId("stream-recovery-live")).toHaveCount(0);
});

test("a provider rejection is shown as a request retry and clears with the turn's error", async ({ page }) => {
  const frameId = await startConversation(page, "zh");

  await emitTauriEvent(page, "agent", recovery(frameId, "waiting", {
    cause: "request",
    attempt: 2,
    retry_in_ms: 10_000,
    error: "api: 503 overloaded",
  }));
  const live = page.getByTestId("stream-recovery-live");
  await expect(live).toContainText("模型请求失败，正在重试（第 2/5 次）");
  await expect(live).toContainText("将在 10 秒后重试");
  await expect(live).toContainText("api: 503 overloaded");
  await expect(page.locator(".topbar")).not.toContainText(/重试|503/);

  // Stop, or an exhausted budget, ends the turn with one error card and no
  // lingering notice.
  await emitTauriEvent(page, "agent", { kind: "Error", frame_id: frameId, message: "stopped by user" });
  await expect(page.getByTestId("stream-recovery-live")).toHaveCount(0);
  await expect(page.locator(".finding.err")).toHaveCount(1);
});

test("the recovery notice stays with its own session", async ({ page }) => {
  await page.goto("/?mockSessionModels=1");
  await page.locator(".proj-card-main").first().click();
  const frameId = "s-model-a";
  await page.locator(`[data-session-id="${frameId}"]`).click();
  await emitTauriEvent(page, "agent", recovery(frameId, "waiting"));
  await expect(page.getByTestId("stream-recovery-live")).toBeVisible();
  await page.locator('[data-session-id="s-model-b"]').click();
  await expect(page.getByTestId("stream-recovery-live")).toHaveCount(0);
  await page.locator(`[data-session-id="${frameId}"]`).click();
  await expect(page.getByTestId("stream-recovery-live")).toBeVisible();
  await emitTauriEvent(page, "agent", { kind: "Done", frame_id: frameId });
  await expect(page.getByTestId("stream-recovery-live")).toHaveCount(0);
});
