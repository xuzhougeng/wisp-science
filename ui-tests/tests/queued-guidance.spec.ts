import { test, expect, type Page } from "@playwright/test";
import { parallelMock } from "./mock-tauri";

async function queueWithDelayedAcceptance(page: Page) {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await page.locator("#composer-input").fill("alpha");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  // The send slot turns into Stop once the turn is running and the draft is gone.
  await expect(page.getByRole("button", { name: "Stop", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Queue…", exact: true })).toBeHidden();
  await page.evaluate(() => {
    const win = window as any;
    const core = win.__TAURI__.core;
    const invoke = core.invoke;
    win.__queueOrder = [];
    core.invoke = async (cmd: string, args: any) => {
      if (cmd === "enqueue_turn") {
        (win.__queueArgs ??= []).push(args instanceof Map ? Object.fromEntries(args) : args);
        win.__queueOrder.push("enqueue-started");
        await new Promise<void>((resolve, reject) => {
          win.__releaseEnqueue = resolve;
          win.__rejectEnqueue = () => reject(new Error("queue unavailable"));
        });
        win.__queueOrder.push("enqueue-accepted");
        return null;
      }
      if (cmd === "stop_agent") {
        win.__queueOrder.push("stop_agent");
        return null;
      }
      if (cmd === "queued_turn_action") {
        win.__queueOrder.push(args instanceof Map ? args.get("action") : args.action);
        if (win.__queueActionError) throw new Error(win.__queueActionError);
        return null;
      }
      return invoke(cmd, args);
    };
  });
  await page.locator("#composer-input").fill("Use the revised question");
  // A draft swaps the Stop icon for the send arrow in the same slot.
  await expect(page.getByRole("button", { name: "Stop", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Queue…", exact: true }).click();
  await expect(page.getByRole("button", { name: "Stop", exact: true })).toBeVisible();
  return page.locator(".msg.user.queued", { hasText: "Use the revised question" });
}

test("Guide now waits for the queued message to be accepted by the backend", async ({ page }) => {
  const row = await queueWithDelayedAcceptance(page);
  await row.getByRole("button", { name: "Guide now", exact: true }).click();
  expect(await page.evaluate(() => (window as any).__queueOrder)).toEqual(["enqueue-started"]);
  await page.evaluate(() => (window as any).__releaseEnqueue());
  await expect.poll(() => page.evaluate(() => (window as any).__queueOrder))
    .toEqual(["enqueue-started", "enqueue-accepted", "cutin"]);
});

test("failed enqueue never dispatches a waiting Guide now action", async ({ page }) => {
  const row = await queueWithDelayedAcceptance(page);
  await row.getByRole("button", { name: "Guide now", exact: true }).click();
  await page.evaluate(() => (window as any).__rejectEnqueue());
  await expect(row).toHaveCount(0);
  await expect(page.locator(".topbar .hint")).toContainText("queue unavailable");
  expect(await page.evaluate(() => (window as any).__queueOrder)).toEqual(["enqueue-started"]);
});

test("Guide now reports command failure and leaves its queued message available", async ({ page }) => {
  const row = await queueWithDelayedAcceptance(page);
  await page.evaluate(() => {
    (window as any).__queueActionError = "guidance unavailable";
    (window as any).__releaseEnqueue();
  });
  await row.getByRole("button", { name: "Guide now", exact: true }).click();
  await expect(page.locator(".topbar .hint")).toContainText("Queue action failed: guidance unavailable");
  await expect(row).toBeVisible();
  await page.evaluate(() => { (window as any).__queueActionError = null; });
  await row.getByRole("button", { name: "Guide now", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__queueOrder))
    .toEqual(["enqueue-started", "enqueue-accepted", "cutin", "cutin"]);
  await expect(page.locator(".topbar .hint")).toHaveCount(0);
});


test("cut-in lifecycle shows its wait and reconciles only the exact queue ID", async ({ page }) => {
  const row = await queueWithDelayedAcceptance(page);
  await page.evaluate(() => (window as any).__releaseEnqueue());
  await expect.poll(() => page.evaluate(() => (window as any).__queueOrder))
    .toContain("enqueue-accepted");
  await page.locator("#composer-input").fill("Use the revised question");
  await page.getByRole("button", { name: "Queue…", exact: true }).click();
  await expect(row).toHaveCount(2);
  await page.evaluate(() => {
    const win = window as any;
    win.__releaseEnqueue();
    const first = win.__queueArgs[0];
    // Another session's identical numeric ID must not affect this session.
    win.__emitParallelEvent("queued-turn-state", { sessionId: "elsewhere", id: first.id, state: "cutin_pending" });
  });
  await expect(page.locator(".queue-state")).toHaveCount(0);
  await page.evaluate(() => {
    const win = window as any;
    const consumed = win.__queueArgs[1];
    win.__emitParallelEvent("queued-turn-state", { sessionId: consumed.sessionId, id: consumed.id, state: "cutin_pending" });
  });
  await expect(row.last()).toContainText("Sent · waiting for the current step");
  await expect(row.last().getByRole("button", { name: "Guide now", exact: true })).toHaveCount(0);
  await page.evaluate(() => {
    const win = window as any;
    const consumed = win.__queueArgs[1];
    win.__emitParallelEvent("agent", { kind: "User", frame_id: consumed.sessionId, text: consumed.message, queue_id: consumed.id });
  });
  await expect(row).toHaveCount(1);
  await expect(page.locator(".queue-state")).toHaveCount(0);
  await page.evaluate(() => {
    const win = window as any;
    const remaining = win.__queueArgs[0];
    win.__emitParallelEvent("queued-turn-state", { sessionId: remaining.sessionId, id: remaining.id, state: "superseded" });
  });
  await expect(row).toHaveCount(0);
});

test("a cut-in folded into the running turn clears its waiting row", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await page.locator("#composer-input").fill("alpha");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByRole("button", { name: "Stop", exact: true })).toBeVisible();
  await page.locator("#composer-input").fill("Use the revised question");
  await page.getByRole("button", { name: "Queue…", exact: true }).click();
  const row = page.locator(".msg.user.queued", { hasText: "Use the revised question" });
  await expect(row).toBeVisible();
  await row.getByRole("button", { name: "Guide now", exact: true }).click();
  await expect(row).toContainText("Sent · waiting for the current step");
  // The loop folds the message into the running turn: the parked row and its
  // waiting hint are gone and the text is an ordinary user turn, while the
  // original turn is still running.
  await expect(page.locator(".queue-state")).toHaveCount(0);
  await expect(row).toHaveCount(0);
  await expect(page.locator(".msg.user:not(.queued)", { hasText: "Use the revised question" })).toHaveCount(1);
  await expect(page.getByRole("button", { name: "Stop", exact: true })).toBeVisible();
});

test("the row menu jumps the queue before it stops the running turn", async ({ page }) => {
  const row = await queueWithDelayedAcceptance(page);
  await page.evaluate(() => (window as any).__releaseEnqueue());
  await expect.poll(() => page.evaluate(() => (window as any).__queueOrder))
    .toContain("enqueue-accepted");
  await row.getByRole("button", { name: "More actions", exact: true }).click();
  await page.getByRole("button", { name: "Interrupt & replace", exact: true }).click();
  // Order matters: the freed session must find this item already at the front.
  await expect.poll(() => page.evaluate(() => (window as any).__queueOrder))
    .toEqual(["enqueue-started", "enqueue-accepted", "move_front", "stop_agent"]);
  await expect(row).toBeVisible();
});

test("the row menu hands a queued message to the side chat", async ({ page }) => {
  const row = await queueWithDelayedAcceptance(page);
  await page.evaluate(() => (window as any).__releaseEnqueue());
  await expect.poll(() => page.evaluate(() => (window as any).__queueOrder))
    .toContain("enqueue-accepted");
  await row.getByRole("button", { name: "More actions", exact: true }).click();
  await page.getByRole("button", { name: "Side chat", exact: true }).click();
  await expect(row).toHaveCount(0);
  await expect(page.locator(".rightpane .sidechat-in-pane")).toContainText("Use the revised question");
  await expect.poll(() => page.evaluate(() => (window as any).__queueOrder))
    .toEqual(["enqueue-started", "enqueue-accepted", "cancel"]);
});

test("Escape closes the row menu without touching the queued message", async ({ page }) => {
  const row = await queueWithDelayedAcceptance(page);
  await row.getByRole("button", { name: "More actions", exact: true }).click();
  await expect(page.getByRole("button", { name: "Interrupt & replace", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: "Interrupt & replace", exact: true })).toHaveCount(0);
  await expect(row).toBeVisible();
});
