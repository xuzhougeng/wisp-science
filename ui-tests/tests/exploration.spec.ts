import { expect, test, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

async function enterExplorationProject(page: Page) {
  await page.goto("/?mockExplorations=1");
  await page.locator(".proj-card-main").first().click();
  const mainline = page.locator('[data-session-id="exploration-mainline"]');
  await expect(mainline).toBeVisible();
  await mainline.click();
  await expect(page.getByText("Mainline result")).toBeVisible();
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function lastInvokeArgs(page: Page, cmd: string) {
  return page.evaluate((name) => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((call: any) => call.cmd === name);
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : (args ?? null);
  }, cmd);
}

test("a sidebar parent folds its explorations and branches behind one chevron", async ({ page }) => {
  await page.goto("/?mockExplorations=1&mockBranches=1&mockOtherExplorationSession=1");
  await page.locator(".proj-card-main").first().click();

  const nest = page.getByTestId("sidebar-nest");
  const toggle = page.getByTestId("sidebar-nest-toggle");
  const explorations = page.getByTestId("sidebar-explorations").locator(".side-exploration");
  const branch = page.locator('.sidebar [data-session-id="conversation-branch"]');
  const count = page.locator(".sidebar .side-nest-count");

  // The open conversation shows its whole family under one chevron, without per-kind headings.
  await expect(toggle).toHaveCount(1);
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(explorations).toHaveCount(2);
  await expect(explorations.first()).toBeVisible();
  await expect(branch).toBeVisible();
  await expect(nest).not.toContainText("Explorations");
  await expect(nest).not.toContainText("Branches");
  await expect(count).toBeHidden();

  // Leaving the family folds it down to a count.
  await page.locator('.sidebar [data-session-id="session-b"]').click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(nest).toBeHidden();
  await expect(count).toBeVisible();
  await expect(count).toHaveText("3");

  // A manual toggle wins over that default, in both directions.
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(explorations.first()).toBeVisible();
  await branch.click();
  await expect(branch).toHaveClass(/active/);
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(nest).toBeHidden();
});

test("inline exploration and branch cards collapse independently", async ({ page }) => {
  await page.goto("/?mockExplorations=1&mockBranches=1");
  await page.locator(".proj-card-main").first().click();
  await page.locator('[data-session-id="exploration-mainline"]').click();

  const explorationCards = page.getByTestId("exploration-message-card");
  const explorationToggle = page.getByTestId("message-exploration-toggle");
  await expect(explorationCards).toHaveCount(2);
  await expect(explorationToggle).toHaveAttribute("aria-expanded", "true");

  await explorationToggle.click();
  await expect(explorationToggle).toHaveAttribute("aria-expanded", "false");
  await expect(explorationCards).toHaveCount(0);
  await expect(page.getByTestId("message-branch-link")).toHaveCount(1);

  await explorationToggle.click();
  await expect(explorationCards).toHaveCount(2);

  const branchToggle = page.getByTestId("message-branch-toggle");
  await expect(branchToggle).toHaveAttribute("aria-expanded", "true");
  await branchToggle.click();
  await expect(page.getByTestId("message-branch-link")).toHaveCount(0);
  await expect(explorationCards).toHaveCount(2);
  await branchToggle.click();
  await expect(page.getByTestId("message-branch-link")).toHaveCount(1);
});

test("exploration sidebar, banners, diff tabs, and Escape stack remain distinct from Branch", async ({ page }) => {
  await enterExplorationProject(page);

  const group = page.getByTestId("sidebar-explorations");
  await expect(group).toBeVisible();
  await expect(group.locator(".side-exploration")).toHaveCount(2);
  const cards = page.getByTestId("exploration-message-card");
  await expect(cards).toHaveCount(2);
  await expect(cards.nth(0)).toContainText("Exploration A");
  await expect(cards.nth(1)).toContainText("Exploration B");
  await expect(page.getByTestId("start-exploration")).toBeVisible();
  await expect(page.locator(".msg-branch-btn").last()).toBeVisible();

  await cards.nth(0).click();
  await expect(page.getByText("Exploration A result")).toBeVisible();
  await expect(page.getByTestId("exploration-banner")).toContainText("Exploration A");

  await page.getByTestId("exploration-banner").getByRole("button", { name: "View diff" }).click();
  const diff = page.getByTestId("exploration-diff-overlay");
  await expect(diff).toBeVisible();
  await expect(diff.getByRole("tab")).toHaveCount(5);
  await diff.getByRole("tab", { name: /Artifacts/ }).click();
  await expect(page.getByTestId("exploration-diff-body")).toContainText("exploration-a/result");
  await diff.getByRole("button", { name: "Set as mainline" }).click();
  const confirm = page.getByTestId("exploration-confirm-overlay");
  await expect(confirm).toBeVisible();
  const confirmZ = Number(await confirm.evaluate((el) => getComputedStyle(el).zIndex));
  const diffZ = Number(await diff.evaluate((el) => getComputedStyle(el).zIndex));
  expect(confirmZ).toBeGreaterThan(diffZ);

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("exploration-confirm-overlay")).toBeHidden();
  await expect(diff).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(diff).toBeHidden();

  await page.locator('[data-session-id="exploration-mainline"]').click();
  const mainlineCards = page.getByTestId("exploration-message-card");
  await mainlineCards.nth(1).click();
  await expect(page.getByText("Exploration B result")).toBeVisible();
  await expect(page.getByTestId("exploration-banner")).toContainText("Exploration B");
});

