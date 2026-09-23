import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

async function setup(page: Page) {
  await page.addInitScript(tauriMock);
  await page.goto("/");
  await expect.poll(() => page.evaluate(() => Boolean((window as any).__tauriListenerReady?.("open-session")))).toBe(true);
  await page.evaluate(() => {
    const w = window as any;
    const invoke = w.__TAURI__.core.invoke;
    w.hydration = { loads: 0, snapshotGate: null, fail: false, text: "Latest results ready for transfer", approval: true };
    w.approval = { approval_id: "approval-1", frame_id: "live-session", tool: "transfer_between_contexts", preview: "CPU3 results -> local results", message: "Run tool 'transfer_between_contexts'?" };
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
      if (cmd === "list_sessions_page") return {
        items: [{ id: "live-session", title: "Running research", ts: 1, folder_id: null }],
        next_cursor: null, running_ids: ["live-session"],
      };
      if (cmd === "load_session" && arg("id") === "live-session") {
        const state = { ...w.hydration };
        w.hydration.snapshotGate = null;
        w.hydration.loads++;
        if (state.snapshotGate) await state.snapshotGate;
        if (state.fail) throw new Error("snapshot unavailable");
        return {
          items: [{ role: "user", text: "Analyze CRA002586" }, { role: "assistant", text: state.text }],
          next_before_seq: null, user_offset: 0,
          pending_approvals: state.approval ? [w.approval] : [],
        };
      }
      return invoke(cmd, args);
    };
  });
}

async function open(page: Page, session = "live-session") {
  await page.evaluate(sessionId => (window as any).__tauriEmit("open-session", { projectId: "other", sessionId }), session);
}

async function holdNextSnapshot(page: Page) {
  await page.evaluate(() => {
    const state = (window as any).hydration;
    state.snapshotGate = new Promise<void>(resolve => { state.releaseSnapshot = resolve; });
  });
}

test("a cold window hydrates a running conversation and its existing approval", async ({ page }) => {
  await setup(page);
  await holdNextSnapshot(page);
  await open(page);
  await expect(page.getByTestId("transcript-loading")).toBeVisible();
  await expect(page.locator(".chat .empty")).toHaveCount(0);
  await page.evaluate(() => (window as any).hydration.releaseSnapshot());
  await expect(page.getByText("Latest results ready for transfer", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Deny", exact: true })).toBeVisible();
  await expect(page.getByTestId("transcript-loading")).toHaveCount(0);
  await page.screenshot({ path: "test-results/session-hydration.png", fullPage: true });
});

test("returning to a running conversation replaces stale window cache", async ({ page }) => {
  await setup(page);
  await open(page);
  await expect(page.getByText("Latest results ready for transfer", { exact: true })).toBeVisible();
  await open(page, "idle-session");
  await page.evaluate(() => { (window as any).hydration.text = "Newer completed analysis"; });
  await open(page);
  await expect(page.getByText("Newer completed analysis", { exact: true })).toBeVisible();
  await expect(page.getByText("Latest results ready for transfer", { exact: true })).toHaveCount(0);
});

test("an approval resolved during hydration is not resurrected by a late snapshot", async ({ page }) => {
  await setup(page);
  await holdNextSnapshot(page);
  await open(page);
  await expect.poll(() => page.evaluate(() => (window as any).hydration.loads)).toBe(1);
  await page.evaluate(() => {
    const w = window as any;
    w.hydration.approval = false;
    w.__tauriEmit("confirm-resolved", w.approval);
    w.hydration.releaseSnapshot();
  });
  await expect.poll(() => page.evaluate(() => (window as any).hydration.loads)).toBeGreaterThan(1);
  await expect(page.getByTestId("transcript-loading")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Deny", exact: true })).toHaveCount(0);
  await expect(page.getByText("Latest results ready for transfer", { exact: true })).toBeVisible();
});

test("live events invalidate an older snapshot without duplicating deltas", async ({ page }) => {
  await setup(page);
  await holdNextSnapshot(page);
  await page.evaluate(() => { (window as any).hydration.approval = false; });
  await open(page);
  await expect.poll(() => page.evaluate(() => (window as any).hydration.loads)).toBe(1);
  await page.evaluate(() => {
    const w = window as any;
    w.hydration.text = "Latest results plus streamed update";
    w.__tauriEmit("agent", { kind: "Text", frame_id: "live-session", delta: " plus streamed update" });
    w.hydration.releaseSnapshot();
  });
  await expect(page.getByText("Latest results plus streamed update", { exact: true })).toBeVisible();
  await expect(page.getByTestId("transcript-loading")).toHaveCount(0);
  await expect(page.locator(".msg.assistant .body")).toHaveCount(1);
});

test("native approval resolution in another window removes the restored card", async ({ page }) => {
  await setup(page);
  await open(page);
  await expect(page.getByRole("button", { name: "Deny", exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).__tauriEmit("confirm-resolved", (window as any).approval));
  await expect(page.getByRole("button", { name: "Deny", exact: true })).toHaveCount(0);
});

test("a cold window restores a Workflow node approval with one-time scope", async ({ page }) => {
  await setup(page);
  await page.evaluate(() => {
    Object.assign((window as any).approval, {
      tool: "run_in_context", preview: "python verify.py",
      message: "Workflow node verify requests confirmation:\nRun the verification command?",
    });
  });
  await open(page);
  await expect(page.getByTestId("workflow-approval-node")).toHaveText("Workflow node: verify");
  await expect(page.getByLabel("Approval scope")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Allow once", exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).__tauriEmit("confirm-resolved", (window as any).approval));
  await expect(page.getByRole("button", { name: "Deny", exact: true })).toHaveCount(0);
  await expect(page.getByText("Latest results ready for transfer", { exact: true })).toBeVisible();
});

test("failed hydration shows an error with retry instead of the welcome screen", async ({ page }) => {
  await setup(page);
  await page.evaluate(() => { (window as any).hydration.fail = true; });
  await open(page);
  await expect(page.getByRole("alert")).toContainText("snapshot unavailable");
  await expect(page.locator(".empty-logo")).toHaveCount(0);
  await page.evaluate(() => { (window as any).hydration.fail = false; });
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(page.getByText("Latest results ready for transfer", { exact: true })).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("a failed conversation does not hide the welcome screen in a new conversation", async ({ page }) => {
  await setup(page);
  await page.evaluate(() => { (window as any).hydration.fail = true; });
  await open(page);
  await expect(page.getByRole("alert")).toContainText("snapshot unavailable");
  await page.getByRole("button", { name: /New session/ }).first().click();
  await expect(page.locator(".empty-logo")).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});
