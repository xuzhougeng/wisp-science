import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto("/?mockReuse=1");
  await page.locator(".proj-card-main").first().click();
  await page.locator('[data-session-id="reuse-source"]').click();
  await expect(page.locator(".msg.user", { hasText: "first idea" })).toBeVisible();
});

const composer = (page: Page) => page.locator("#composer-input");

async function selectRange(page: Page, reverse = false) {
  const first = page.locator(".msg.user", { hasText: "first idea" });
  const last = page.locator(".msg.user", { hasText: "verify output" });
  await (reverse ? last : first).getByTestId("turn-excerpt").click();
  await (reverse ? first : last).getByTestId("turn-excerpt").click();
  await expect(page.getByTestId("session-reuse-overlay")).toBeVisible();
}

async function lastArgs(page: Page, command: string) {
  return page.evaluate((cmd) => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      return value;
    };
    const calls = (window as any).__skillInvokeLog ?? [];
    return plain([...calls].reverse().find((call: any) => call.cmd === cmd)?.args ?? null);
  }, command);
}

test("quote to an existing conversation preserves both drafts and accumulates quotes", async ({ page }) => {
  await composer(page).fill("Source draft");
  await page.locator('[data-session-id="reuse-target"]').click();
  await composer(page).fill("Target draft");
  await page.locator('[data-session-id="reuse-source"]').click();
  await expect(composer(page)).toHaveValue("Source draft");

  for (let i = 0; i < 2; i++) {
    await selectRange(page, i === 1);
    const target = page.getByTestId("excerpt-destination");
    await expect(target.locator('option[value="reuse-source"]')).toHaveCount(0);
    await expect(target.locator('option[value="reuse-merged"]')).toHaveCount(0);
    await expect(target.locator('option[value="reuse-subagent"]')).toHaveCount(0);
    await page.getByTestId("excerpt-target-search").fill("Another dataset");
    await expect.poll(() => lastArgs(page, "list_sessions_page")).toMatchObject({ query: "Another dataset" });
    await expect(target.locator('option[value="reuse-target"]')).toHaveCount(1);
    await target.selectOption("reuse-target");
    await page.getByTestId("excerpt-attach").click();
    await expect(page.getByTestId("session-reuse-overlay")).toHaveCount(0);
    await expect(composer(page)).toHaveValue("Target draft");
    await expect(page.locator(".composer-reference-card.quote")).toHaveCount(i + 1);
    expect(await lastArgs(page, "send_message")).toBeNull();
    if (i === 0) {
      await page.locator('[data-session-id="reuse-source"]').click();
      await expect(composer(page)).toHaveValue("Source draft");
      await expect(page.locator(".composer-reference-card.quote")).toHaveCount(0);
    }
  }
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect.poll(() => lastArgs(page, "send_message")).toMatchObject({ sessionId: "reuse-target" });
  const sent = await lastArgs(page, "send_message");
  expect(sent.message).toContain("Target draft");
  expect(sent.message).toContain("Mock conversation · turns 1–3");
  expect(sent.message).not.toContain("Source draft");
});

test("a selected range becomes an editable experience that can be saved and reused", async ({ page }) => {
  await selectRange(page);
  await page.getByTestId("excerpt-experience").click();
  await page.getByTestId("excerpt-focus").fill("The validation before normalization");
  await page.getByTestId("excerpt-generate").click();
  await expect(page.getByTestId("excerpt-draft")).toHaveValue(/When to use/);
  expect(await lastArgs(page, "generate_session_experience")).toMatchObject({
    id: "reuse-source", startUserIndex: 0, endUserIndex: 2,
    instruction: "The validation before normalization",
  });
  expect(await lastArgs(page, "confirm_turn_memory")).toBeNull();
  const edited = "Validate input counts, normalize, then compare output rows to input rows.";
  await page.getByTestId("excerpt-draft").fill(edited);
  await page.getByTestId("excerpt-memory-scope").selectOption("global");
  await page.getByTestId("excerpt-save-memory").click();
  await expect(page.getByTestId("excerpt-save-memory")).toHaveText("Saved");
  const saved = await lastArgs(page, "confirm_turn_memory");
  expect(saved).toMatchObject({ sessionId: "reuse-source", turnIndex: 0, scope: "global" });
  expect(saved.content).toContain(edited);
  expect(saved.content).toContain("Mock conversation · turns 1–3 (session reuse-source)");
  await page.getByTestId("excerpt-destination").selectOption("reuse-target");
  await page.getByTestId("excerpt-attach").click();
  await expect(page.locator(".composer-reference-card.quote")).toContainText("turns 1–3");
  expect(await lastArgs(page, "send_message")).toBeNull();
  await composer(page).fill("Apply this procedure to my new dataset");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect.poll(() => lastArgs(page, "send_message")).toMatchObject({ sessionId: "reuse-target" });
  const sent = await lastArgs(page, "send_message");
  expect(sent.message).toContain("Reusable experience distilled from conversation");
  expect(sent.message).toContain(edited);
  expect(sent.message).not.toContain("AI source-edit instruction");
});