test("Escape immediately after opening the diff overlay closes only that layer", async ({ page }) => {
  await enterExplorationProject(page);
  await page.getByTestId("exploration-message-card").nth(0).click();
  const banner = page.getByTestId("exploration-banner");
  await expect(banner).toContainText("Exploration A");

  await banner.getByRole("button", { name: "View diff" }).click();
  const diff = page.getByTestId("exploration-diff-overlay");
  await expect(diff).toBeVisible();
  // Escape stack rule: press Escape immediately, before any focus moves into
  // the overlay. One press closes only the topmost layer; the exploration
  // view underneath must stay open.
  await page.keyboard.press("Escape");
  await expect(diff).toBeHidden();
  await expect(banner).toBeVisible();
  await expect(page.getByText("Exploration A result")).toBeVisible();
});

test("an exploration round banner and checkpoint cards stay scoped to its source session", async ({ page }) => {
  await page.goto("/?mockExplorations=1&mockOtherExplorationSession=1");
  await page.locator(".proj-card-main").first().click();

  await page.locator('[data-session-id="exploration-mainline"]').click();
  await expect(page.getByTestId("mainline-exploration-banner")).toBeVisible();
  await expect(page.getByTestId("exploration-message-card")).toHaveCount(2);

  await page.locator('[data-session-id="session-b"]').click();
  await expect(page.getByTestId("mainline-exploration-banner")).toBeHidden();
  await expect(page.getByTestId("exploration-banner")).toBeHidden();
  await expect(page.getByTestId("exploration-message-card")).toHaveCount(0);
});

test("exploration cards expose right-click actions and selecting opens guarded promotion", async ({ page }) => {
  await enterExplorationProject(page);

  const candidate = page
    .getByTestId("sidebar-explorations")
    .locator('[data-exploration-id="exploration-a"]');
  await candidate.click({ button: "right" });
  const menu = page.locator(".ctx-menu").first();
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("button", { name: "Open exploration", exact: true })).toBeVisible();
  await expect(menu.getByRole("button", { name: "Select as mainline", exact: true })).toBeVisible();
  await expect(menu.getByRole("button", { name: "View diff", exact: true })).toBeVisible();
  await expect(menu.getByRole("button", { name: "Discard", exact: true })).toBeVisible();
  await expect(menu.getByRole("button", { name: "Archive", exact: true })).toHaveCount(0);
  await expect(menu.getByRole("button", { name: "Restore", exact: true })).toHaveCount(0);

  await menu.getByRole("button", { name: "Select as mainline", exact: true }).click();
  const diff = page.getByTestId("exploration-diff-overlay");
  await expect(diff).toBeVisible();
  await expect(diff).toContainText("Exploration changes");
  await expect.poll(() => lastInvokeArgs(page, "promote_exploration")).toBeNull();
  await diff.getByRole("button", { name: "Set as mainline", exact: true }).click();
  await expect(page.getByTestId("exploration-confirm-overlay")).toBeVisible();
});

