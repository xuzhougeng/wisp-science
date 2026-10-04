import { test, expect } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await page.locator(".sidebar").getByRole("button", {name:"New session",exact:true}).click();
  await expect(page.locator("#composer-input")).toBeEnabled();
});

test("slash timer creates without a model turn; clock supports edit, pause, resume and cancel", async ({ page }) => {
  const input = page.locator("#composer-input");
  await input.fill("/timer 1h 当前进展如何");
  await input.press("Enter");
  const clock = page.getByTestId("session-timer-button");
  await expect(clock).toContainText("1h");
  await expect(input).toHaveValue("");
  const saved = await page.evaluate(() => (window as any).__sessionTimer);
  expect(saved.prompt).toBe("当前进展如何");
  expect(saved.frame_id).toBeTruthy();
  expect(await page.evaluate(() => ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "send_message").length)).toBe(0);
  await clock.click();
  await page.screenshot({path:"../test-results/session-timer-panel.png", fullPage:true});
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("session-timer-panel")).toHaveCount(0);
  await expect(clock).toBeVisible();
  await expect(input).toBeVisible();
  await clock.click();
  await page.getByTestId("timer-interval").fill("30m");
  await page.getByTestId("timer-prompt").fill("检查最新日志");
  await page.getByTestId("timer-save").click();
  await expect(clock).toContainText("30m");
  expect((await page.evaluate(() => (window as any).__sessionTimer)).id).toBe(saved.id);
  await page.getByTestId("timer-toggle").click();
  await expect(clock).toContainText("paused");
  await page.getByTestId("timer-toggle").click();
  await expect(clock).toContainText("30m");
  await page.getByTestId("timer-delete").click();
  await expect(clock).toHaveCount(0);
  await expect(page.getByTestId("session-timer-panel")).toHaveCount(0);
});

test("invalid intervals and backend errors keep the draft and do not send a model message", async ({ page }) => {
  const input = page.locator("#composer-input");
  for (const command of ["/timer 0h status", "/timer 1h", "/timer 1.5h status"]) {
    await input.fill(command);
    await input.press("Enter");
    await expect(input).toHaveValue(command);
    await expect(page.getByTestId("session-timer-button")).toHaveCount(0);
  }
  await page.evaluate(() => { (window as any).__timerError = "Timer storage unavailable"; });
  await input.fill("/timer 1h status");
  await input.press("Enter");
  await expect(input).toHaveValue("/timer 1h status");
  await expect(page.getByTestId("session-timer-button")).toHaveCount(0);
});

test("a replacement removes the old complete timer turn and preserves a manual follow-up", async ({ page }) => {
  const input = page.locator("#composer-input");
  await input.fill("/timer 1h status");
  await input.press("Enter");
  await expect(page.getByTestId("session-timer-button")).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__tauriListenerReady("session-timer-replaced"))).toBe(true);
  // Start with a new empty conversation's live transcript.
  await page.evaluate(() => {
    const w = window as any, id = w.__sessionTimer.frame_id;
    const emit = (kind: string, rest: any) => w.__tauriEmit("agent",{kind,frame_id:id,...rest});
    emit("User",{text:"old timer question"});
    emit("Text",{delta:"old timer answer"});
    emit("Done",{});
    emit("User",{text:"manual follow-up to retain"});
    emit("Text",{delta:"manual answer to retain"});
    emit("Done",{});
  });
  await expect(page.getByText("old timer answer", {exact:true})).toBeVisible();
  await page.evaluate(() => {
    const w = window as any, id = w.__sessionTimer.frame_id;
    w.__tauriEmit("session-timer-replaced", {frame_id:id,first_user_index:0,user_count:1,base_epoch:0});
    w.__tauriEmit("agent", {kind:"User",frame_id:id,text:"new timer question"});
    w.__tauriEmit("agent", {kind:"Text",frame_id:id,delta:"new timer answer"});
    w.__tauriEmit("agent", {kind:"Done",frame_id:id});
  });
  await expect(page.getByText("old timer question", {exact:true})).toHaveCount(0);
  await expect(page.getByText("old timer answer", {exact:true})).toHaveCount(0);
  await expect(page.locator("#chat-thread").getByText("manual follow-up to retain", {exact:true})).toBeVisible();
  await expect(page.getByText("manual answer to retain", {exact:true})).toBeVisible();
  await expect(page.getByText("new timer answer", {exact:true})).toBeVisible();
});

test("a background timer cannot switch conversations or erase the current draft", async ({ page }) => {
  const input = page.locator("#composer-input");
  await input.fill("/timer 1h status");
  await input.press("Enter");
  await expect(page.getByTestId("session-timer-button")).toBeVisible();
  const original = await page.evaluate(() => (window as any).__sessionTimer.frame_id);
  await page.locator(".sidebar").getByRole("button", {name:"New session",exact:true}).click();
  await expect(page.getByTestId("session-timer-button")).toHaveCount(0);
  await input.fill("keep this unsent draft");
  await page.evaluate((frame_id) => {
    const emit = (window as any).__tauriEmit;
    emit("session-timer-replaced",{frame_id,first_user_index:0,user_count:1,base_epoch:0});
    emit("agent",{kind:"User",frame_id,text:"background check"});
    emit("agent",{kind:"Text",frame_id,delta:"background result"});
    emit("agent",{kind:"Done",frame_id});
  }, original);
  await expect(input).toHaveValue("keep this unsent draft");
  await expect(page.locator("#chat-thread").getByText("background result",{exact:true})).toHaveCount(0);
  await expect(page.getByTestId("session-timer-button")).toHaveCount(0);
});

test("replacing a timer in paged history keeps the current reading window", async ({ page }) => {
  await page.goto("/?mockLongPages=8");
  await page.locator(".proj-card-main").first().click();
  const input = page.locator("#composer-input");
  await input.fill("/timer 1h status");
  await input.press("Enter");
  await expect(page.getByTestId("session-timer-button")).toBeVisible();
  for (let index = 1; index <= 4; index++) {
    await page.getByRole("button", { name: "Load earlier messages", exact: true }).click();
    await expect(page.getByText(new RegExp(`Window page ${index} row 0 `))).toBeAttached();
  }
  await page.getByRole("button", { name: "Show newer messages", exact: true }).click();
  const reading = page.locator("#chat-thread").getByText(/Window page 2 row 0 /);
  await expect(reading).toBeAttached();
  await page.evaluate(() => {
    const w = window as any;
    // The loaded page starts at absolute user 30; its reading window starts
    // twenty turns later. Removing user 35 must shift that local window by one.
    w.__tauriEmit("session-timer-replaced", {
      frame_id: w.__sessionTimer.frame_id, first_user_index: 35, user_count: 1, base_epoch: 0,
    });
  });
  await expect(reading).toBeAttached();
  await expect(page.locator(".msg.user").first()).toContainText("Window page 2 row 0 ");
});