test("Escape cancels experience generation without closing the range or accepting a late result", async ({ page }) => {
  await selectRange(page);
  await page.getByTestId("excerpt-experience").click();
  await page.evaluate(() => (window as any).__holdReuseCommand("generate_session_experience"));
  await page.getByTestId("excerpt-generate").click();
  await expect(page.getByTestId("excerpt-generate")).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("session-reuse-overlay")).toHaveCount(0);
  await expect(page.getByTestId("excerpt-selection-banner")).toBeVisible();
  // Open a new dialog before the earlier generation completes.
  await page.locator(".msg.user", { hasText: "verify output" }).getByTestId("turn-excerpt").click();
  await expect(page.getByTestId("session-reuse-overlay")).toBeVisible();
  await page.getByTestId("excerpt-experience").click();
  await page.evaluate(() => (window as any).__releaseReuseCommand("generate_session_experience"));
  await expect(page.getByTestId("excerpt-draft")).toHaveValue("");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("excerpt-selection-banner")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("excerpt-selection-banner")).toHaveCount(0);
  expect(await lastArgs(page, "confirm_turn_memory")).toBeNull();
});

test("a cancelled range cannot open a delayed review dialog", async ({ page }) => {
  await page.evaluate(() => (window as any).__holdReuseCommand("render_session_excerpt"));
  await page.locator(".msg.user", { hasText: "first idea" }).getByTestId("turn-excerpt").click();
  await page.locator(".msg.user", { hasText: "verify output" }).getByTestId("turn-excerpt").click();
  await expect.poll(() => lastArgs(page, "render_session_excerpt")).toMatchObject({ endUserIndex: 2 });
  await page.keyboard.press("Escape");
  await page.evaluate(() => (window as any).__releaseReuseCommand("render_session_excerpt"));
  await expect(page.getByTestId("session-reuse-overlay")).toHaveCount(0);
  await expect(page.getByTestId("excerpt-selection-banner")).toHaveCount(0);
});

test("generation and new-conversation failures preserve the selection for retry", async ({ page }) => {
  await selectRange(page);
  await page.getByTestId("excerpt-experience").click();
  await page.evaluate(() => { (window as any).__failExperience = true; });
  await page.getByTestId("excerpt-generate").click();
  await expect(page.getByTestId("session-reuse-overlay").getByRole("alert")).toContainText("Experience model unavailable");
  await page.evaluate(() => { (window as any).__failExperience = false; });
  await page.getByTestId("excerpt-generate").click();
  await expect(page.getByTestId("excerpt-draft")).toHaveValue(/When to use/);
  await page.evaluate(() => (window as any).__failNextNewSession("Could not create conversation"));
  await page.getByTestId("excerpt-attach").click();
  await expect(page.getByTestId("session-reuse-overlay").getByRole("alert")).toContainText("Could not create conversation");
  await expect(page.getByTestId("excerpt-draft")).toHaveValue(/When to use/);
  await page.getByTestId("excerpt-attach").click();
  await expect(page.getByTestId("session-reuse-overlay")).toHaveCount(0);
  await expect(page.locator(".composer-reference-card.quote")).toHaveCount(1);
});
