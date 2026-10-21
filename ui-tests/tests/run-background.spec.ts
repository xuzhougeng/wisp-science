import { expect, test, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";
import { clickInChat } from "./chat-scroll";

// A Run card moved to the background (src-tauri/src/run_background.rs) ends
// the turn's wait on the Run so the conversation can go on. When the Run ends,
// the backend emits RunBackgroundFinished and appends a notice to the
// conversation as a BackgroundReply.
async function setup(page: Page) {
  await page.addInitScript(tauriMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator(".sidebar").getByRole("button", { name: "New session" })).toBeVisible();
  await page.evaluate(() => {
    const w = window as any;
    const original = w.__TAURI__.core.invoke;
    w.__notifications = [];
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      // serde_wasm_bindgen sends invoke arguments as a Map.
      if (cmd === "notify_user") w.__notifications.push(args instanceof Map ? Object.fromEntries(args) : args);
      return original(cmd, args);
    };
  });
}

test("a Run card moved to the background frees the conversation and reports when the Run ends", async ({ page }) => {
  await setup(page);
  await page.locator("#composer-input").fill("MONITORRUN");
  await page.getByRole("button", { name: "Send" }).click();
  const card = page.locator('[data-testid="run-monitor-card"][data-run-id="run-local-002"]');
  await expect(card).toBeVisible();
  await expect(card.getByTestId("run-monitor-backgrounded")).toHaveCount(0);

  await clickInChat(card.getByTestId("run-monitor-background"));

  // The wait ends and the turn finishes while the Run keeps executing.
  await expect(page.getByText("The run continues in the background")).toBeVisible();
  await expect(card.locator(".run-status")).toHaveText("Running");
  await expect(card.getByTestId("run-monitor-backgrounded")).toContainText("Running in the background");
  await expect(card.getByTestId("run-monitor-background")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
  expect(await page.evaluate(() =>
    (window as any).__mockRuns.find((run: any) => run.id === "run-local-002").background_requested_at,
  )).toBeTruthy();

  await page.evaluate(() => {
    const w = window as any;
    const run = w.__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, { status: "succeeded", exit_code: 0, ended_at: run.started_at + 65 });
    w.__tauriEmit("agent", {
      kind: "RunBackgroundFinished",
      frame_id: run.frame_id,
      run_id: run.id,
      title: run.title,
      status: "succeeded",
    });
    w.__tauriEmit("agent", {
      kind: "BackgroundReply",
      frame_id: run.frame_id,
      text: "Background Run “Local normalization” succeeded (exit code 0, took 1m 5s).\n\nRun `run-local-002` · local",
    });
  });

  await expect(page.getByText("Background Run “Local normalization” succeeded")).toBeVisible();
  await expect(card.locator(".run-status")).toHaveText("Succeeded");
  await expect(card.getByTestId("run-monitor-backgrounded")).toHaveCount(0);
  await expect.poll(() => page.evaluate(() =>
    (window as any).__notifications.map((call: any) => call.body).join("\n"),
  )).toContain("Background run finished: Local normalization");
});

test("a failed background request is explained on the card and leaves the Run monitored", async ({ page }) => {
  await setup(page);
  // The Run ended between the last poll and the click.
  await page.evaluate(() => {
    const w = window as any;
    const original = w.__TAURI__.core.invoke;
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      if (cmd === "background_run") throw new Error("The Run has already ended");
      return original(cmd, args);
    };
  });
  await page.locator("#composer-input").fill("MONITORRUN");
  await page.getByRole("button", { name: "Send" }).click();
  const card = page.locator('[data-testid="run-monitor-card"][data-run-id="run-local-002"]');
  await clickInChat(card.getByTestId("run-monitor-background"));
  await expect(card.getByTestId("run-monitor-background-error")).toContainText("already ended");
  await expect(card.getByTestId("run-monitor-backgrounded")).toHaveCount(0);
  await expect(card.getByTestId("run-monitor-background")).toBeVisible();
});