test("blocked promotion offers a guarded manual file recovery flow", async ({ page }) => {
  await page.goto("/?mockExplorations=1&mockMainlineAdvanced=1");
  await page.locator(".proj-card-main").first().click();
  await page
    .getByTestId("sidebar-explorations")
    .locator('[data-exploration-id="exploration-a"]')
    .click();
  await page.getByTestId("exploration-banner").getByRole("button", { name: "View diff" }).click();

  const diff = page.getByTestId("exploration-diff-overlay");
  await expect(diff.getByTestId("exploration-promotion-blocked")).toContainText(
    "Automatic promotion stopped to avoid overwriting it",
  );
  const manual = diff.getByTestId("exploration-manual-resolution");
  await expect(manual).toContainText("Resolve files manually");
  await expect(diff.getByTestId("exploration-promote")).toBeDisabled();

  await manual.getByTestId("exploration-open-manual-folders").click();
  await expect.poll(() => lastInvokeArgs(page, "open_exploration_manual_resolution")).toMatchObject({
    explorationId: "exploration-a",
  });

  await manual.getByTestId("exploration-finish-manual").click();
  const confirm = page.getByTestId("exploration-confirm-overlay");
  await expect(confirm).toContainText("Exploration-only conversation history and structured records will not be merged");
  await page.keyboard.press("Escape");
  await expect(confirm).toBeHidden();
  await expect(diff).toBeVisible();

  await manual.getByTestId("exploration-finish-manual").click();
  await page.getByTestId("exploration-confirm-action").click();
  await expect.poll(() => lastInvokeArgs(page, "abandon_exploration_round")).toMatchObject({
    sourceFrameId: "exploration-mainline",
  });
  await expect(page.getByTestId("sidebar-explorations")).toHaveCount(0);
  await expect(page.locator("#composer-input")).toBeEnabled();
});

test("discard permanently removes the exploration instead of leaving an unwritable tombstone", async ({ page }) => {
  await enterExplorationProject(page);
  const candidate = page
    .getByTestId("sidebar-explorations")
    .locator('[data-exploration-id="exploration-a"]');
  await candidate.click();
  await expect(page.getByText("Exploration A result")).toBeVisible();

  await candidate.click({ button: "right" });
  await page.locator(".ctx-menu").first().getByRole("button", { name: "Discard", exact: true }).click();
  const diff = page.getByTestId("exploration-diff-overlay");
  await expect(diff).toBeVisible();
  await diff.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(page.getByTestId("exploration-confirm-overlay")).toBeVisible();
  await page.getByTestId("exploration-confirm-action").click();

  await expect(page.getByText("Mainline result")).toBeVisible();
  await expect(page.getByText("Exploration A result")).toHaveCount(0);
  await expect(page.locator('[data-exploration-id="exploration-a"]')).toHaveCount(0);
  await expect(page.getByText("Discarded exploration", { exact: true })).toHaveCount(0);
  await expect(page.getByTestId("exploration-message-card")).toHaveCount(1);
  await expect.poll(() => lastInvokeArgs(page, "discard_exploration")).toMatchObject({
    explorationId: "exploration-a",
  });
});

test("conversation branches appear at their checkpoint and expose merge-back actions", async ({ page }) => {
  await page.goto("/?mockExplorations=1&mockBranches=1");
  await page.locator(".proj-card-main").first().click();

  const branch = page.locator('.sidebar [data-session-id="conversation-branch"]');
  const exploration = page.locator('[data-exploration-id="exploration-a"]');
  await expect(branch.locator(".session-branch-icon svg")).toHaveCount(1);
  await expect(exploration.locator(".exploration-kind-icon svg")).toHaveCount(1);
  expect(await branch.locator(".session-branch-icon").innerHTML())
    .not.toBe(await exploration.locator(".exploration-kind-icon").innerHTML());

  await branch.click();
  await expect(page.locator(".msg-branch-btn")).toHaveCount(0);
  await expect(page.locator('.user-bubble [title="Branch"]')).toHaveCount(0);
  await expect(page.getByTestId("start-exploration")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Start exploration", exact: true })).toHaveCount(0);
  await expect(page.getByTestId("exploration-start-overlay")).toHaveCount(0);
  await expect(page.locator("#composer-input")).toBeEnabled();
  // The composer send dropdown was removed: branching a branch is guarded in
  // the slash menu instead, and side chat stays reachable through /btw.
  await expect(page.getByRole("button", { name: "Message options" })).toHaveCount(0);
  await page.locator("#composer-input").pressSequentially("/");
  const slashMenu = page.locator(".mention-menu");
  await expect(slashMenu).toBeVisible();
  await expect(slashMenu).toContainText("/compact");
  await expect(slashMenu).toContainText("/btw");
  await expect(slashMenu).not.toContainText("/fork");
  await page.keyboard.press("Escape");
  await page.locator("#composer-input").fill("/fork nested branch");
  await page.getByRole("button", { name: "Send" }).click();
  expect(await lastInvokeArgs(page, "branch_session")).toBeNull();

  await branch.click({ button: "right" });
  await expect(page.getByRole("button", { name: "Merge back", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Compare branches", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Make independent", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Delete branch", exact: true })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.locator(".ctx-menu")).toBeHidden();
  await expect(branch).toBeVisible();

  await branch.click({ button: "right" });
  await page.getByRole("button", { name: "Merge back", exact: true }).click();
  const merge = page.getByTestId("branch-merge-overlay");
  await expect(merge).toBeVisible();
  await expect(merge.getByTestId("branch-merge-delta")).toContainText("alternate analysis result");
  await expect(merge.locator("textarea")).toHaveValue(/completed its focused analysis/);
  await page.keyboard.press("Escape");
  await expect(merge).toBeHidden();
  await expect(branch).toBeVisible();

  const main = page.locator('.sidebar [data-session-id="exploration-mainline"]');
  await main.click();
  const inlineBranch = page.getByTestId("message-branch-link");
  await expect(inlineBranch).toHaveCount(1);
  await expect(inlineBranch).toContainText("alternate analysis");
});

test("an edited branch-only summary appends to the current main tail and keeps the branch", async ({ page }) => {
  await page.goto("/?mockBranches=1");
  await page.locator(".proj-card-main").first().click();

  const branch = page.locator('.sidebar [data-session-id="conversation-branch"]');
  await branch.click({ button: "right" });
  await page.getByRole("button", { name: "Merge back", exact: true }).click();
  const merge = page.getByTestId("branch-merge-overlay");
  await merge.locator("textarea").fill("Edited branch result ready for main.");
  await merge.getByTestId("branch-merge-action").click();
  await expect.poll(() => lastInvokeArgs(page, "merge_session_branch_summary")).toMatchObject({
    id: "conversation-branch",
    expectedGuardHash: "mock-branch-merge-guard",
    summary: "Edited branch result ready for main.",
  });
  await expect(merge).toBeHidden();
  await expect(page.getByText("Main current result", { exact: true })).toBeVisible();
  await expect(page.getByText("Edited branch result ready for main.", { exact: true })).toHaveCount(0);
  const mergedCard = page.getByTestId("branch-merge-card");
  await expect(mergedCard).toContainText("Merged branch result");
  await expect(mergedCard).not.toContainText("alternate analysis");
  const inlineBranch = page.getByTestId("message-branch-link").filter({ hasText: "alternate analysis" });
  await expect(inlineBranch
    .locator("xpath=..").getByTestId("branch-merge-card")).toHaveCount(1);
  const branchBox = await inlineBranch.boundingBox();
  const mergeBox = await mergedCard.boundingBox();
  expect(branchBox).not.toBeNull();
  expect(mergeBox).not.toBeNull();
  expect(mergeBox!.x).toBeGreaterThan(branchBox!.x);
  expect(mergeBox!.width).toBeLessThan(branchBox!.width);
  expect(mergeBox!.height).toBeLessThanOrEqual(branchBox!.height);
  await mergedCard.click();
  const detail = page.getByTestId("branch-merge-detail-overlay");
  await expect(detail).toContainText("Edited branch result ready for main.");
  await expect(detail.locator(".artifact-modal .am-head")).toBeVisible();
  await expect(detail.locator(".am-figure .rp-heavy.md")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(detail).toBeHidden();
  await expect(mergedCard).toBeVisible();
  await expect(branch).toBeVisible();
  await branch.click();
  await expect(page.locator("#composer-input")).toBeDisabled();
  await expect(page.locator(".msg-branch-btn")).toHaveCount(0);
  await branch.click({ button: "right" });
  await expect(page.getByRole("button", { name: "Merge back", exact: true })).toHaveCount(0);
});

test("branch summaries can be regenerated or revised with explicit guidance", async ({ page }) => {
  await page.goto("/?mockBranches=1");
  await page.locator(".proj-card-main").first().click();

  const branch = page.locator('.sidebar [data-session-id="conversation-branch"]');
  await branch.click({ button: "right" });
  await page.getByRole("button", { name: "Merge back", exact: true }).click();
  const merge = page.getByTestId("branch-merge-overlay");
  const draft = merge.locator("textarea");
  await expect(draft).toHaveValue(/completed its focused analysis/);

  await merge.getByTestId("branch-regenerate").click();
  await expect.poll(() => lastInvokeArgs(page, "summarize_session_branch_merge")).toMatchObject({
    id: "conversation-branch",
    expectedGuardHash: "mock-branch-merge-guard",
  });
  const regenerateArgs = await lastInvokeArgs(page, "summarize_session_branch_merge");
  expect(regenerateArgs.currentVersion).toBeUndefined();
  expect(regenerateArgs.userGuidance).toBeUndefined();

  await draft.fill("Current edited version for main.");
  await merge.getByTestId("branch-guided-generate").click();
  const guidance = page.getByTestId("branch-guidance-overlay");
  await expect(guidance).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(guidance).toBeHidden();
  await expect(merge).toBeVisible();
  await expect(draft).toHaveValue("Current edited version for main.");

  await merge.getByTestId("branch-guided-generate").click();
  await guidance.locator("textarea").fill("Emphasize the evidence and shorten the conclusion.");
  await guidance.getByTestId("branch-guidance-action").click();
  await expect.poll(() => lastInvokeArgs(page, "summarize_session_branch_merge")).toMatchObject({
    id: "conversation-branch",
    expectedGuardHash: "mock-branch-merge-guard",
    currentVersion: "Current edited version for main.",
    userGuidance: "Emphasize the evidence and shorten the conclusion.",
  });
  await expect(guidance).toBeHidden();
  await expect(draft).toHaveValue(
    "Guided version: Emphasize the evidence and shorten the conclusion.",
  );
});

test("the latest completed native reply starts an isolated exploration", async ({ page }) => {
  await enterExplorationProject(page);
  const start = page.getByTestId("start-exploration");
  await expect(start).toBeEnabled();
  await start.click();
  const overlay = page.getByTestId("exploration-start-overlay");
  await expect(overlay).toBeVisible();
  await overlay.locator("input").fill("Third candidate");
  await overlay.getByRole("button", { name: "Create exploration" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__startExplorationCalls?.at(-1))).toMatchObject({
    sourceFrameId: "exploration-mainline",
    turnIndex: 0,
    name: "Third candidate",
  });
  await expect(page.getByText("New exploration result")).toBeVisible();
  await expect(page.locator('[data-session-id="exploration-frame-created-3"]')).toHaveCount(0);
  await expect(page.getByText("Untitled session", { exact: true })).toHaveCount(0);
  await expect(page.getByTestId("sidebar-explorations").locator(".side-exploration")).toHaveCount(3);
});

test("every historical reply exposes exploration and the selected turn anchors the round", async ({ page }) => {
  await page.goto("/?mockExplorations=1&mockHistoricalExploration=1&mockNoExplorationRound=1");
  await page.locator(".proj-card-main").first().click();
  await page.locator('[data-session-id="exploration-mainline"]').click();
  const starts = page.getByTestId("start-exploration");
  await expect(starts).toHaveCount(3);
  for (const start of await starts.all()) await expect(start).toBeEnabled();

  // Open directly from a historical response, then Escape without moving focus.
  await starts.nth(1).click();
  const overlay = page.getByTestId("exploration-start-overlay");
  await expect(overlay).toContainText("through the selected response");
  await expect(overlay).toContainText("earlier file versions are not restored");
  await page.keyboard.press("Escape");
  await expect(overlay).toBeHidden();
  await expect(page.getByText("Legacy result", { exact: true })).toBeVisible();

  await starts.nth(1).click();
  await overlay.locator("input").fill("Revisit legacy parameters");
  await overlay.getByRole("button", { name: "Create exploration" }).click();
  await expect.poll(() => lastInvokeArgs(page, "start_exploration")).toMatchObject({
    sourceFrameId: "exploration-mainline", turnIndex: 1, name: "Revisit legacy parameters",
  });
  await expect(page.getByText("New exploration result")).toBeVisible();
  await page.locator('[data-session-id="exploration-mainline"]').click();
  await expect(starts.nth(0)).toBeDisabled();
  await expect(starts.nth(1)).toBeEnabled();
  await expect(starts.nth(2)).toBeDisabled();
  const checkpoint = page.getByText("Legacy result", { exact: true }).locator("xpath=ancestor::div[@data-ui-index]");
  await expect(checkpoint.getByTestId("exploration-message-card")).toContainText("Revisit legacy parameters");

  await page.getByTestId("mainline-exploration-banner").getByRole("button", { name: "Start another" }).click();
  await overlay.getByRole("button", { name: "Create exploration" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__startExplorationCalls?.length)).toBe(2);
  await expect.poll(() => lastInvokeArgs(page, "start_exploration")).toMatchObject({ turnIndex: 1 });
});

test("mainline stays frozen and can abandon the complete exploration round", async ({ page }) => {
  await enterExplorationProject(page);
  const mainline = page.locator('[data-session-id="exploration-mainline"]');
  await expect(page.locator("#composer-input")).toBeDisabled();
  await expect(page.locator("#composer-input")).toHaveAttribute(
    "placeholder",
    "Mainline is frozen while this exploration round is unresolved",
  );
  await mainline.click({ button: "right" });
  await expect(page.getByRole("button", { name: "Delete", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Abandon exploration", exact: true }).click();
  await expect(page.locator(".confirm-modal")).toContainText("permanently remove every exploration");
  await page.locator(".confirm-modal").getByRole("button", { name: "Abandon exploration" }).click();
  await expect(page.getByTestId("sidebar-explorations")).toHaveCount(0);
  await expect(page.locator("#composer-input")).toBeEnabled();
  await expect.poll(() => lastInvokeArgs(page, "abandon_exploration_round")).toMatchObject({
    sourceFrameId: "exploration-mainline",
  });
});

test("historical exploration sends the absolute turn index from a paged transcript", async ({ page }) => {
  await page.goto("/?mockExplorations=1&mockHistoricalExploration=1&mockNoExplorationRound=1&mockExplorationUserOffset=20");
  await page.locator(".proj-card-main").first().click();
  await page.locator('[data-session-id="exploration-mainline"]').click();
  await page.getByTestId("start-exploration").first().click();
  await page.getByTestId("exploration-start-overlay").getByRole("button", { name: "Create exploration" }).click();
  await expect.poll(() => lastInvokeArgs(page, "start_exploration")).toMatchObject({
    sourceFrameId: "exploration-mainline", turnIndex: 20,
  });
});

test("exploration candidates remain writable while mainline is frozen", async ({ page }) => {
  await enterExplorationProject(page);
  await page
    .getByTestId("sidebar-explorations")
    .locator('[data-exploration-id="exploration-a"]')
    .click();
  await expect(page.getByTestId("exploration-banner")).toContainText("Exploration A");
  await expect(page.locator("#composer-input")).toBeEnabled();
  await expect(page.getByTestId("start-exploration")).toHaveCount(0);
  await expect(page.locator(".msg-branch-btn")).toHaveCount(0);
  await expect(page.locator('.user-bubble [title="Branch"]')).toHaveCount(0);
  const userMessage = page.locator(".user-bubble").first();
  await userMessage.click({ button: "right" });
  await expect(page.getByRole("button", { name: "Branch to new conversation", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  // The removed composer send dropdown leaves no "Branch in new session"
  // entry anywhere inside an exploration.
  await expect(page.locator(".send-menu-toggle")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Branch in new session", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Branch to new conversation", exact: true })).toHaveCount(0);
});

test("user messages offer the mature branch flow from the context menu", async ({ page }) => {
  await page.goto("/?mockExplorations=1&mockHistoricalExploration=1");
  await page.locator(".proj-card-main").first().click();
  await page.locator('[data-session-id="exploration-mainline"]').click();

  const userMessage = page.locator(".user-bubble[data-branch-ui-index]").first();
  await userMessage.click({ button: "right" });
  const branch = page.getByRole("button", { name: "Branch to new conversation", exact: true });
  await expect(branch).toBeVisible();
  await expect(page.getByRole("button", { name: "Start exploration", exact: true })).toHaveCount(3);
  await branch.click();
  await expect(page.locator("#composer-input")).toHaveValue("");
  await expect(page.getByText("Legacy method", { exact: true })).toHaveCount(0);
});

test("promotion merges one exploration into the original mainline and discards the round", async ({ page }) => {
  await enterExplorationProject(page);
  const group = page.getByTestId("sidebar-explorations");

  await group.locator('[data-exploration-id="exploration-a"]').click();
  await page.getByTestId("exploration-banner").getByRole("button", { name: "View diff" }).click();
  await page.getByTestId("exploration-diff-overlay").getByRole("button", { name: "Set as mainline" }).click();
  await expect(page.getByTestId("exploration-confirm-overlay")).toBeVisible();
  await page.getByTestId("exploration-confirm-action").click();

  await expect(page.getByText("Mainline result")).toBeVisible();
  await expect(page.locator('[data-session-id="exploration-mainline"]')).toHaveCount(1);
  await expect(page.locator('[data-session-id="exploration-frame-a"]')).toHaveCount(0);
  await expect(page.getByTestId("mainline-exploration-banner")).toBeHidden();
  await expect(page.locator("#composer-input")).toBeEnabled();
  await expect(page.locator('[data-exploration-id="exploration-b"]')).toHaveCount(0);
  await expect(page.getByTestId("sidebar-explorations")).toHaveCount(0);
});
