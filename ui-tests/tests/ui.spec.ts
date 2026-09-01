import { test, expect, type Locator, type Page } from "@playwright/test";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { tauriMock, parallelMock, parallelReplyTailText } from "./mock-tauri";

const officeFixtures = {
  xlsxBase64: readFileSync(resolve(__dirname, "../fixtures/office-preview.xlsx")).toString("base64"),
  pptxBase64: readFileSync(resolve(__dirname, "../fixtures/office-preview.pptx")).toString("base64"),
};
const motifAppHtmlPath = process.env.WISP_MOTIF_APP_HTML;
const snapGeneFixturePath = process.env.WISP_SNAPGENE_FIXTURE;

function providerSelect(page: Page) {
  return page.getByTestId("settings-provider");
}

function globalSettingsButton(page: Page) {
  // "Project settings" on landing cards also matches name: "Settings" unless exact.
  return page.getByRole("button", { name: "Settings", exact: true });
}

async function expectInsideViewport(locator: Locator, width: number, height: number) {
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.y).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(width);
  expect(box!.y + box!.height).toBeLessThanOrEqual(height);
}

async function openModelsSettings(page: Page) {
  await globalSettingsButton(page).click();
  await page.getByRole("button", { name: "Models" }).click();
  const row = page.locator(".settings-list-row").first();
  if (await row.count()) {
    await row.click();
  } else {
    await page.getByRole("button", { name: /Add API access/i }).click();
  }
  await expect(providerSelect(page)).toBeVisible();
}

async function openSettingsSection(page: Page, name: string) {
  await globalSettingsButton(page).click();
  if (name === "Session") {
    await page.getByTestId("settings-nav-session").click();
    return;
  }
  await page.getByRole("button", { name, exact: true }).click();
}

// The app now boots to the Projects landing screen; open a real project (not
// the "Example project" card) to reach the chat UI the tests assert against.
async function enterApp(page: Page, path = "/") {
  await page.goto(path);
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();
  if (path.includes("mockAgentWorkflow=")) {
    const session = page.locator('[data-session-id="s-current"]');
    await expect(session).toBeVisible();
    await session.click();
  }
}

async function openMockPlanSession(page: Page, kind: "acp" | "compat" | "native") {
  await enterApp(page, `/?mockPlanFlow=${kind}`);
  const session = page.locator('[data-session-id="s1"]');
  await expect(session).toBeVisible();
  await session.click();
  await expect(page.getByTestId("plan-card")).toBeVisible();
}

async function emitTauriEvent(page: Page, event: string, payload: unknown) {
  await expect.poll(() => page.evaluate((name) =>
    Boolean((window as any).__tauriListenerReady?.(name)), event
  )).toBe(true);
  await page.evaluate(({ name, value }) => {
    (window as any).__tauriEmit(name, value);
  }, { name: event, value: payload });
}

async function activateAcpPlanMode(page: Page) {
  await emitTauriEvent(page, "acp-session-state", {
    frameId: "s1",
    modes: {
      currentModeId: "plan",
      availableModes: [
        { id: "default", name: "Default" },
        { id: "plan", name: "Plan" },
      ],
    },
  });
}

function composer(page: Page) {
  return page.locator("#composer-input");
}

// Transcript-scoped locators for parallelMock conversations. The mock replies
// to each user turn with a single assistant bubble quoting the message text,
// so specs assert on the user's own text landing in the right session's rows
// instead of depending on the mock's internal reply format.
function userTurn(page: Page, text: string) {
  return page.locator(".msg.user .body", { hasText: text });
}

function assistantReplyQuoting(page: Page, text: string) {
  return page.locator(".msg.assistant .body", { hasText: text });
}

// Shortcut labels come from the UI's platform detection (`is_mac` in
// ui/src/api.js reads navigator.userAgent/platform), so any test asserting
// literal "Ctrl+…" text must pin a non-mac platform first or it renders
// "⌘…" on macOS hosts. Call before the first page.goto().
async function pinNonMacPlatform(page: Page) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "userAgent", {
      configurable: true,
      value: "wisp-science/Tauri",
    });
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: "Linux x86_64",
    });
  });
}

async function selectAssistantReplyText(
  page: Page,
  eventType: "mouseup" | "contextmenu" = "mouseup",
) {
  return page.evaluate((type) => {
    const body = document.querySelector(".msg.assistant .body");
    if (!body) return "";
    const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT);
    let node: Text | null = null;
    while (walker.nextNode()) {
      const candidate = walker.currentNode as Text;
      if (candidate.data.trim().length > 20) {
        node = candidate;
        break;
      }
    }
    if (!node) return "";
    const range = document.createRange();
    range.selectNodeContents(node);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    const rect = range.getBoundingClientRect();
    const target = document.querySelector(".chat") ?? body;
    target.dispatchEvent(new MouseEvent(type, {
      bubbles: true,
      cancelable: true,
      button: type === "contextmenu" ? 2 : 0,
      clientX: rect.left + rect.width / 2,
      clientY: rect.top + Math.min(rect.height / 2, 12),
    }));
    return node.data.trim();
  }, eventType);
}

function newSessionButton(page: Page) {
  return page.locator(".sidebar").getByRole("button", { name: "New session" });
}

async function openAgentMenu(page: Page) {
  await page.getByRole("button", { name: "Agent options" }).click();
  return page.getByRole("menu", { name: "Agent options" });
}

async function enableDelegation(page: Page) {
  const menu = await openAgentMenu(page);
  const row = menu.locator("label.agent-menu-row", { hasText: "Delegation" });
  const toggle = row.locator('input[type="checkbox"]');
  if (!(await toggle.isChecked())) await row.click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_delegation_enabled"))
    .toMatchObject({ enabled: true });
  await page.keyboard.press("Escape");
}

async function openComputeMenu(page: Page) {
  const agentMenu = await openAgentMenu(page);
  await agentMenu.getByRole("button", { name: /^Compute/ }).click();
  return page.getByRole("menu", { name: "Compute" });
}

async function selectRemoteContext(page: Page) {
  const menu = await openComputeMenu(page);
  const server = menu.locator('[data-context-id="ssh:gpu-server"]');
  if (!(await server.getAttribute("class"))?.includes("enabled")) {
    await server.click();
    await expect(server).toHaveClass(/enabled/);
  }
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
}

function commandPalette(page: Page) {
  return page.locator("#command-palette-input");
}

async function lastInvokeArgs(page: Page, cmd: string) {
  return page.evaluate((name) => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      return value;
    };
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === name);
    return plain(calls.at(-1)?.args ?? null);
  }, cmd);
}

async function invokeCount(page: Page, cmd: string) {
  return page.evaluate((name) =>
    ((window as any).__skillInvokeLog ?? []).filter((call: any) => call.cmd === name).length,
  cmd);
}

// How many run-list refreshes the UI has performed. The app polls `list_runs`
// on a ticker (every second while a turn is busy or a transfer is active),
// so "wait until N more polls happened" replaces fixed sleeps in tests that
// assert the UI survives a poll-driven rebuild.
async function runListPollCount(page: Page) {
  return page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "list_runs").length);
}

async function invokeArgsList(page: Page, cmd: string) {
  return page.evaluate((name) => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      return value;
    };
    return ((window as any).__skillInvokeLog ?? [])
      .filter((call: any) => call.cmd === name)
      .map((call: any) => plain(call.args));
  }, cmd);
}

async function setMockUpdateCheck(page: Page, value: Record<string, unknown>) {
  await page.evaluate((payload) => {
    (window as any).__setMockUpdateCheck(payload);
  }, value);
}

async function setMockUpdateCheckPending(page: Page, pending: boolean) {
  await page.evaluate((value) => {
    (window as any).__setMockUpdateCheckPending(value);
  }, pending);
}

async function resolveMockUpdateCheck(page: Page) {
  await page.evaluate(() => {
    (window as any).__resolveMockUpdateCheck();
  });
}

async function setMockUpdateCheckError(page: Page, error: string) {
  await page.evaluate((message) => {
    (window as any).__setMockUpdateCheckError(message);
  }, error);
}

async function setMockUpdateDownload(
  page: Page,
  value: { pending?: boolean; error?: string | null },
) {
  await page.evaluate((options) => {
    (window as any).__setMockUpdateDownload(options);
  }, value);
}

async function resolveMockUpdateDownload(page: Page) {
  await page.evaluate(() => {
    (window as any).__resolveMockUpdateDownload();
  });
}

async function setMockInstallUpdateError(page: Page, error: string) {
  await page.evaluate((message) => {
    (window as any).__setMockInstallUpdateError(message);
  }, error);
}

test.beforeEach(async ({ page }) => {
  // Install the Tauri bridge mock before the page's wasm runs.
  await page.addInitScript(tauriMock, officeFixtures);
});

test("Example project shows bundled demos as read-only transcripts", async ({ page }) => {
  await page.goto("/");
  const sessionsBefore = (await invokeArgsList(page, "new_session")).length;
  const sendsBefore = (await invokeArgsList(page, "send_message")).length;
  const scratchBefore = (await invokeArgsList(page, "start_scratch_chat")).length;
  // The synthetic "Example project" opens a demo view whose sidebar lists the
  // bundled demos (no per-project "Open demo" button any more).
  await page.getByText("Example project").click();
  await expect(page.getByTestId("demo-read-only")).toBeVisible();
  await expect(newSessionButton(page)).toHaveCount(0);
  await expect(composer(page)).not.toBeVisible();

  // Keyboard paths are guarded too; the read-only demo cannot be turned into
  // either a regular or scratch conversation.
  await page.keyboard.press("Control+n");
  await page.keyboard.press("Control+Shift+n");
  expect((await invokeArgsList(page, "new_session")).length).toBe(sessionsBefore);
  expect((await invokeArgsList(page, "start_scratch_chat")).length).toBe(scratchBefore);

  await expect(page.getByText("Help me find RNA-seq knockdown datasets")).toBeVisible();
  await expect(page.getByText("What specific samples are included in GSE153250")).toBeVisible();
  await expect(page.getByText("Based on the upstream Counts data from GSE153250")).toBeVisible();
  await expect(page.getByText("Based on the Counts data from our study")).toBeVisible();
  await page.getByText("Connect to the remote compute host, locate the FASTQ data").click();

  // The demo request renders as the user turn…
  await expect(
    page.getByText("Connect to the remote compute host, locate the FASTQ data for GSE153250, keep only the siESR1 and siNT groups."),
  ).toBeVisible();
  // …and the agent's final report renders as the assistant turn.
  await expect(page.getByText("GSE153250 RNA-seq Upstream Analysis")).toBeVisible();
  // Full transcript includes SSH/run operation cards, not just the summary.
  await expect(page.getByText("Re-run pipeline with fixed STAR index")).toBeVisible();
  await expect(page.getByTestId("run-monitor-card")).toBeVisible();
  expect((await invokeArgsList(page, "new_session")).length).toBe(sessionsBefore);
  expect((await invokeArgsList(page, "send_message")).length).toBe(sendsBefore);
});

test("Example project demos can be copied into a workspace", async ({ page }) => {
  await page.goto("/");
  await page.getByText("Example project").click();
  await expect(page.getByTestId("demo-read-only")).toBeVisible();

  const demo = page.locator(".side-item.ses", { hasText: "Long-context memory demo" });
  await expect(demo).toBeVisible();
  await demo.click({ button: "right" });
  const menu = page.locator(".ctx-menu");
  await expect(menu.getByRole("button", { name: "Copy to a project…", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(page.getByTestId("demo-read-only")).toBeVisible();

  await demo.click({ button: "right" });
  await menu.getByRole("button", { name: "Copy to a project…", exact: true }).click();
  const transfer = page.locator(".session-transfer-modal");
  await expect(transfer).toBeVisible();
  await expect(transfer.getByRole("heading", { name: "Copy demo to a project" })).toBeVisible();
  await expect(transfer.locator("select")).toHaveValue("default");
  await page.keyboard.press("Escape");
  await expect(transfer).toHaveCount(0);
  await expect(page.getByTestId("demo-read-only")).toBeVisible();

  await demo.click({ button: "right" });
  await page.getByRole("button", { name: "Copy to a project…", exact: true }).click();
  await page.locator(".session-transfer-modal").getByRole("button", { name: "Copy", exact: true }).click();
  await expect.poll(() => lastInvokeArgs(page, "copy_demo_to_project")).toMatchObject({
    id: "manifest_memory_01_long_context",
    targetProjectId: "default",
  });
});

test("send streams a mocked assistant reply", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await enterApp(page);
  await composer(page).fill("hello there");
  await page.getByRole("button", { name: "Send" }).click();
  // Deltas "Hello " + "from mock wisp-science." accumulate into one assistant bubble.
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  const followUps = page.getByTestId("follow-up-questions");
  await expect(followUps.getByRole("button")).toHaveCount(4);
  await followUps.getByRole("button", { name: "Expand the search for underrepresented species" }).click();
  await expect(composer(page)).toHaveValue("Expand the search for underrepresented species");
  await followUps.getByRole("button", { name: "Hide follow-up questions" }).click();
  await expect(followUps).toHaveCount(0);
  await page.locator(".msg.assistant").getByRole("button", { name: "Review" }).click();
  await expect.poll(() => lastInvokeArgs(page, "review_session")).toMatchObject({
    sessionId: expect.stringMatching(/^s-/),
  });
  await page.locator(".msg.assistant").getByRole("button", { name: "Copy" }).click();
  await expect(page.locator(".copy-toast")).toHaveText("Copied");
});

test("sending a follow-up hides suggestions before the User event arrives", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("hello there");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  const followUps = page.getByTestId("follow-up-questions");
  await expect(followUps.getByRole("button")).toHaveCount(4);

  await page.evaluate(() => {
    (window as any).__userEventDelayMs = 800;
  });
  await composer(page).fill("how to read this figure");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("how to read this figure")).toBeVisible();
  await expect(followUps).toHaveCount(0, { timeout: 300 });
});

test("completed turns propose editable memory and require confirmation", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("summarize this project convention");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await expect(page.locator(".rightpane")).toBeVisible();

  await page.locator(".msg.assistant").getByRole("button", { name: "Memory" }).click();
  const modal = page.getByTestId("turn-memory-overlay");
  await expect(modal).toBeVisible();
  await expect(page.getByTestId("turn-memory-content")).toHaveValue(
    "Prefer reproducible local workflows for this project.",
  );
  await expect.poll(() => lastInvokeArgs(page, "propose_turn_memory")).toMatchObject({
    sessionId: expect.stringMatching(/^s-/),
    turnIndex: 0,
    automatic: false,
  });

  // Root-owned modal participates in the window Escape stack without focus.
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  await expect(page.locator(".rightpane")).toBeVisible();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();

  await page.locator(".msg.assistant").getByRole("button", { name: "Memory" }).click();
  await page.getByTestId("turn-memory-content").fill("Always prefer reproducible local workflows.");
  await page.getByTestId("turn-memory-scope").selectOption("global");
  await expect(page.getByTestId("turn-memory-replace")).toHaveValue("");
  await page.getByTestId("turn-memory-replace").selectOption("global-memory-existing");
  await page.getByTestId("turn-memory-confirm").click();
  await expect(modal).toHaveCount(0);
  await expect(page.locator(".copy-toast")).toHaveText(
    "Global memory saved. It will be used from the next turn.",
  );
  await expect.poll(() => lastInvokeArgs(page, "confirm_turn_memory")).toMatchObject({
    scope: "global",
    content: "Always prefer reproducible local workflows.",
    replaceId: "global-memory-existing",
    turnIndex: 0,
  });
});

test("explicit remember requests open a global confirmation even when failure analysis is off", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("REMEMBER always use SI units");
  await page.getByRole("button", { name: "Send" }).click();

  const modal = page.getByTestId("turn-memory-overlay");
  await expect(modal).toBeVisible();
  await expect(page.getByTestId("turn-memory-scope")).toHaveValue("global");
  await expect(modal).toContainText("asked Wisp to remember");
});

test("optional tool-failure analysis exposes thresholds and proposes a confirmed lesson", async ({ page }) => {
  await enterApp(page);
  let menu = await openAgentMenu(page);
  await menu.locator("label.agent-menu-row", { hasText: "Analyze tool failures" }).click();
  await expect(page.getByTestId("failure-rate-threshold")).toHaveValue("30");
  await expect(page.getByTestId("minimum-failures")).toHaveValue("2");
  await page.getByTestId("failure-rate-threshold").fill("60");
  await page.getByTestId("failure-rate-threshold").press("Tab");
  await expect.poll(() => lastInvokeArgs(page, "set_auto_failure_analysis_settings"))
    .toMatchObject({ settings: { enabled: true, failure_rate_threshold: 60, minimum_failures: 2 } });

  await page.keyboard.press("Escape");
  await composer(page).fill("TOOLFAILMEMORY diagnose retries");
  await page.getByRole("button", { name: "Send" }).click();

  const modal = page.getByTestId("turn-memory-overlay");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("2 of 3 tool calls failed (66.7%)");
  await expect(page.getByTestId("turn-memory-scope")).toHaveValue("project");
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
});

test("tool-only turn endings do not generate follow-up questions", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("TOOLONLYDONE finish the report");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByRole("button", { name: "Processed" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
  await page.waitForTimeout(100);
  await expect(page.getByTestId("follow-up-questions")).toHaveCount(0);
  expect(await page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? [])
      .filter((call: any) => call.cmd === "generate_follow_up_questions").length,
  )).toBe(0);
});

test("manual review blocks sending and shows a playful progress animation", async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__reviewDelayMs = 800;
  });
  await enterApp(page);
  await composer(page).fill("review this answer");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();

  await page.locator(".msg.assistant").getByRole("button", { name: "Review" }).click();
  await expect(page.getByRole("button", { name: "Send" })).toBeDisabled();
  const progress = page.getByTestId("review-live");
  await expect(progress).toBeVisible();
  await expect(progress).toContainText("Reviewer is checking the margins");
  await expect(progress.locator(".review-live-lens")).toBeVisible();

  await expect(page.getByRole("button", { name: "Send" })).toBeEnabled();
  await expect(progress).toHaveCount(0);
});

test("undo returns the latest prompt and keeps unsupported Word files", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("revise my notes");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  const undo = page.locator(".msg.assistant").getByRole("button", { name: "Undo" });
  await expect(undo).toHaveCount(1);
  await undo.click();
  await expect.poll(() => lastInvokeArgs(page, "preview_turn_undo")).toMatchObject({
    sessionId: expect.stringMatching(/^s-/),
    userIndex: 0,
  });

  const modal = page.getByTestId("turn-undo-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("Word, Excel, PDF, images");
  await expect(modal).toContainText("notes.md");
  await expect(modal).toContainText("summary.md");
  await expect(modal).toContainText("paper.docx");
  await modal.getByRole("button", { name: "Undo turn" }).click();

  await expect.poll(() => lastInvokeArgs(page, "undo_turn")).toMatchObject({
    sessionId: expect.stringMatching(/^s-/),
    userIndex: 0,
  });
  await expect(modal).toHaveCount(0);
  await expect(page.locator(".msg.user", { hasText: "revise my notes" })).toHaveCount(0);
  await expect(page.getByText("Hello from mock wisp-science.")).toHaveCount(0);
  await expect(composer(page)).toHaveValue("revise my notes");
});

test("general settings can use Ctrl+Enter to send and Enter for newline", async ({ page }) => {
  await pinNonMacPlatform(page);
  await enterApp(page);
  await openSettingsSection(page, "General");
  const shortcut = page.getByTestId("send-shortcut");
  await expect(shortcut).toHaveValue("enter");
  await shortcut.selectOption("modifier_enter");
  await page.locator(".settings-footer").getByRole("button", { name: "Save" }).click();

  await page.reload();
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator(".composer-hint")).toContainText("Ctrl+Enter to send · Enter for newline");

  const input = composer(page);
  await input.fill("first line");
  await input.press("Enter");
  await input.pressSequentially("second line");
  await expect(input).toHaveValue("first line\nsecond line");
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toBeNull();

  await input.press("Control+Enter");
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    message: "first line\nsecond line",
  });
});

test("Usage groups workspaces, charts activity and models, and paginates sessions", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "Usage");

  const usage = page.getByTestId("usage-pane");
  await expect(usage).toBeVisible();
  await expect(usage.locator(".usage-tile")).toHaveCount(4);
  const activity = page.getByTestId("usage-activity");
  await expect(activity.locator(".usage-activity-cell")).toHaveCount(371);

  const daily = activity.getByRole("button", { name: "Daily", exact: true });
  const weekly = activity.getByRole("button", { name: "Weekly", exact: true });
  const cumulative = activity.getByRole("button", { name: "Cumulative", exact: true });
  await expect(daily).toHaveAttribute("aria-pressed", "true");
  await weekly.click();
  await expect(weekly).toHaveAttribute("aria-pressed", "true");
  await cumulative.click();
  await expect(cumulative).toHaveAttribute("aria-pressed", "true");

  const modelShare = page.getByTestId("usage-model-share");
  await expect(modelShare.getByText("deepseek-v4-pro", { exact: true })).toBeVisible();
  await expect(modelShare.getByText("opus-4.8", { exact: true })).toBeVisible();
  await expect(modelShare.locator(".usage-model-pie")).toHaveCSS(
    "background-image",
    /conic-gradient/,
  );

  const toolRank = page.getByTestId("usage-tool-rank");
  await expect(toolRank.getByText("bear-support", { exact: true })).toBeVisible();
  await expect(toolRank.getByText("pubmed_search", { exact: true })).toBeVisible();
  await expect(toolRank.getByTestId("usage-tool-rank-row")).toHaveCount(3);

  const workspaces = page.getByTestId("usage-workspace-row");
  await expect(workspaces).toHaveCount(2);
  await workspaces.first().click();
  await expect(page.getByTestId("usage-session-row")).toHaveCount(20);
  await expect(page.getByTestId("usage-pagination")).toContainText("Page 1 of 2");

  await page.getByTestId("usage-pagination").getByRole("button", { name: "Next" }).click();
  await expect(page.getByTestId("usage-session-row")).toHaveCount(3);
  await expect(page.getByText("Workspace session 21", { exact: true })).toBeVisible();
  await expect(page.getByTestId("usage-pagination")).toContainText("Page 2 of 2");

  await page.getByTestId("usage-back").click();
  await expect(page.getByTestId("usage-workspace-row")).toHaveCount(2);
});

test("language select shows the saved locale so Chinese can switch to English directly (#431)", async ({ page }) => {
  await page.goto("/?mockLocale=zh");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator(".sidebar").getByRole("button", { name: "新建会话" })).toBeVisible();

  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.getByRole("button", { name: "常规", exact: true }).click();
  const language = page.getByTestId("settings-language");
  // Regression: the select's value binding was applied before its options
  // existed, so it fell back to the first option ("en") while the saved locale
  // was Chinese — picking English then fired no change event and never saved.
  await expect(language).toHaveValue("zh");

  await language.selectOption("en");
  // Selecting a language re-renders the UI in that language immediately.
  await page.locator(".settings-footer").getByRole("button", { name: "Save" }).click();
  await expect.poll(() =>
    page.evaluate(() => (window as any).__lastSetSettings?.locale)
  ).toBe("en");

  // The app stays in English and the select stays on English after reopening.
  await globalSettingsButton(page).click();
  await expect(page.getByTestId("settings-language")).toHaveValue("en");
});

test("storage separates project paths and filters usage when a project is clicked", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Storage");

  const projects = page.getByTestId("storage-project-list");
  const paths = projects.locator(".storage-project-path");
  await expect(paths).toHaveCount(2);
  await expect(paths.nth(0)).toHaveText("/mock/root");
  await expect(paths.nth(1)).toHaveText("/mock/other");

  const other = projects.locator('[data-project-id="other"]');
  await other.click();
  await expect(other).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".storage-path")).toHaveText("/mock/other");
  await expect(page.locator(".storage-legend-row").filter({ hasText: "Workspace files" }))
    .toContainText("24.0 MB");
  await expect(page.locator(".storage-legend-row").filter({ hasText: "Python environment" }))
    .toHaveCount(0);

  await projects.locator(".storage-project-row").first().click();
  await expect(page.locator(".storage-path")).toHaveText("C:\\mock\\AppData\\wisp-science");
  await expect(page.locator(".storage-legend-row").filter({ hasText: "Workspace files" }))
    .toContainText("120.0 MB");
});

test("sidebar Feedback opens a blank conversation and waits for the user's first turn (#596)", async ({ page }) => {
  await enterApp(page);
  await page.setInputFiles("#composer-file-input", {
    name: "counts.csv",
    mimeType: "text/csv",
    buffer: Buffer.from("a,b\n1,2"),
  });
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  const tile = page.locator('.rp-tile[data-artifact-name="counts.csv"]');
  await tile.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-tab.active")).toContainText("counts.csv");
  const sendsBefore = (await invokeArgsList(page, "send_message")).length;
  const sessionsBefore = (await invokeArgsList(page, "new_session")).length;

  await page.getByTestId("report-problem-entry").click();
  await expect(page.getByTestId("feedback-context")).toContainText("System information");
  await expect(page.getByTestId("feedback-context")).toContainText("Attached automatically");
  await expect(page.locator("#composer-input")).toBeFocused();
  await expect(page.locator(".center-file-preview")).toHaveCount(0);
  expect((await invokeArgsList(page, "send_message")).length).toBe(sendsBefore);
  expect((await invokeArgsList(page, "new_session")).length).toBe(sessionsBefore);

  await page.locator("#composer-input").fill("The app freezes when I open a document");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => invokeArgsList(page, "new_session")).toHaveLength(sessionsBefore + 1);
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    message: expect.stringContaining("The app freezes when I open a document"),
  });
  const sent = await lastInvokeArgs(page, "send_message");
  expect(sent?.message).toContain("Feedback context:");
  expect(sent?.message).toContain("GitHub issue");
  expect(sent?.message).toMatch(/Wisp version: 0\.29\.0/);
  expect(sent?.message).toMatch(/OS \/ architecture: windows \/ x86_64/);
  expect(sent?.message).toMatch(/Model profile: deepseek-v4-pro/);
  expect(sent?.message).not.toMatch(/\/mock\/root/);
  await expect(page.getByTestId("feedback-context")).toHaveCount(0);
  const userBubble = page.locator(".msg.user").last();
  await expect(userBubble).toContainText("The app freezes when I open a document");
  await expect(userBubble).not.toContainText("Feedback context");
  await expect(userBubble).not.toContainText("GitHub issue");
});

test("Feedback send shows the new_session error instead of a {msg} placeholder", async ({ page }) => {
  await enterApp(page);
  await page.getByTestId("report-problem-entry").click();
  await expect(page.getByTestId("feedback-context")).toContainText("System information");
  await page.evaluate(() => {
    (window as any).__failNextNewSession("Project not found");
  });
  await page.locator("#composer-input").fill("The send button does nothing");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.locator(".topbar .hint")).toHaveText("Send failed: Project not found");
  await expect(page.locator(".topbar .hint")).not.toContainText("{msg}");
  await expect(page.getByTestId("feedback-context")).toBeVisible();
  await expect(page.locator("#composer-input")).toHaveValue("The send button does nothing");
});

test("Memory settings show the active project name", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Memory");
  const project = page.getByTestId("memory-project");
  await expect(page.getByTestId("memory-project-select")).toHaveAttribute("data-project-id", "default");
  await expect(page.getByTestId("memory-project-select")).toContainText("wisp-science");
  await expect(project).toContainText("(1)");
  await expect(page.locator(".conn-group-label", { hasText: "Project memory" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Clear all" })).toHaveClass("memory-clear-btn");
  await expect(page.locator(".memory-toggle-label")).toHaveText("Memory");
  await expect(
    page.getByTestId("memory-notes").locator(".memory-note-icon svg").first(),
  ).toBeVisible();
});

test("Memory settings show and forget global habits", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Memory");

  const global = page.getByTestId("global-memories");
  await expect(global).toContainText("Prefer SI units across projects.");
  await expect(page.getByText(/Snapshotted when a turn starts/)).toBeVisible();
  await global.getByRole("button", { name: "Edit global habit" }).click();
  const editor = page.getByTestId("global-memory-editor");
  await expect(editor.getByRole("textbox", { name: "Memory" })).toHaveValue(
    "Prefer SI units across projects.",
  );
  await editor.getByRole("textbox", { name: "Memory" }).fill("Prefer metric units across projects.");
  await editor.getByRole("button", { name: "Save global habit" }).click();
  await expect.poll(() => lastInvokeArgs(page, "update_global_memory")).toMatchObject({
    id: "global-memory-existing",
    content: "Prefer metric units across projects.",
  });
  await expect(global).toContainText("Prefer metric units across projects.");
  await expect(page.getByText(/new value will be used from the next turn/)).toBeVisible();
  await global.getByRole("button", { name: "Forget global habit" }).click();
  await expect.poll(() => lastInvokeArgs(page, "delete_global_memory")).toMatchObject({
    id: "global-memory-existing",
  });
  await expect(global).toContainText("No global habits yet.");
  await expect(global.locator(".memory-empty")).toBeVisible();
  await expect(page.getByText(/older chat history may still affect this session/)).toBeVisible();
});

test("Memory settings can add a global habit", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Memory");

  const global = page.getByTestId("global-memories");
  await expect(global).toContainText("Prefer SI units across projects.");
  await page.getByTestId("global-memory-add").click();
  const editor = page.getByTestId("global-memory-editor");
  await expect(editor.getByRole("textbox", { name: "Memory" })).toHaveValue("");
  await editor.getByRole("textbox", { name: "Memory" }).fill("Always plot with ggplot2.");
  await editor.getByRole("button", { name: "Save global habit" }).click();
  await expect.poll(() => lastInvokeArgs(page, "create_global_memory")).toMatchObject({
    content: "Always plot with ggplot2.",
  });
  await expect(global).toContainText("Always plot with ggplot2.");
  await expect(global.locator(".global-memory-content").first()).toHaveText(
    "Always plot with ggplot2.",
  );
  await expect(page.getByText(/Global habit added/)).toBeVisible();
});

test("Memory settings can browse another project's notes without switching workspace", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Memory");
  await expect(page.getByText("2026-07-01.md", { exact: true })).toBeVisible();
  const openProjectCalls = await invokeArgsList(page, "open_project");

  await page.getByTestId("memory-project-select").click();
  await expect(page.getByTestId("memory-project-menu")).toBeVisible();
  await page.getByTestId("memory-project-option-other").click();
  await expect(page.getByTestId("memory-project-menu")).toHaveCount(0);
  await expect(page.getByTestId("memory-project-select")).toHaveAttribute("data-project-id", "other");
  await expect(page.getByTestId("memory-project-select")).toContainText("Other project");
  await expect(page.getByText("other-2026-07-02.md", { exact: true })).toBeVisible();
  await expect(page.getByText("2026-07-01.md", { exact: true })).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "get_memory_view")).toMatchObject({
    projectId: "other",
  });
  // Browsing memory must not switch the active chat project.
  await expect.poll(() => invokeArgsList(page, "open_project")).toHaveLength(openProjectCalls.length);
});

test("Memory settings edit and delete notes in the browsed project", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Memory");
  await page.getByTestId("memory-project-select").click();
  await page.getByTestId("memory-project-option-other").click();
  await expect(page.getByText("other-2026-07-02.md", { exact: true })).toBeVisible();

  await page.getByText("other-2026-07-02.md", { exact: true }).click();
  const editor = page.locator(".memory-editor-text");
  await expect(editor).toHaveValue("Notes for other workspace.");
  await expect.poll(() => lastInvokeArgs(page, "read_memory_file")).toMatchObject({
    name: "other-2026-07-02.md",
    projectId: "other",
  });

  await editor.fill("Edited from the picker.");
  await page.getByRole("button", { name: "Save note" }).click();
  await expect.poll(() => lastInvokeArgs(page, "write_memory_file")).toMatchObject({
    name: "other-2026-07-02.md",
    content: "Edited from the picker.",
    projectId: "other",
  });

  await page.getByRole("button", { name: "Delete file" }).click();
  await expect.poll(() => lastInvokeArgs(page, "delete_memory_file")).toMatchObject({
    name: "other-2026-07-02.md",
    projectId: "other",
  });
  await expect(page.getByText("other-2026-07-02.md", { exact: true })).toHaveCount(0);
  await expect(page.getByText("No notes yet.")).toBeVisible();
});

test("Memory project picker consumes Escape before leaving Settings", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Memory");
  await page.getByTestId("memory-project-select").click();
  await expect(page.getByTestId("memory-project-menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("memory-project-menu")).toHaveCount(0);
  await expect(page.locator(".settings-page")).toBeVisible();
  await expect(page.locator(".settings-nav button.active")).toHaveText("Memory");
});

test("settings subpages consume Escape before leaving Settings", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Memory");
  await page.getByText("2026-07-01.md", { exact: true }).click();
  await expect(page.locator(".memory-editor-text")).toBeVisible();
  await expect(page.locator(".settings-breadcrumb")).toContainText("2026-07-01.md");

  await page.keyboard.press("Escape");
  await expect(page.locator(".memory-editor-text")).toHaveCount(0);
  await expect(page.locator(".settings-page")).toBeVisible();
  await expect(page.locator(".settings-nav button.active")).toHaveText("Memory");
  await expect(page.getByText("2026-07-01.md", { exact: true })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toHaveCount(0);
});

test("Workflow Studio Escape returns to Quick Actions before leaving Settings", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Workflows");
  await expect(page.getByTestId("workflow-studio")).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("workflow-studio")).toHaveCount(0);
  await expect(page.locator(".settings-page")).toBeVisible();
  await expect(page.locator(".settings-nav button.active")).toHaveText("Quick Actions");

  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toHaveCount(0);
});

test("background Agent completion appears in its owning conversation", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("start background analysis");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  const sent = await lastInvokeArgs(page, "send_message");
  await page.evaluate((frameId) => {
    (window as any).__tauriEmit("agent", {
      kind: "DelegationCompleted",
      frame_id: frameId,
      workflow_id: "workflow-background-1",
      status: "succeeded",
      result: JSON.stringify({
        type: "delegated_batch_completion",
        result: { status: "succeeded", results: [{ id: "analysis", summary: "finished" }] },
      }),
      auto_resume: false,
    });
  }, sent.sessionId);

  const activity = page.locator(".steps.activity-summary").last();
  await activity.getByRole("button", { name: /Processed/ }).click();
  const card = activity.locator(".step", { hasText: "delegate_tasks" }).last();
  await expect(card).toContainText("Background Agent batch completed");
  await expect(card).toContainText("· workflow");
});

test("switching HTTP models confirms cache invalidation", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("bind this conversation model");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();

  await page.locator(".model-picker-btn").click();
  const opusOption = page.getByRole("button", { name: /opus-4\.8/ });
  await expect(opusOption).toBeVisible();
  await opusOption.evaluate((element: HTMLElement) => element.click());
  const modal = page.getByTestId("model-switch-confirm");
  await expect(modal).toContainText("invalidates this conversation's model cache");
  await expect(modal).toContainText("opus-4.8");
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toBeNull();

  await modal.getByRole("button", { name: "No", exact: true }).click();
  await expect(modal).toHaveCount(0);
  await expect(page.locator(".model-picker-label")).toHaveText("deepseek-v4-pro");

  await page.locator(".model-picker-btn").click();
  await expect(opusOption).toBeVisible();
  await opusOption.evaluate((element: HTMLElement) => element.click());
  await page.getByTestId("model-switch-confirm")
    .getByRole("button", { name: "Yes, switch" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toMatchObject({ id: "opus" });
  await expect(page.locator(".model-picker-label")).toHaveText("opus-4.8");
});

test("model switch confirm consumes Escape before the right pane", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("bind this conversation model");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await expect(page.locator(".rightpane")).toBeVisible();

  await page.locator(".model-picker-btn").click();
  const opusOption = page.getByRole("button", { name: /opus-4\.8/ });
  await expect(opusOption).toBeVisible();
  await opusOption.evaluate((element: HTMLElement) => element.click());
  const modal = page.getByTestId("model-switch-confirm");
  await expect(modal).toBeVisible();

  // Root-owned confirm participates in the window Escape stack without focus:
  // one press cancels only the switch, leaving the right pane open and the
  // session model untouched.
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  await expect(page.locator(".rightpane")).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toBeNull();
  await expect(page.locator(".model-picker-label")).toHaveText("deepseek-v4-pro");

  // The next press reaches the parent layer.
  await page.keyboard.press("Escape");
  await expect(page.locator(".rightpane")).toBeHidden();
});

test("switching to a text-only model confirms historical images will be ignored", async ({ page }) => {
  await enterApp(page, "/?mockTextOnlyModel=1");

  // Start on the visual profile, then leave a real image attachment in this
  // conversation's history before switching back to the text-only profile.
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /opus-4\.8/ }).click();
  await expect(page.locator(".model-picker-label")).toHaveText("opus-4.8");
  await page.setInputFiles("#composer-file-input", {
    name: "historical.png",
    mimeType: "image/png",
    buffer: Buffer.from([137, 80, 78, 71]),
  });
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /deepseek-v4-pro/ }).click();
  const modal = page.getByTestId("model-switch-confirm");
  await expect(modal).toContainText("does not accept image input");
  await expect(modal).toContainText("saved conversation and existing text replies stay unchanged");
  await expect(modal.getByTestId("model-switch-dont-ask")).toHaveCSS("flex-direction", "row");
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toMatchObject({ id: "opus" });

  await modal.getByRole("button", { name: "Ignore & switch" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toMatchObject({ id: "default" });
  await expect(page.locator(".model-picker-label")).toHaveText("deepseek-v4-pro");
});

test("model switch warning can be permanently dismissed", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("bind this conversation model");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /opus-4\.8/ }).click();
  const modal = page.getByTestId("model-switch-confirm");
  await expect(modal.getByRole("checkbox", { name: "Don't ask again" })).not.toBeChecked();
  await modal.getByRole("checkbox", { name: "Don't ask again" }).check();
  await modal.getByRole("button", { name: "Yes, switch" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toMatchObject({ id: "opus" });
  await expect.poll(() => page.evaluate(() => localStorage.getItem("wisp-model-switch-warning-disabled")))
    .toBe("1");
  await expect(page.locator(".model-picker-label")).toHaveText("opus-4.8");

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /deepseek-v4-pro/ }).click();
  await expect(page.getByTestId("model-switch-confirm")).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toMatchObject({ id: "default" });
});

test("empty conversations switch models without warning", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /opus-4\.8/ }).click();

  await expect(page.getByTestId("model-switch-confirm")).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toMatchObject({
    id: "opus",
    sessionId: expect.any(String),
  });
  await expect(page.locator(".model-picker-label")).toHaveText("opus-4.8");
});

test("model selection stays bound to its conversation", async ({ page }) => {
  await enterApp(page, "/?mockSessionModels=1");
  await page.locator('[data-session-id="s-model-a"]').click();
  await expect(page.locator(".model-picker-label")).toHaveText("deepseek-v4-pro");

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /opus-4\.8/ }).click();
  await page.getByTestId("model-switch-confirm")
    .getByRole("button", { name: "Yes, switch" }).click();
  await expect(page.locator(".model-picker-label")).toHaveText("opus-4.8");

  await page.locator('[data-session-id="s-model-b"]').click();
  await expect(page.locator(".model-picker-label")).toHaveText("deepseek-v4-pro");
  await page.locator('[data-session-id="s-model-a"]').click();
  await expect(page.locator(".model-picker-label")).toHaveText("opus-4.8");
});

test("model effort is revealed on hover and saved to the model profile", async ({ page }) => {
  await enterApp(page, "/?mockSessionModels=1");
  await page.locator(".model-picker-btn").click();
  await page.mouse.move(0, 0);

  // Effort has no resting layout box, then overlays the model info on hover.
  const opusRow = page.locator(".model-menu-row", { hasText: "opus-4.8" });
  const deepseekRow = page.locator(".model-menu-row", { hasText: "deepseek-v4-pro" });
  await expect(opusRow.locator(".model-menu-effort-tag")).toBeHidden();
  await expect(deepseekRow.locator(".model-menu-effort-tag")).toBeHidden();
  expect(await opusRow.locator(".model-menu-effort-tag").boundingBox()).toBeNull();
  await opusRow.hover();
  await expect(opusRow.locator(".model-menu-effort-tag")).toHaveText("max");
  await expect(opusRow.locator(".model-menu-effort-tag")).toBeVisible();
  await expect(opusRow.locator(".model-menu-effort-edit")).toBeVisible();
  await expect(opusRow.locator(".model-menu-effort-edit svg")).toHaveCount(0);
  const [effortBox, textBox] = await Promise.all([
    opusRow.locator(".model-menu-effort-tag").boundingBox(),
    opusRow.locator(".model-menu-text").boundingBox(),
  ]);
  expect(effortBox).not.toBeNull();
  expect(textBox).not.toBeNull();
  expect(effortBox!.x).toBeGreaterThanOrEqual(textBox!.x);
  expect(effortBox!.x + effortBox!.width).toBeLessThanOrEqual(textBox!.x + textBox!.width + 1);

  const menuBoxBefore = await page.locator(".model-menu").boundingBox();
  await opusRow.locator(".model-menu-effort-edit").click();
  const flyout = page.locator(".model-menu-effort-flyout[data-effort-for='opus']");
  await expect(flyout).toBeVisible();
  await expect.poll(() => flyout.evaluate((el) => el.parentElement?.classList.contains("model-picker"))).toBe(true);
  const [menuBox, flyoutBox] = await Promise.all([
    page.locator(".model-menu").boundingBox(),
    flyout.boundingBox(),
  ]);
  expect(menuBox).not.toBeNull();
  expect(flyoutBox).not.toBeNull();
  expect(menuBoxBefore).not.toBeNull();
  expect(menuBox!.x).toBeLessThan(menuBoxBefore!.x);
  expect(flyoutBox!.x).toBeGreaterThanOrEqual(menuBox!.x + menuBox!.width + 5);
  expect(flyoutBox!.x + flyoutBox!.width).toBeLessThanOrEqual(page.viewportSize()!.width - 7);

  // Switching editors while the first flyout is open keeps the same stable
  // right-side anchor instead of recalculating from the already shifted menu.
  await deepseekRow.hover();
  await deepseekRow.locator(".model-menu-effort-edit").click();
  const deepseekFlyout = page.locator(".model-menu-effort-flyout");
  await expect(deepseekFlyout).toBeVisible();
  await expect(deepseekFlyout).not.toHaveAttribute("data-effort-for", "opus");
  const [switchedMenuBox, switchedFlyoutBox] = await Promise.all([
    page.locator(".model-menu").boundingBox(),
    deepseekFlyout.boundingBox(),
  ]);
  expect(switchedMenuBox).not.toBeNull();
  expect(switchedFlyoutBox).not.toBeNull();
  expect(Math.abs(switchedMenuBox!.x - menuBox!.x)).toBeLessThanOrEqual(1.5);
  expect(switchedFlyoutBox!.x).toBeGreaterThanOrEqual(switchedMenuBox!.x + switchedMenuBox!.width + 5);

  await opusRow.hover();
  await opusRow.locator(".model-menu-effort-edit").click();
  await expect(flyout).toBeVisible();
  // The stored value carries the check mark.
  await expect(
    flyout.locator(".model-menu-effort-option[data-effort='max'] .model-menu-effort-check"),
  ).toBeVisible();
  await flyout.locator(".model-menu-effort-option[data-effort='high']").click();

  // The effort is written onto the model profile, not the conversation.
  await expect.poll(() => lastInvokeArgs(page, "save_model")).toMatchObject({
    profile: { id: "opus", reasoning_effort: "high" },
  });
  await expect.poll(() => lastInvokeArgs(page, "set_session_reasoning_effort")).toBeNull();

  // The flyout closes, the menu stays open, and the row shows the new value.
  await expect(page.locator(".model-menu-effort-flyout")).toHaveCount(0);
  await expect(page.locator(".model-menu")).toBeVisible();
  await expect(opusRow.locator(".model-menu-effort-tag")).toHaveText("high");

  // "default" clears the profile value again.
  await opusRow.locator(".model-menu-effort-edit").click();
  await flyout.locator(".model-menu-effort-option[data-effort='default']").click();
  await expect.poll(() => lastInvokeArgs(page, "save_model")).toMatchObject({
    profile: { id: "opus", reasoning_effort: "" },
  });
  await expect(opusRow.locator(".model-menu-effort-tag")).toHaveCount(0);
});

test("model picker uses a compact left-aligned ACP group label", async ({ page }) => {
  await enterApp(page);
  await page.locator(".model-picker-btn").click();

  const label = page.locator(".model-menu-acp-label");
  const acpRowLabel = page.locator(".model-menu-row", { hasText: "Test ACP Agent" })
    .locator(".model-menu-label");
  await expect(label).toHaveText("ACP");
  await expect(label).not.toContainText("Agents");
  const [labelBox, rowLabelBox, labelPadding] = await Promise.all([
    label.boundingBox(),
    acpRowLabel.boundingBox(),
    label.evaluate((el) => Number.parseFloat(getComputedStyle(el).paddingLeft)),
  ]);
  expect(labelBox).not.toBeNull();
  expect(rowLabelBox).not.toBeNull();
  expect(Math.abs(labelBox!.x + labelPadding - rowLabelBox!.x)).toBeLessThanOrEqual(1);
});

test("Chinese reasoning effort title does not duplicate the English label", async ({ page }) => {
  await page.goto("/?mockSessionModels=1&mockLocale=zh");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator(".sidebar").getByRole("button", { name: "新建会话" })).toBeVisible();
  await page.locator(".model-picker-btn").click();
  const opusRow = page.locator(".model-menu-row", { hasText: "opus-4.8" });
  await opusRow.hover();
  await opusRow.locator(".model-menu-effort-edit").click();

  const title = page.locator(".model-menu-effort-flyout-label");
  await expect(title).toHaveText("推理强度");
  await expect(title).not.toContainText(/thinking effort/i);
});

test("effort flyout closes on Escape before the model menu", async ({ page }) => {
  await enterApp(page, "/?mockSessionModels=1");
  await page.locator(".model-picker-btn").click();
  await page
    .locator(".model-menu-row", { hasText: "opus-4.8" })
    .locator(".model-menu-effort-edit")
    .click();
  await expect(page.locator(".model-menu-effort-flyout")).toBeVisible();

  // One Escape closes only the flyout; the model menu stays open.
  await page.keyboard.press("Escape");
  await expect(page.locator(".model-menu-effort-flyout")).toHaveCount(0);
  await expect(page.locator(".model-menu")).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.locator(".model-menu")).toHaveCount(0);
});

test("Settings Models page can open ACP Agents dialog", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await expect(page.getByTestId("models-category-http")).toHaveClass(/active/);
  await page.getByTestId("open-acp-agents-from-settings").click();
  await expect(page.getByTestId("open-acp-agents-from-settings")).toHaveClass(/active/);
  await expect(page.getByTestId("acp-agents-list")).toBeVisible();
  await page.getByTestId("add-acp-agent-settings").click();
  await expect(page.getByTestId("acp-agents-settings")).toBeVisible();
  await expect(page.locator(".settings-breadcrumb")).toContainText(/ACP/);
});

test("ACP Agent settings create, test, and authenticate an installed agent", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await page.getByTestId("open-acp-agents-from-settings").click();
  await page.getByTestId("add-acp-agent-settings").click();
  const settings = page.getByTestId("acp-agents-settings");
  await expect(settings).toBeVisible();
  await expect(page.locator(".settings-breadcrumb")).toContainText(/ACP/);
  await settings.getByTestId("acp-agent-label").fill("My ACP");
  await settings.getByTestId("acp-agent-command").fill("my-acp");
  await settings.getByTestId("acp-agent-args").fill("--stdio\n  spaced  \n\n--safe");
  await settings.getByTestId("save-acp-agent").click();
  await expect(page.getByTestId("acp-agents-list")).toBeVisible();
  const row = page.getByTestId("acp-agent-row").filter({ hasText: "My ACP" });
  await expect(row).toBeVisible();
  await row.getByTestId("test-acp-agent").click();
  await expect(row.getByTestId("acp-agent-info")).toContainText("ACP v1");
  await row.getByTestId("authenticate-acp-agent").click();
  await expect.poll(() => lastInvokeArgs(page, "save_acp_agent")).toMatchObject({
    profile: { label: "My ACP", command: "my-acp", args: ["--stdio", "  spaced  ", "", "--safe"] },
  });
  await expect.poll(() => lastInvokeArgs(page, "authenticate_acp_agent")).toMatchObject({ methodId: "browser" });
});

test("ACP terminal authentication opens the advertised login command in the terminal dock", async ({ page }) => {
  await enterApp(page, "/?mockAcpTerminalAuth=1");
  await globalSettingsButton(page).click();
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await page.getByTestId("open-acp-agents-from-settings").click();
  await page.getByTestId("add-acp-agent-settings").click();
  const settings = page.getByTestId("acp-agents-settings");
  await settings.getByTestId("acp-agent-label").fill("Claude ACP");
  await settings.getByTestId("acp-agent-command").fill("npx");
  await settings.getByTestId("acp-agent-args").fill("-y\n@agentclientprotocol/claude-agent-acp");
  await settings.getByTestId("save-acp-agent").click();

  const row = page.getByTestId("acp-agent-row").filter({ hasText: "Claude ACP" });
  await row.getByTestId("test-acp-agent").click();
  await expect(row.getByTestId("acp-agent-info")).toContainText("Claude Agent 0.69.0 · ACP v1");
  await row.getByRole("button", { name: "Claude Subscription" }).click();

  await expect.poll(() => lastInvokeArgs(page, "authenticate_acp_agent")).toMatchObject({
    methodId: "claude-ai-login",
  });
  await expect(page.locator(".settings-page")).toHaveCount(0);
  const terminalDock = page.getByTestId("terminal-dock");
  await expect(terminalDock).toBeVisible();
  await expect(terminalDock.getByRole("tab", { name: "Claude ACP — Claude Subscription" })).toBeVisible();
  await expect(page.getByText("Authentication terminal opened. Complete sign-in there, then retry the ACP session.")).toBeVisible();
});

test("selecting an ACP Agent from a populated HTTP session starts a fresh session", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("existing HTTP turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  const firstSend = await lastInvokeArgs(page, "send_message");
  await composer(page).fill("preserved draft");

  await page.locator(".model-picker-btn").click();
  const agent = page.getByRole("button", { name: /Test ACP Agent/ });
  await expect(agent).toBeEnabled();
  await agent.click();
  await expect(page.locator(".model-picker-label")).toHaveText("Test ACP Agent");
  await expect(composer(page)).toHaveValue("preserved draft");
  await expect(page.locator(".copy-toast")).toContainText(
    "Started a new session because ACP cannot take over existing conversation history",
  );

  await composer(page).fill("continue with ACP");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    acpAgentId: "acp-test",
    message: "continue with ACP",
  });
  const secondSend = await lastInvokeArgs(page, "send_message");
  expect(secondSend.sessionId).not.toBe(firstSend.sessionId);
});

test("ACP turn maps config, overlapping tools, plan, usage, and exact permission response", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACP PERMISSION");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText("Hello from ACP.")).toBeVisible();
  const activity = page.locator(".steps.activity-summary").last();
  await activity.getByRole("button", { name: /Processed/ }).click();
  await expect(activity.getByTestId("acp-tool")).toHaveCount(2);
  await expect(page.getByText("Inspect")).toBeVisible();
  await expect(page.getByTestId("acp-session-config")).toHaveCount(0);
  await expect(page.locator(".model-picker-btn")).toContainText("Test ACP Agent");
  await page.locator(".model-picker-btn").click();
  const config = page.getByTestId("acp-session-config");
  await expect(config).toContainText("Agent");
  await expect(config).toContainText("Smart");
  await config.getByLabel("Model").selectOption("fast");
  await expect.poll(() => lastInvokeArgs(page, "set_acp_session_config")).toMatchObject({
    configId: "model", value: { value: "fast" },
  });
  await config.getByLabel("Session mode").selectOption("full-access");
  await expect.poll(() => lastInvokeArgs(page, "set_acp_session_mode")).toMatchObject({
    modeId: "full-access",
  });
  await expect(config.getByLabel("Session mode")).toHaveValue("full-access");
  const fastMode = config.getByLabel("Fast Mode");
  await expect(fastMode).not.toBeChecked();
  await config.locator("label.model-menu-config-row", { hasText: "Fast Mode" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_acp_session_config")).toMatchObject({
    configId: "fast_mode", value: { type: "boolean", value: true },
  });
  await expect(fastMode).toBeChecked();
  await page.keyboard.press("Escape");
  await expect(config).toHaveCount(0);

  const permission = page.getByTestId("acp-permission-card");
  await expect(permission).toBeVisible();
  await permission.getByRole("button", { name: "Allow once" }).click();
  await expect.poll(() => lastInvokeArgs(page, "respond_acp_permission")).toMatchObject({
    requestId: "permission-1", optionId: "allow",
  });
  await expect(permission).toHaveCount(0);
  const contextTrigger = page.getByTestId("context-usage-trigger");
  await expect(contextTrigger).toHaveText("15%");
  await expect(contextTrigger).toHaveAttribute("data-tone", "ok");
  await expect.poll(() => contextTrigger.evaluate((el) =>
    getComputedStyle(el).getPropertyValue("--context-gauge-angle").trim())).toBe("-76.5deg");
  await expect(page.locator(".topbar .hint")).toHaveCount(0);
  await contextTrigger.click();
  const contextPanel = page.getByTestId("context-usage-panel");
  await expect(contextPanel).toContainText("1.2K / 8K Tokens");
  await expect(contextPanel).toContainText("Agent-reported total");
  await page.keyboard.press("Escape");
  await expect(contextPanel).toHaveCount(0);

  await page.locator(".model-picker-btn").click();
  await expect(page.getByRole("button", { name: /deepseek-v4-pro/ })).toBeDisabled();
});

test("persisted wisp:plan reload rebuilds all entry states and priority", async ({ page }) => {
  await openMockPlanSession(page, "acp");

  const assertPlan = async () => {
    const card = page.getByTestId("plan-card");
    const completed = card.locator('li[data-status="completed"]');
    const inProgress = card.locator('li[data-status="in_progress"]');
    const pending = card.locator('li[data-status="pending"]');

    await expect(completed.locator(".plan-entry-mark")).toHaveText("✓");
    await expect(inProgress.locator(".plan-entry-mark")).toHaveText("▸");
    await expect(inProgress).toHaveCSS("font-weight", "600");
    await expect(inProgress.getByRole("img", { name: "High priority" })).toHaveText("!");
    await expect(pending.locator(".plan-entry-mark")).toHaveText("");
  };
  await assertPlan();

  await page.reload();
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();
  await page.locator('[data-session-id="s1"]').click();
  await assertPlan();
});

test("live plan is dashed while streaming and settles on Done", async ({ page }) => {
  await openMockPlanSession(page, "acp");
  await activateAcpPlanMode(page);
  await emitTauriEvent(page, "acp-session-update", {
    frameId: "s1",
    kind: "Plan",
    payload: {
      entries: [
        { content: "Stream the proposed change", status: "in_progress", priority: "high" },
      ],
    },
  });

  const live = page.getByTestId("plan-card").last();
  await expect(page.getByTestId("plan-card")).toHaveCount(2);
  await expect(live).toHaveClass(/streaming/);
  await expect(live).toHaveCSS("border-style", "dashed");
  await expect(live).toContainText("Planning…");

  await emitTauriEvent(page, "agent", {
    kind: "Done",
    frame_id: "s1",
    stop_reason: "end_turn",
  });
  await expect(live).not.toHaveClass(/streaming/);
  await expect(live).toHaveCSS("border-style", "solid");
  await expect(live).toContainText("Review before execution");
  await expect(live).not.toContainText("Planning…");
});

test("agent without plan mode renders a read-only compatibility plan", async ({ page }) => {
  await openMockPlanSession(page, "compat");

  const card = page.getByTestId("plan-card");
  await expect(card).toHaveClass(/compat/);
  await expect(card.getByTestId("plan-compat")).toHaveText("compat");
  await expect(card.locator(".plan-card-actions button")).toHaveCount(0);
});

test("plan action bar explains revisions and dispatches approve or defer", async ({ page }) => {
  await openMockPlanSession(page, "acp");
  await activateAcpPlanMode(page);

  await expect(page.getByTestId("plan-approve")).toBeVisible();
  await expect(page.getByTestId("plan-modify")).toHaveCount(0);
  await expect(page.getByTestId("plan-revision-hint")).toHaveText(
    "Not happy with the plan? Send your requested changes in the chat box.",
  );
  await expect(page.getByTestId("plan-save-exit")).toBeVisible();
  await expect(page.getByTestId("plan-save-exit")).toHaveText("Not now");

  await page.getByTestId("plan-save-exit").click();
  await expect(page.locator(".copy-toast")).toContainText("Plan not executed; Default mode restored");
  await expect.poll(() => invokeArgsList(page, "set_acp_session_mode")).toEqual([
    expect.objectContaining({ frameId: "s1", modeId: "default" }),
  ]);
  expect(await invokeArgsList(page, "send_message")).toHaveLength(0);

  await activateAcpPlanMode(page);
  await page.getByTestId("plan-approve").click();
  await expect.poll(() => invokeArgsList(page, "set_acp_session_mode")).toHaveLength(2);
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    sessionId: "s1",
    acpAgentId: "acp-test",
    message: "Approve and execute",
  });
});

test("built-in propose_plan renders a plan card with a working action bar", async ({ page }) => {
  await openMockPlanSession(page, "native");
  const menu = await openAgentMenu(page);
  await menu.locator("label.agent-menu-row", { hasText: "Plan first" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_plan_mode")).toMatchObject({
    sessionId: "s1", enabled: true,
  });
  await page.keyboard.press("Escape");

  // The built-in plan tool streams through the ordinary tool events; its result
  // is the card body, so neither event may leave a tool step behind.
  await emitTauriEvent(page, "agent", {
    kind: "ToolCall", frame_id: "s1", name: "propose_plan", preview: "1 steps",
  });
  await emitTauriEvent(page, "agent", {
    kind: "ToolResult", frame_id: "s1", name: "propose_plan", ok: true,
    content: JSON.stringify({
      v: 1,
      source: "native",
      entries: [{ content: "Wire propose_plan", status: "pending", priority: "high" }],
      note: "Plan submitted; end your turn.",
    }),
  });

  const live = page.getByTestId("plan-card").last();
  await expect(page.getByTestId("plan-card")).toHaveCount(2);
  await expect(live).toHaveClass(/streaming/);
  await expect(live).toContainText("Wire propose_plan");
  await expect(live).not.toContainText("end your turn");
  await expect(page.locator(".step")).toHaveCount(0);
  await expect(live.getByTestId("plan-compat")).toHaveCount(0);

  await emitTauriEvent(page, "agent", { kind: "Done", frame_id: "s1", stop_reason: "end_turn" });
  await expect(live).not.toHaveClass(/streaming/);

  await live.getByTestId("plan-approve").click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_plan_mode")).toMatchObject({
    sessionId: "s1", enabled: false,
  });
  // ?mock=1 send_message does not update the thread, so assert at the invoke layer.
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    sessionId: "s1", message: "Approve and execute",
  });
  await expect(page.locator(".copy-toast")).toContainText("Plan approved; execution is starting");
});

test("plan entries render full Markdown without breaking status layout", async ({ page }) => {
  await openMockPlanSession(page, "native");
  await emitTauriEvent(page, "agent", {
    kind: "ToolCall", frame_id: "s1", name: "propose_plan", preview: "1 steps",
  });
  await emitTauriEvent(page, "agent", {
    kind: "ToolResult", frame_id: "s1", name: "propose_plan", ok: true,
    content: JSON.stringify({
      v: 1, source: "native",
      entries: [{
        status: "pending", priority: "high",
        content: [
          "## **实现** `Fast`",
          "",
          "> 保留 [Issue #1025](https://github.com/xuzhougeng/wisp-science/issues/1025)",
          "",
          "- Chat Completions",
          "  - `service_tier=priority`",
          "",
          "```json",
          "{\\\"service_tier\\\":\\\"priority\\\"}",
          "```",
          "",
          "| 模式 | 值 |",
          "| --- | --- |",
          "| Fast | priority |",
        ].join("\n"),
      }],
    }),
  });
  const entry = page.getByTestId("plan-card").last().locator(".plan-entry-text");
  await expect(entry.locator("h2 strong")).toHaveText("实现");
  await expect(entry.locator("code").first()).toHaveText("Fast");
  await expect(entry.locator("blockquote")).toBeVisible();
  await expect(entry.locator("ul li")).toHaveCount(2);
  await expect(entry.locator("pre")).toBeVisible();
  await expect(entry.locator("table")).toBeVisible();
  await expect(entry).not.toContainText("**实现**");
  await expect(page.getByTestId("plan-card").last().getByRole("img", { name: "High priority" })).toBeVisible();
});

test("built-in ask_user renders a question card whose option click sends the answer", async ({ page }) => {
  await openMockPlanSession(page, "native");

  // The built-in question tool streams through the ordinary tool events; its
  // result is the card body, so neither event may leave a tool step behind.
  await emitTauriEvent(page, "agent", {
    kind: "ToolCall", frame_id: "s1", name: "ask_user", preview: "Which aligner?",
  });
  await emitTauriEvent(page, "agent", {
    kind: "ToolResult", frame_id: "s1", name: "ask_user", ok: true,
    content: JSON.stringify({
      v: 1,
      source: "native",
      question: "Which aligner should the pipeline use?",
      options: [
        { label: "STAR", description: "splice-aware, needs more RAM" },
        { label: "HISAT2", description: "lighter" },
      ],
      allow_freeform: true,
      note: "Question submitted; end your turn.",
    }),
  });

  const card = page.getByTestId("question-card");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Which aligner should the pipeline use?");
  await expect(card).toContainText("splice-aware, needs more RAM");
  await expect(card).not.toContainText("end your turn");
  await expect(page.locator(".step")).toHaveCount(0);
  // wry's window.prompt is a no-op, so the freeform answer must be in-app.
  await expect(card.locator(".plan-question-freeform input")).toBeVisible();

  await card.getByRole("button", { name: "STAR" }).click();
  // ?mock=1 send_message does not update the thread, so assert at the invoke layer.
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    sessionId: "s1", message: "STAR",
  });
  await expect(card).toHaveAttribute("data-state", "answered");
  await expect(card).toContainText("Answer sent to the agent");
  await expect(card.locator(".plan-question-options")).toHaveCount(0);
});

test("ask_user question body renders markdown instead of source markers", async ({ page }) => {
  await openMockPlanSession(page, "native");
  await emitTauriEvent(page, "agent", { kind: "ToolCall", frame_id: "s1", name: "ask_user", preview: "Confirm?" });
  await emitTauriEvent(page, "agent", {
    kind: "ToolResult", frame_id: "s1", name: "ask_user", ok: true,
    content: JSON.stringify({
      v: 1, source: "native",
      question: [
        "请先定怎么测。**已核实** Local Published: `release-abc`.",
        "",
        "- 纳入：`code/organized.R`",
        "- 排除：`code-example.r`",
        "",
        "详见 [PR #17](https://github.com/jarxunlai/ScientificFigureLibrary-community-archives/pull/17)。",
      ].join("\n"),
      options: [{ label: "先修 render.R", description: "给 organized.R 加上 `--input-dir`" }],
      allow_freeform: true,
      note: "Question submitted; end your turn.",
    }),
  });

  const body = page.getByTestId("question-card").locator(".plan-question-text");
  await expect(body.locator("strong")).toHaveText("已核实");
  await expect(body.locator("code").first()).toHaveText("release-abc");
  await expect(body.locator("ul li")).toHaveCount(2);
  await expect(body.locator("a[href*='pull/17']")).toBeVisible();
  await expect(body).not.toContainText("**已核实**");
  await expect(page.getByTestId("question-card")).not.toContainText("end your turn");
});

test("ACP ask_user card resolves through respond_ask_user and settles", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACP ASKUSER");
  await page.getByRole("button", { name: "Send" }).click();

  const card = page.getByTestId("question-card");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Which aligner should the pipeline use?");
  await expect(card).toHaveAttribute("data-state", "pending");

  await card.getByRole("button", { name: "HISAT2" }).click();
  // The ACP answer returns inside the agent's own turn, so it resolves the
  // pending bridge request instead of sending a user message.
  await expect.poll(() => lastInvokeArgs(page, "respond_ask_user")).toMatchObject({
    requestId: "ask-1", answer: "HISAT2",
  });
  await expect(card).toHaveAttribute("data-state", "answered");
  await expect(card).toContainText("Answer sent to the agent");
});

test("composer plan toggle routes ACP and built-in sessions separately", async ({ page }) => {
  await openMockPlanSession(page, "acp");
  let menu = await openAgentMenu(page);
  let row = menu.locator("label.agent-menu-row", { hasText: "Plan first" });
  let toggle = row.getByTestId("plan-first-toggle");
  await expect(row).toBeVisible();
  await expect(toggle).not.toBeChecked();
  await row.click();
  await expect.poll(() => lastInvokeArgs(page, "set_acp_session_mode")).toMatchObject({
    frameId: "s1",
    modeId: "plan",
  });
  await expect(toggle).toBeChecked();
  await row.click();
  await expect.poll(() => lastInvokeArgs(page, "set_acp_session_mode")).toMatchObject({
    frameId: "s1",
    modeId: "default",
  });
  await expect(toggle).not.toBeChecked();

  await openMockPlanSession(page, "native");
  menu = await openAgentMenu(page);
  row = menu.locator("label.agent-menu-row", { hasText: "Plan first" });
  toggle = row.getByTestId("plan-first-toggle");
  await expect(row).toBeVisible();
  await expect(toggle).not.toBeChecked();
  await row.click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_plan_mode")).toMatchObject({
    sessionId: "s1",
    enabled: true,
  });
  await expect(toggle).toBeChecked();
  await row.click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_plan_mode")).toMatchObject({
    sessionId: "s1",
    enabled: false,
  });
  await expect(toggle).not.toBeChecked();
});

test("Full Permission requires a warning and stays scoped to the conversation", async ({ page }) => {
  await enterApp(page);
  const menu = await openAgentMenu(page);
  const row = menu.locator("label.agent-menu-row", { hasText: "Full Permission" });
  const toggle = row.getByTestId("full-permission-toggle");
  await expect(toggle).not.toBeChecked();

  await row.click();
  await expect(page.getByRole("heading", { name: "Enable Full Permission?" })).toBeVisible();
  expect(await invokeArgsList(page, "set_session_full_permission")).toHaveLength(0);

  // The warning is the topmost surface: one immediate Escape closes only it,
  // leaving its Agent options parent open.
  await page.keyboard.press("Escape");
  await expect(page.getByRole("heading", { name: "Enable Full Permission?" })).toHaveCount(0);
  await expect(menu).toBeVisible();
  await expect(toggle).not.toBeChecked();

  await row.click();
  await page.getByRole("button", { name: "Enable Full Permission" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_full_permission")).toMatchObject({
    sessionId: expect.stringMatching(/^s-/),
    enabled: true,
  });
  await expect(toggle).toBeChecked();
  await expect(page.locator(".copy-toast")).toContainText(
    "Full Permission enabled for this conversation",
  );

  await row.click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_full_permission")).toMatchObject({
    enabled: false,
  });
  await expect(toggle).not.toBeChecked();
  await expect(page.getByRole("heading", { name: "Enable Full Permission?" })).toHaveCount(0);
});

test("ACP turns retain explicitly selected Wisp skills", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).pressSequentially("/lit");
  await page.locator(".mention-menu .mention-item")
    .filter({ hasText: "literature-review" })
    .click();
  await composer(page).fill("use this skill");
  await page.getByRole("button", { name: "Send" }).click();

  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    acpAgentId: "acp-test",
    references: [{ kind: "skill", name: "literature-review" }],
  });
});

test("ACP cancellation is scoped to the active bound frame", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACP LONG");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await page.getByRole("button", { name: "Stop" }).click();
  await expect.poll(() => lastInvokeArgs(page, "stop_agent")).toMatchObject({ sessionId: expect.any(String) });
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
  await expect(page.getByTestId("stopping-toast")).toHaveCount(0);
  await page.waitForTimeout(100);
  expect(await invokeArgsList(page, "propose_turn_memory")).toHaveLength(0);
});

test("an idle composer dismisses a leftover stopping banner", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACP LONG");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();

  await page.evaluate(() => { (window as any).__holdStopAgent = true; });
  await page.getByRole("button", { name: "Stop" }).click();
  const toast = page.getByTestId("stopping-toast");
  await expect(toast).toBeVisible();
  const composerInner = page.locator(".composer-inner");
  const toastBox = await toast.boundingBox();
  const innerBox = await composerInner.boundingBox();
  expect(toastBox).not.toBeNull();
  expect(innerBox).not.toBeNull();
  expect(toastBox!.y + toastBox!.height).toBeLessThanOrEqual(innerBox!.y + 1);

  // The turn can become idle (Send returns) without another Done. The banner
  // must not stay over the composer once the session is no longer running.
  await page.evaluate(() => { (window as any).__finishAcpLong(); });
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
  await expect(toast).toHaveCount(0);
});

test("stopping after a late Done does not leave the banner over Send", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACP LONG");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();

  const sessionId = await page.locator(".side-item.ses.active").getAttribute("data-session-id");
  expect(sessionId).toBeTruthy();
  // Done can land while send_message is still in flight, so Stop is still
  // clickable. That click must not pin the banner after the turn goes idle.
  await emitTauriEvent(page, "agent", {
    kind: "Done",
    frame_id: sessionId,
    stop_reason: "end_turn",
  });
  await page.evaluate(() => { (window as any).__holdStopAgent = true; });
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByTestId("stopping-toast")).toBeVisible();

  await page.evaluate(() => { (window as any).__finishAcpLong(); });
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
  await expect(page.getByTestId("stopping-toast")).toHaveCount(0);
});

test("switching conversations dismisses the previous conversation's stopping modal", async ({ page }) => {
  await enterApp(page, "/?mockPlanFlow=acp");
  await expect(page.locator('[data-session-id="s1"]')).toBeVisible();
  // Keep the seeded session as the navigation target, then run and stop work
  // in a fresh conversation.
  await newSessionButton(page).click();
  const runningSessionId = await page.locator(".side-item.ses.active").getAttribute("data-session-id");
  expect(runningSessionId).toBeTruthy();

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACP LONG");
  await page.getByRole("button", { name: "Send" }).click();
  await page.evaluate(() => { (window as any).__holdStopAgent = true; });
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.locator(".stopping-toast")).toBeVisible();

  const otherSession = page.locator(`.side-item.ses:not([data-session-id="${runningSessionId}"])`).first();
  await expect(otherSession).toBeVisible();
  await otherSession.click();
  await expect(page.locator(".stopping-toast")).toHaveCount(0);
});

test("failed stop command restores the Stop control instead of staying in Stopping", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACP LONG");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await page.evaluate(() => { (window as any).__failStopAgent = true; });

  await page.getByRole("button", { name: "Stop" }).click();

  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await expect(page.locator(".copy-toast-warning")).toContainText("Could not stop the task");
});

test("pre-start send failures roll back optimistic rows and restore the draft", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("PRESTARTFAIL retry this draft");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(composer(page)).toHaveValue("PRESTARTFAIL retry this draft");
  await expect(page.locator(".user-bubble")).toHaveCount(0);
});

test("post-start send failures keep the persisted user row and hide the phase prefix", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("POSTSTARTFAIL keep this turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.locator(".user-bubble")).toContainText("POSTSTARTFAIL keep this turn");
  await expect(page.locator(".finding.err")).toContainText("execution failed after turn/start");
  await expect(page.locator(".finding.err")).not.toContainText("[turn-started]");
  await expect(composer(page)).toHaveValue("");
});

test("post-start API errors keep the user bubble when an Error event lands first", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("POSTSTARTFAIL_EVENT keep question A");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.locator(".user-bubble")).toContainText("POSTSTARTFAIL_EVENT keep question A");
  await expect(page.locator(".finding.err")).toContainText("max_tokens too high");
  await expect(page.locator(".finding.err")).not.toContainText("[turn-started]");
  await expect(composer(page)).toHaveValue("");
  // Resume must still see the original question in the transcript.
  await expect(page.getByRole("button", { name: /Resume|继续执行/ })).toBeVisible();
});

test("truncated output auto-continue is shown as progress", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("AUTOCONTINUE long task");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByTestId("auto-continue-flag")).toContainText("automatically continued (1/10)");
  await expect(page.locator(".msg.assistant")).toContainText("Final segment");
  await expect(page.locator(".finding.err")).toHaveCount(0);
});

test("automatic reviewer resolves its finding and jumps past UI-only rows (#550)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("REVIEWBASE");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.locator(".msg.assistant")).toContainText("Earlier answer.");

  await composer(page).fill("AUTOREVIEW inspect the result");
  await page.getByRole("button", { name: "Send" }).click();

  const assistants = page.locator(".msg.assistant");
  await expect(assistants).toHaveCount(3);
  await expect(assistants.nth(1)).toContainText("5 significant genes");
  await expect(assistants.nth(2)).toContainText("Correction: the analysis found 3 significant genes");

  const handoffs = page.locator(".review-transition");
  await expect(handoffs).toHaveCount(2);
  await expect(handoffs.nth(0)).toContainText("wisp-science nudged Reviewer");
  await expect(handoffs.nth(0)).toHaveAttribute("data-phase", "reviewing");
  await expect(handoffs.nth(1)).toContainText("Reviewer nudged wisp-science");
  await expect(handoffs.nth(1)).toContainText("deepseek-v4-pro");
  await expect(handoffs.nth(1)).toHaveAttribute("data-phase", "correcting");

  const review = page.locator(".review-card");
  await expect(review).toContainText("Reviewer findings");
  await expect(review.locator(".review-model")).toHaveText("claude-sonnet-5 · high");
  await expect(review).toContainText("resolved");
  await expect(review).toContainText("All findings fixed and independently rechecked.");
  await expect(review.locator(".review-finding")).toHaveCount(1);
  const scroller = page.locator("#chat-scroller");
  await scroller.evaluate((element) => {
    element.style.height = "180px";
    element.scrollTop = element.scrollHeight;
  });
  await expect(assistants.nth(1)).not.toBeInViewport();
  await review.getByRole("button", { name: "Go to transcript" }).click();
  await expect.poll(async () => {
    const [target, viewport] = await Promise.all([
      assistants.nth(1).boundingBox(),
      scroller.boundingBox(),
    ]);
    return target && viewport ? Math.abs(target.y - viewport.y) : Number.POSITIVE_INFINITY;
  }).toBeLessThan(4);
});

test("automatic reviewer visibly returns a clean response without correction", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("AUTOREVIEWCLEAN inspect the result");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.locator(".msg.assistant")).toHaveCount(1);
  const handoffs = page.locator(".review-transition");
  await expect(handoffs).toHaveCount(2);
  await expect(handoffs.nth(0)).toHaveAttribute("data-phase", "reviewing");
  await expect(handoffs.nth(1)).toContainText("no issues found, please continue");
  await expect(handoffs.nth(1)).toHaveAttribute("data-phase", "passed");
  await expect(page.locator(".review-card")).toContainText("No traceability problems found");
});

test("ACP review with missing tool output is unreviewable instead of passed", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("AUTOREVIEWUNREVIEWABLE inspect the result");
  await page.getByRole("button", { name: "Send" }).click();

  const review = page.locator(".review-card");
  await expect(review.locator(".review-unreviewable")).toContainText("Evidence coverage is 0%");
  await expect(review).toContainText("Missing review evidence");
  await expect(review).toContainText("python analysis.py did not persist inspectable output");
  await expect(review.locator(".review-empty")).not.toContainText("No traceability problems found");
  await expect(page.locator('.review-transition[data-phase="passed"]')).toHaveCount(0);
});

test("review backend failures stay visible without failing the primary answer", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("AUTOREVIEWFAIL inspect the result");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.locator(".msg.assistant").first()).toContainText("The primary answer still completed");
  await expect(page.locator(".msg.assistant").last()).toContainText(
    "Review failed: ACP reviewer returned invalid JSON",
  );
});

test("assistant markdown table can be copied separately", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await enterApp(page);
  await composer(page).fill("MDTABLE");
  await page.getByRole("button", { name: "Send" }).click();
  const copyButton = page.locator(".msg.assistant .md-table-copy").first();
  await expect(copyButton).toBeVisible();
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, "writeText", {
      configurable: true,
      value: async (text: string) => { (window as any).__copiedTableText = text; },
    });
  });
  await copyButton.click();
  await expect(page.locator(".copy-toast")).toHaveText("Copied");
  await expect.poll(() => page.evaluate(() => (window as any).__copiedTableText)).toBe(
    "Tissue\tTPM\nVeg 0DAF\t2.62\nNotch 0DAF\t1.81",
  );
});

test("assistant markdown code block can be copied separately", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await enterApp(page);
  await composer(page).fill("MDCODE");
  await page.getByRole("button", { name: "Send" }).click();

  const copyButtons = page.locator(".msg.assistant .md-code-copy");
  await expect(copyButtons).toHaveCount(3);
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, "writeText", {
      configurable: true,
      value: async (text: string) => { (window as any).__copiedCodeText = text; },
    });
  });
  await copyButtons.first().click();

  await expect(page.locator(".copy-toast")).toHaveText("Copied");
  await expect.poll(() => page.evaluate(() => (window as any).__copiedCodeText)).toBe(
    "CAF状态 → 免疫变化\nCAF状态 → 上皮变化\n",
  );
});

test("dark palettes keep rendered markdown code readable", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await enterApp(page);
  await composer(page).fill("MDCODE");
  await page.getByRole("button", { name: "Send" }).click();

  const highlightedBlocks = page.locator(".msg.assistant .md pre code[data-hl='1']");
  await expect(highlightedBlocks).toHaveCount(3);
  await expect(page.locator(".msg.assistant code.language-text")).toContainText("CAF状态 → 免疫变化");
  await expect(page.locator(".msg.assistant code.language-python .hljs-comment")).toContainText("暗色代码注释");
  await expect(page.locator(".msg.assistant code.language-diff .hljs-addition")).toContainText("CAF状态 → 免疫变化");
  await expect(page.locator(".msg.assistant code.language-diff .hljs-deletion")).toContainText("CAF状态 → 未知");

  const auditContrast = () => page.locator(".msg.assistant .md").evaluate((root) => {
    const channels = (value: string) => (value.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number);
    const luminance = (value: string) => {
      const rgb = channels(value).map((channel) => channel / 255)
        .map((channel) => channel <= 0.04045
          ? channel / 12.92
          : ((channel + 0.055) / 1.055) ** 2.4);
      return 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    };
    const contrast = (foreground: string, background: string) => {
      const foregroundLuminance = luminance(foreground);
      const backgroundLuminance = luminance(background);
      return (Math.max(foregroundLuminance, backgroundLuminance) + 0.05)
        / (Math.min(foregroundLuminance, backgroundLuminance) + 0.05);
    };
    const samples = [...root.querySelectorAll("pre code.hljs")].flatMap((code) => {
      const preBackground = getComputedStyle(code.closest("pre")!).backgroundColor;
      return [code, ...code.querySelectorAll("span")]
        .filter((element) => element.textContent?.trim())
        .map((element) => {
          const style = getComputedStyle(element);
          const background = style.backgroundColor === "rgba(0, 0, 0, 0)"
            ? preBackground
            : style.backgroundColor;
          return {
            text: element.textContent?.trim().slice(0, 40),
            color: style.color,
            background,
            ratio: contrast(style.color, background),
          };
        });
    });
    return {
      minimum: Math.min(...samples.map((sample) => sample.ratio)),
      samples,
    };
  });

  await openSettingsSection(page, "Appearance");
  await page.getByTestId("theme-mode-dark").click();
  const paletteSelect = page.getByTestId("appearance-palette-select");
  for (const palette of ["charcoal", "codex", "github", "catppuccin", "gruvbox"]) {
    await paletteSelect.selectOption(palette);
    await expect(page.locator("html")).toHaveAttribute("data-dark-palette", palette);
    const audit = await auditContrast();
    expect(audit.minimum, `${palette}: ${JSON.stringify(audit.samples)}`).toBeGreaterThanOrEqual(4.5);
  }

  await page.getByTestId("theme-mode-system").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "system");
  const systemAudit = await auditContrast();
  expect(systemAudit.minimum, `system dark: ${JSON.stringify(systemAudit.samples)}`).toBeGreaterThanOrEqual(4.5);
});

test("composer @ # and / add typed context references", async ({ page }) => {
  await enterApp(page);
  const composerInput = composer(page);

  await composerInput.press("@");
  await expect(page.locator(".mention-menu")).toContainText("nif3.treefile");
  await page.locator(".mention-menu .mention-item").first().click();
  await expect(composerInput).toHaveValue("");
  await expect(composerInput).toBeFocused();

  await composerInput.pressSequentially("#Current");
  await expect(page.locator(".mention-menu")).toContainText("Current analysis");
  await page.locator(".mention-menu .mention-item").first().click();
  await expect(composerInput).toHaveValue("");
  await expect(composerInput).toBeFocused();

  await composerInput.pressSequentially("#project");
  await expect(page.locator(".mention-menu")).toContainText("Search every session in wisp-science");
  await page.locator(".mention-menu .mention-item").first().click();
  await expect(composerInput).toHaveValue("");
  await expect(composerInput).toBeFocused();

  await composerInput.pressSequentially("/lit");
  await expect(page.locator(".mention-menu")).toContainText("literature-review");
  await page.locator(".mention-menu .mention-item")
    .filter({ hasText: "literature-review" })
    .click();

  await composerInput.pressSequentially("/round");
  const workflowItem = page.locator(".mention-menu .mention-item")
    .filter({ hasText: "Roundtable" });
  await expect(workflowItem).toContainText("neutral chair synthesis");
  await workflowItem.click();

  await composerInput.pressSequentially("@gpu");
  const environmentItem = page.locator(".mention-menu .mention-item").filter({
    has: page.locator(".mention-item-name", { hasText: /^gpu-server$/ }),
  });
  await expect(environmentItem).toContainText("ssh:gpu-server");
  await environmentItem.click();
  const composerEnvironment = page.locator(
    '.composer-reference-card[data-reference-kind="context"]',
  );
  await expect(composerEnvironment).toContainText("gpu-server");
  await expect(composerEnvironment).toContainText("Environment");

  await composerInput.fill("use the attached context");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    references: [
      { kind: "artifact", id: "art-tree" },
      { kind: "session", id: "s-current" },
      { kind: "project", id: "default" },
      { kind: "skill", name: "literature-review" },
      { kind: "workflow", id: "roundtable" },
      { kind: "context", id: "ssh:gpu-server" },
    ],
  });
  const sentContext = page.locator(".msg.user .user-context-card");
  await expect(sentContext).toHaveCount(6);
  await expect(page.locator('.msg.user [data-reference-kind="artifact"]')).toContainText("nif3.treefile");
  await expect(page.locator('.msg.user [data-reference-kind="session"]')).toContainText("Current analysis");
  await expect(page.locator('.msg.user [data-reference-kind="project"]')).toContainText("wisp-science");
  await expect(page.locator('.msg.user [data-reference-kind="skill"]')).toContainText("literature-review");
  await expect(page.locator('.msg.user [data-reference-kind="workflow"]')).toContainText("Roundtable");
  const sentEnvironment = page.locator('.msg.user [data-reference-kind="context"]');
  await expect(sentEnvironment).toContainText("gpu-server");
  await expect(sentEnvironment).toContainText("Environment");
  await expect(sentEnvironment.locator("svg")).toBeVisible();
  await expect(page.locator(".msg.user .body")).not.toContainText("Selected skills:");
  await expect(page.locator(".msg.user .body")).not.toContainText("Target environments:");
});

test("composer / suggests the built-in /compact command", async ({ page }) => {
  await enterApp(page);
  const composerInput = composer(page);

  await composerInput.pressSequentially("/comp");
  // The matching command leads under its own section label.
  await expect(page.locator(".mention-menu .mention-group-label").first()).toHaveText("Commands");
  const commandItem = page.locator(".mention-menu .mention-item").filter({ hasText: "/compact" });
  await expect(commandItem).toContainText("Archive full history");
  await expect(commandItem.locator(".mention-item-icon svg")).toBeVisible();
  await commandItem.click();
  // Commands fill the composer instead of attaching a reference chip.
  await expect(composerInput).toHaveValue("/compact");
  await expect(composerInput).toBeFocused();
  await expect(page.locator(".composer-reference-card")).toHaveCount(0);

  await composerInput.fill("");
  await composerInput.pressSequentially("/comp");
  await expect(commandItem).toBeVisible();
  // Enter selects the highlighted command; it must not send the message yet.
  await composerInput.press("Enter");
  await expect(composerInput).toHaveValue("/compact");
  await expect(page.locator(".mention-menu")).toHaveCount(0);
  expect(await lastInvokeArgs(page, "send_message")).toBeNull();
});

test("composer slash commands run the matching shell actions", async ({ page }) => {
  await enterApp(page);
  const composerInput = composer(page);
  const menu = page.locator(".mention-menu");

  // Empty session lists the shell commands but hides the turn-bound ones.
  await composerInput.pressSequentially("/");
  await expect(menu.locator(".mention-group-label")).toHaveText(["Commands", "Workflows", "Skills"]);
  await expect(menu).toContainText("/compact");
  await expect(menu).toContainText("/fork");
  await expect(menu).toContainText("/btw");
  await expect(menu).toContainText("/plan");
  await expect(menu).toContainText("/permission");
  await expect(menu).toContainText("/save-as-skill");
  await expect(menu).toContainText("/skills");
  await expect(menu).toContainText("/files");
  await expect(menu).toContainText("/upload");
  await expect(menu).not.toContainText("/rewind");
  await expect(menu).not.toContainText("/review");
  await expect(menu).not.toContainText("/remember");
  await expect(menu).not.toContainText("/context");
  await expect(menu).not.toContainText("/share");
  await page.keyboard.press("Escape");

  // /btw opens the side chat; a payload goes straight to it.
  await composerInput.fill("/btw");
  await composerInput.press("Enter");
  const panel = page.locator(".rightpane");
  await expect(panel.locator(".sidechat-in-pane")).toBeVisible();
  expect(await lastInvokeArgs(page, "side_chat")).toBeNull();

  await composerInput.fill("/btw what did the main thread miss?");
  await composerInput.press("Enter");
  await expect.poll(() => lastInvokeArgs(page, "side_chat")).toMatchObject({
    question: "what did the main thread miss?",
  });

  // /save-as-skill drafts the skill-creator prompt into the composer.
  await composerInput.fill("/save-as-skill");
  await composerInput.press("Enter");
  await expect(composerInput).toHaveValue(/skill-creator/);
  await composerInput.fill("");

  // /skills opens the skills settings page.
  await composerInput.fill("/skills");
  await composerInput.press("Enter");
  await expect(page.locator(".settings-page")).toBeVisible();
  await expect(page.locator(".settings-nav button.active")).toHaveText("Skills");
  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toHaveCount(0);

  // /files opens the file panel in the right pane.
  await composerInput.fill("/files");
  await composerInput.press("Enter");
  await expect(panel.locator(".rp-files")).toBeVisible();

  // A completed turn unlocks the turn-bound commands.
  await composerInput.fill("hello there");
  await composerInput.press("Enter");
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  // Picked action commands run immediately without filling the composer.
  await composerInput.pressSequentially("/rev");
  await menu.locator(".mention-item").filter({ hasText: "/review" }).click();
  await expect(composerInput).toHaveValue("");
  await expect.poll(() => lastInvokeArgs(page, "review_session")).toMatchObject({
    sessionId: expect.stringMatching(/^s-/),
  });

  // /remember distills the latest turn, like the message-level Memory button.
  await composerInput.fill("/remember");
  await composerInput.press("Enter");
  const memoryModal = page.getByTestId("turn-memory-overlay");
  await expect(memoryModal).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "propose_turn_memory")).toMatchObject({
    turnIndex: 0,
    automatic: false,
  });
  await page.keyboard.press("Escape");
  await expect(memoryModal).toHaveCount(0);

  // /rewind previews the rollback of the latest turn, like the Undo button.
  await composerInput.fill("/rewind");
  await composerInput.press("Enter");
  const undoModal = page.getByTestId("turn-undo-modal");
  await expect(undoModal).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "preview_turn_undo")).toMatchObject({
    userIndex: 0,
  });
  await page.keyboard.press("Escape");
  await expect(undoModal).toHaveCount(0);

  // Unknown slash text still goes to the model untouched.
  await composerInput.fill("/notacommand");
  await composerInput.press("Enter");
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    message: "/notacommand",
  });

  // /fork fills via the picker; Enter then sends the payload as a branch,
  // exactly like the "Branch in new session" send-mode item.
  await composerInput.pressSequentially("/fork");
  await menu.locator(".mention-item").filter({ hasText: "/fork" }).click();
  await expect(composerInput).toHaveValue("/fork ");
  await composerInput.pressSequentially("branch this idea");
  await composerInput.press("Enter");
  await expect.poll(() => lastInvokeArgs(page, "branch_session")).toMatchObject({
    title: "branch this idea",
    checkpointKind: "after_response",
  });
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    sessionId: expect.stringMatching(/^branch-/),
    message: "branch this idea",
  });
});

test("/share exports selected, keyword-redacted messages as a PNG", async ({ page }) => {
  await enterApp(page);
  const composerInput = composer(page);
  const overlay = page.getByTestId("share-overlay");

  // Empty session: /share is hidden from the picker and opens nothing, and
  // the composer "+" menu entry and topbar share button are disabled.
  const topbarShare = page.getByTestId("share-topbar");
  await expect(topbarShare).toBeDisabled();
  await composerInput.pressSequentially("/sha");
  await expect(page.locator(".mention-menu .mention-item").filter({ hasText: "/share" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await composerInput.fill("/share");
  await composerInput.press("Enter");
  await expect(overlay).toHaveCount(0);
  expect(await lastInvokeArgs(page, "send_message")).toBeNull();
  await page.locator(".composer-plus").click();
  const shareItem = page.locator(".compose-item").filter({
    has: page.locator(".compose-item-label", { hasText: "Share" }),
  });
  await expect(shareItem).toBeDisabled();
  await page.locator(".compose-backdrop").click();
  await expect(shareItem).toHaveCount(0);

  // Seed one turn that streams a thinking block before the reply.
  await composerInput.fill("SHARETHINK check the spectrum");
  await composerInput.press("Enter");
  await expect(page.getByText("Alice confirmed the spectrum is clean.")).toBeVisible({ timeout: 10_000 });

  await composerInput.fill("/share");
  await composerInput.press("Enter");
  await expect(overlay).toBeVisible();
  // One Escape right after opening closes only the dialog; chat stays up.
  await page.keyboard.press("Escape");
  await expect(overlay).toHaveCount(0);
  await expect(composerInput).toBeVisible();

  // Reopen through the picker row; the action runs immediately.
  await composerInput.pressSequentially("/share");
  await page.locator(".mention-menu .mention-item").filter({ hasText: "/share" }).click();
  await expect(overlay).toBeVisible();
  await expect(overlay.getByRole("heading", { name: "Share as image" })).toBeVisible();

  // User and assistant rows are preselected; thinking is listed but hidden.
  const rows = overlay.locator(".share-row");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0).locator("input")).toBeChecked();
  await expect(rows.nth(1)).toHaveClass(/share-thinking/);
  await expect(rows.nth(1).locator("input")).not.toBeChecked();
  await expect(rows.nth(2).locator("input")).toBeChecked();

  // Each row shows a role badge; the counter tracks the selection.
  await expect(rows.nth(0).locator(".share-role")).toHaveText("You");
  await expect(rows.nth(1).locator(".share-role")).toHaveText("Thinking");
  await expect(rows.nth(2).locator(".share-role")).toHaveText("Assistant");
  await expect(overlay.locator(".share-count")).toHaveText("2/3 selected");

  // Keywords mask the preview text case-insensitively (raw Markdown source).
  await overlay.locator("#share-redact-input").fill("alice，spectrum");
  await expect(rows.nth(2)).toContainText("xxx confirmed the **xxx** is clean.");

  // Deselect the user turn and export: the PNG bytes reach the save command
  // and the saved toast closes the dialog.
  await rows.nth(0).locator("input").click();
  await expect(overlay.locator(".share-count")).toHaveText("1/3 selected");
  await overlay.getByTestId("share-export").click();
  await expect.poll(() => lastInvokeArgs(page, "save_share_image")).toMatchObject({
    defaultName: expect.stringMatching(/^wisp-share-\d{4}-\d{2}-\d{2}\.png$/),
  });
  const args = await lastInvokeArgs(page, "save_share_image");
  // The Markdown reply (heading/list/code fence) renders into a tall image.
  expect(String(args.pngBase64).length).toBeGreaterThan(10000);
  await expect(overlay).toHaveCount(0);

  // PNG width is the canvas width (IHDR bytes 16..20) at 2× scale: the
  // default is 840 px, and the width field overrides it.
  const pngWidth = (b64: string) =>
    page.evaluate((raw) => {
      const bytes = Uint8Array.from(atob(raw), (c) => c.charCodeAt(0));
      return new DataView(bytes.buffer).getUint32(16, false);
    }, b64);
  expect(await pngWidth(String(args.pngBase64))).toBe(1680);
  await composerInput.fill("/share");
  await composerInput.press("Enter");
  await expect(overlay).toBeVisible();
  await overlay.getByTestId("share-width-input").fill("600");
  await overlay.getByTestId("share-export").click();
  await expect.poll(async () => {
    const latest = await lastInvokeArgs(page, "save_share_image");
    return latest ? pngWidth(String(latest.pngBase64)) : 0;
  }).toBe(1200);
  await expect(overlay).toHaveCount(0);

  // HTML format: same dialog exports a self-contained rendered document.
  await composerInput.fill("/share");
  await composerInput.press("Enter");
  await expect(overlay).toBeVisible();
  await overlay.getByTestId("share-format-html").click();
  await expect(overlay.getByTestId("share-export")).toHaveText("Export HTML");
  await expect(overlay.getByTestId("share-width-input")).toHaveCount(0);
  await overlay.locator("#share-redact-input").fill("alice");
  await overlay.getByTestId("share-export").click();
  await expect.poll(() => lastInvokeArgs(page, "save_share_html")).toMatchObject({
    defaultName: expect.stringMatching(/^wisp-share-\d{4}-\d{2}-\d{2}\.html$/),
  });
  const html = String((await lastInvokeArgs(page, "save_share_html")).html);
  expect(html).toContain("<!doctype html>");
  expect(html).toContain("xxx confirmed the <strong>spectrum</strong>");
  expect(html).toContain("<h2>Fit summary</h2>");
  expect(html).toContain("<li>peak A at 530 nm</li>");
  expect(html).toContain("fit(spectrum)");
  expect(html).toContain("user-bubble");
  expect(html).toContain("body md");
  expect(html).toContain("--bg-panel");
  expect(html).not.toContain("#2f6fed");
  expect(html).not.toContain("class=\"card\"");
  const live = await page.evaluate(() => {
    const read = (el) => {
      if (!el) return { bg: "", color: "" };
      const style = getComputedStyle(el);
      return { bg: style.backgroundColor, color: style.color };
    };
    return {
      user: read(document.querySelector(".msg.user .body")),
      assistant: read(document.querySelector(".msg.assistant .body.md")),
      app: read(document.querySelector(".center")),
    };
  });
  const exported = await page.evaluate(async (documentHtml) => {
    const iframe = document.createElement("iframe");
    iframe.style.cssText = "position:fixed;left:-9999px;width:880px;height:640px;border:0";
    document.body.appendChild(iframe);
    const done = new Promise((resolve) => {
      iframe.addEventListener("load", () => resolve(), { once: true });
    });
    iframe.srcdoc = documentHtml;
    await done;
    const doc = iframe.contentDocument;
    const win = iframe.contentWindow;
    const read = (el) => {
      if (!el || !win) return { bg: "", color: "" };
      const style = win.getComputedStyle(el);
      return { bg: style.backgroundColor, color: style.color };
    };
    const result = {
      user: read(doc && doc.querySelector(".msg.user .body")),
      assistant: read(doc && doc.querySelector(".msg.assistant .body.md")),
      page: read(doc && doc.body),
    };
    iframe.remove();
    return result;
  }, html);
  expect(exported.user.bg).toBe(live.user.bg);
  expect(exported.user.color).toBe(live.user.color);
  expect(exported.assistant.color).toBe(live.assistant.color);
  expect(exported.page.bg).toBe(live.app.bg);
  await expect(overlay).toHaveCount(0);

  // The composer "+" menu offers the same entry once the session has content.
  await page.locator(".composer-plus").click();
  await expect(shareItem).toBeEnabled();
  await shareItem.click();
  await expect(overlay).toBeVisible();
  // One Escape closes only the dialog; the chat stays up.
  await page.keyboard.press("Escape");
  await expect(overlay).toHaveCount(0);
  await expect(composerInput).toBeVisible();

  // The topbar share icon is the same entry, next to the inbox bell.
  await expect(topbarShare).toBeEnabled();
  await topbarShare.click();
  await expect(overlay).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(overlay).toHaveCount(0);
  await expect(composerInput).toBeVisible();
});

test("/share PNG keeps markdown tables and KaTeX from the live thread", async ({ page }) => {
  await enterApp(page);
  const composerInput = composer(page);
  await composerInput.fill("SHARETABLEMATH export the table");
  await composerInput.press("Enter");
  const assistant = page.locator(".msg.assistant .body.md");
  await expect(assistant.locator("table")).toBeVisible({ timeout: 10_000 });
  await expect(assistant.getByRole("columnheader", { name: "项目" })).toBeVisible();
  await expect(assistant.getByRole("cell", { name: "A", exact: true })).toBeVisible();
  await expect(assistant.locator(".katex").first()).toBeVisible({ timeout: 10_000 });
  await expect(assistant.locator(".katex")).toHaveCount(2);

  await composerInput.fill("/share");
  await composerInput.press("Enter");
  const overlay = page.getByTestId("share-overlay");
  await expect(overlay).toBeVisible();
  await overlay.getByTestId("share-export").click();
  await expect.poll(() => lastInvokeArgs(page, "save_share_image")).not.toBeNull();
  const info = await page.evaluate(() => (window as any).__shareRasterInfo);
  expect(info).toMatchObject({ usedHtml: true, fallback: false });
  expect(info.tableCount).toBeGreaterThan(0);
  expect(info.mathCount).toBeGreaterThan(0);
  expect(info.katexCount).toBeGreaterThan(0);
  const args = await lastInvokeArgs(page, "save_share_image");
  expect(String(args.pngBase64).length).toBeGreaterThan(10000);
  const pngHeight = await page.evaluate((raw) => {
    const bytes = Uint8Array.from(atob(raw), (c) => c.charCodeAt(0));
    return new DataView(bytes.buffer).getUint32(20, false);
  }, String(args.pngBase64));
  // Header + table + two equations is taller than a short prose-only image.
  expect(pngHeight).toBeGreaterThan(400);
});

test("/share keeps a long picker at its scroll position and bounds exported prose", async ({ page }) => {
  await enterApp(page);
  const composerInput = composer(page);
  await composerInput.fill("SHARETHINK seed a long share list");
  await composerInput.press("Enter");
  await expect(page.getByText("Alice confirmed the spectrum is clean.")).toBeVisible({ timeout: 10_000 });
  const send = await lastInvokeArgs(page, "send_message");
  const frameId = String(send.sessionId ?? send.session_id ?? "");
  await page.evaluate((fid) => {
    const emit = (window as any).__tauriEmit;
    for (let index = 0; index < 12; index += 1) {
      emit("agent", {
        kind: "User",
        frame_id: fid,
        text: `follow-up ${index} ${"sample".repeat(45)}`,
      });
      emit("agent", {
        kind: "Text",
        frame_id: fid,
        delta: `Long result ${index}: https://example.test/${"unbroken".repeat(55)}`,
      });
    }
  }, frameId);
  await expect(page.locator(".msg.user")).toHaveCount(13);

  await page.getByTestId("share-topbar").click();
  const overlay = page.getByTestId("share-overlay");
  const list = overlay.locator(".share-list");
  const rows = overlay.locator(".share-row");
  await expect(rows).toHaveCount(27);
  const before = await list.evaluate((element: HTMLElement) => {
    element.scrollTop = element.scrollHeight;
    return element.scrollTop;
  });
  expect(before).toBeGreaterThan(0);

  // Updating one checkbox used to replace every row view, resetting the
  // scroll container to zero. Keyed rows must survive consecutive updates.
  await rows.nth(25).locator("input").click();
  await expect.poll(() => list.evaluate((element: HTMLElement) => element.scrollTop))
    .toBeGreaterThan(before * 0.8);
  const afterFirst = await list.evaluate((element: HTMLElement) => element.scrollTop);
  await rows.nth(24).locator("input").click();
  await expect.poll(() => list.evaluate((element: HTMLElement) => element.scrollTop))
    .toBeGreaterThan(afterFirst * 0.8);

  await overlay.getByTestId("share-format-html").click();
  await overlay.getByTestId("share-export").click();
  await expect.poll(() => lastInvokeArgs(page, "save_share_html")).not.toBeNull();
  const html = String((await lastInvokeArgs(page, "save_share_html")).html);
  const layout = await page.evaluate(async (documentHtml) => {
    const iframe = document.createElement("iframe");
    iframe.style.cssText = "position:fixed;left:-9999px;width:420px;height:640px;border:0";
    document.body.appendChild(iframe);
    const loaded = new Promise((resolve) => {
      iframe.addEventListener("load", () => resolve(undefined), { once: true });
    });
    iframe.srcdoc = documentHtml;
    await loaded;
    const doc = iframe.contentDocument!;
    const prose = [...doc.querySelectorAll<HTMLElement>(".msg .body")];
    const result = {
      pageWidth: doc.documentElement.clientWidth,
      pageScrollWidth: doc.documentElement.scrollWidth,
      overflowingProse: prose.filter((element) => element.scrollWidth > element.clientWidth + 1).length,
    };
    iframe.remove();
    return result;
  }, html);
  expect(layout.pageScrollWidth).toBeLessThanOrEqual(layout.pageWidth + 1);
  expect(layout.overflowingProse).toBe(0);
});

test("/share hides the social copy flow and keeps PNG plus HTML export", async ({ page }) => {
  await enterApp(page);
  const composerInput = composer(page);
  const overlay = page.getByTestId("share-overlay");

  await composerInput.fill("SHARETHINK check the spectrum");
  await composerInput.press("Enter");
  await expect(page.getByText("Alice confirmed the spectrum is clean.")).toBeVisible({ timeout: 10_000 });

  await composerInput.fill("/share");
  await composerInput.press("Enter");
  await expect(overlay).toBeVisible();
  await expect(overlay.getByRole("heading", { name: "Share as image" })).toBeVisible();
  await expect(overlay.locator(".share-row")).toHaveCount(3);
  // The platform picker and the skill-copy button stay hidden; HTML export
  // is available again next to the long PNG.
  await expect(overlay.getByTestId("share-social-skill")).toHaveCount(0);
  await expect(overlay.getByTestId("share-platform-xiaohongshu")).toHaveCount(0);
  await expect(overlay.getByTestId("share-platform-wechat")).toHaveCount(0);
  await expect(overlay.getByTestId("share-format-png")).toBeVisible();
  await expect(overlay.getByTestId("share-format-html")).toBeVisible();
  await expect(overlay.getByTestId("share-export")).toHaveText("Export PNG");
  await overlay.getByTestId("share-format-html").click();
  await expect(overlay.getByTestId("share-export")).toHaveText("Export HTML");
});

test("composer / menu layers sections and gives each command its own icon", async ({ page }) => {
  await enterApp(page);
  const composerInput = composer(page);

  await composerInput.pressSequentially("/");
  const menu = page.locator(".mention-menu");
  await expect(menu.locator(".mention-group-label")).toHaveText(["Commands", "Workflows", "Skills"]);
  const commandRows = menu.locator(".mention-item").filter({
    has: page.locator(".mention-item-name", { hasText: /^\// }),
  });
  await expect(commandRows.first()).toBeVisible();
  // Every command row carries an icon, and no two commands share one.
  const svgs = await commandRows
    .locator(".mention-item-icon svg")
    .evaluateAll((nodes) => nodes.map((node) => node.innerHTML));
  expect(svgs.length).toBeGreaterThan(5);
  expect(new Set(svgs).size).toBe(svgs.length);

  // A query matching no command keeps the remaining sections layered.
  // `/lit` hits both the literature skill and the literature-evidence workflow.
  await composerInput.fill("");
  await composerInput.pressSequentially("/lit");
  await expect(menu.locator(".mention-group-label")).toHaveText(["Workflows", "Skills"]);
  await expect(menu).toContainText("literature-review");
  await expect(menu).toContainText("Literature evidence review");
});

test("composer /plan and /permission drive the session mode flags", async ({ page }) => {
  await enterApp(page);
  const composerInput = composer(page);

  // /plan flips plan-first mode, exactly like the agent-menu toggle.
  await composerInput.fill("/plan");
  await composerInput.press("Enter");
  await expect.poll(() => lastInvokeArgs(page, "set_session_plan_mode")).toMatchObject({
    enabled: true,
  });
  await composerInput.fill("/plan");
  await composerInput.press("Enter");
  await expect.poll(() => lastInvokeArgs(page, "set_session_plan_mode")).toMatchObject({
    enabled: false,
  });

  // /permission fills the composer; the mode payload runs on Enter. Enabling
  // full permission always passes through the same warning as the toggle.
  await composerInput.pressSequentially("/perm");
  const menu = page.locator(".mention-menu");
  await menu.locator(".mention-item").filter({ hasText: "/permission" }).click();
  await expect(composerInput).toHaveValue("/permission ");
  await composerInput.pressSequentially("full");
  await composerInput.press("Enter");
  await expect(page.getByRole("heading", { name: "Enable Full Permission?" })).toBeVisible();
  expect(await invokeArgsList(page, "set_session_full_permission")).toHaveLength(0);
  await page.getByRole("button", { name: "Enable Full Permission" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_full_permission")).toMatchObject({
    enabled: true,
  });

  // Repeating full reports the mode is already on; ask returns to approvals.
  await composerInput.fill("/permission full");
  await composerInput.press("Enter");
  await expect(page.locator(".copy-toast")).toContainText("already on");
  await composerInput.fill("/permission ask");
  await composerInput.press("Enter");
  await expect.poll(() => lastInvokeArgs(page, "set_session_full_permission")).toMatchObject({
    enabled: false,
  });

  // auto is part of the mode family but not built yet.
  await composerInput.fill("/permission auto");
  await composerInput.press("Enter");
  await expect(page.locator(".copy-toast")).toContainText("not available yet");
});

test("composer picker follows manual caret insertions and ignores pasted text", async ({ page }) => {
  await enterApp(page);
  const composerInput = composer(page);

  // Windows Chinese IMEs can commit punctuation through the composition path
  // without exposing the committed character as InputEvent.data (#733).
  await composerInput.evaluate((element) => {
    const textarea = element as HTMLTextAreaElement;
    textarea.focus();
    textarea.value = "#";
    textarea.setSelectionRange(1, 1);
    textarea.dispatchEvent(new InputEvent("input", {
      bubbles: true,
      inputType: "insertCompositionText",
    }));
  });
  await expect(page.locator(".mention-menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await composerInput.fill("");

  await composerInput.fill("已有文字");
  await composerInput.evaluate((element) => {
    const textarea = element as HTMLTextAreaElement;
    textarea.focus();
    textarea.setSelectionRange(0, 0);
  });
  await composerInput.press("@");
  await expect(page.locator(".mention-menu")).toBeVisible();
  await page.keyboard.press("Escape");

  const original = "比较当前结果和旧结果";
  await composerInput.fill(original);
  await composerInput.evaluate((element) => {
    const textarea = element as HTMLTextAreaElement;
    textarea.focus();
    textarea.setSelectionRange(2, 2);
  });
  await composerInput.pressSequentially("#Current");
  await expect(page.locator(".mention-menu")).toContainText("Current analysis");
  await page.locator(".mention-menu .mention-item").first().click();
  await expect(composerInput).toHaveValue(original);
  await expect.poll(() => composerInput.evaluate((element) =>
    (element as HTMLTextAreaElement).selectionStart
  )).toBe(2);

  await composerInput.evaluate((element) => {
    const textarea = element as HTMLTextAreaElement;
    textarea.value = "#Current pasted";
    textarea.setSelectionRange(textarea.value.length, textarea.value.length);
    textarea.dispatchEvent(new InputEvent("input", {
      bubbles: true,
      data: "#Current pasted",
      inputType: "insertFromPaste",
    }));
  });
  await expect(page.locator(".mention-menu")).toHaveCount(0);
  await composerInput.press("x");
  await expect(page.locator(".mention-menu")).toHaveCount(0);
});

test("Ctrl+K opens the unified command palette and Shift+Enter attaches", async ({ page }) => {
  await enterApp(page);
  await page.keyboard.press("Control+k");
  const search = commandPalette(page);
  await expect(search).toBeVisible();
  await expect(search).toHaveAttribute("type", "text");
  await expect(search).toHaveAttribute("inputmode", "search");
  await expect(search).toHaveAttribute("autocomplete", "off");
  const paletteRows = page.locator(".project-search-overlay .project-search-row");
  await expect(paletteRows.first()).toBeVisible();
  // Row glyphs are inline Lucide-style SVGs from compose_icon; the row's
  // data-icon marks which kind each result uses (sessions show "bubble").
  const rowIcons = page.locator(".project-search-overlay .project-search-row > svg");
  await expect(rowIcons.first()).toBeVisible();
  const iconBox = await rowIcons.first().boundingBox();
  expect(iconBox?.width ?? 0).toBeLessThanOrEqual(24);
  await search.press("ArrowDown");
  await expect(paletteRows.nth(1)).toHaveClass(/active/);
  await search.fill("counts");
  await expect(page.locator(".project-search-row").filter({ hasText: "counts.csv" })).toBeVisible();
  await expect(page.locator(".project-search-row").filter({ hasText: "Current analysis" })).toBeVisible();
  const sessionTitles = await page
    .locator(".project-search-row[data-icon='bubble'] .project-search-title")
    .allTextContents();
  expect(sessionTitles.indexOf("Current analysis"))
    .toBeLessThan(sessionTitles.indexOf("Cross-project counts"));
  await expect.poll(() => lastInvokeArgs(page, "search_sessions")).toMatchObject({
    query: "counts",
    preferredProjectId: "default",
  });
  await search.press("Shift+Enter");
  await expect(search).not.toBeVisible();
  await expect(page.locator(".composer-reference-chips")).toContainText(/counts\.csv|Cross-project counts/);
});

test("Ctrl+K opens in place and Ctrl+Enter opens a project window", async ({ page }) => {
  await pinNonMacPlatform(page);
  await enterApp(page);
  await page.keyboard.press("Control+k");
  const search = commandPalette(page);
  await search.fill("Other project");
  const projectRow = page.locator(".project-search-row").filter({ hasText: "Other project" });
  await expect(projectRow.locator(".project-window-shortcut")).toHaveText("Ctrl↵ open in new window");

  await search.press("Enter");
  await expect.poll(() => lastInvokeArgs(page, "open_project")).toMatchObject({ id: "other" });
  await expect.poll(() => lastInvokeArgs(page, "open_project_window")).toBeNull();

  await page.keyboard.press("Control+k");
  await search.fill("Other project");
  await search.press("Control+Enter");
  await expect.poll(() => lastInvokeArgs(page, "open_project_window")).toMatchObject({
    id: "other",
  });
});

test("the needs-you inbox opens cross-project sessions in their own window", async ({ page }) => {
  await enterApp(page);
  const bell = page.locator(".inbox-wrap .icon-btn");
  await expect(bell.locator(".inbox-badge")).toHaveText("1");
  await bell.click();
  const item = page.locator(".inbox-item");
  await expect(item).toContainText("Other project");
  await expect(item).toContainText("Cross-project counts");
  await page.keyboard.press("Escape");
  await expect(page.locator(".inbox-drop")).toHaveCount(0);
  await expect(bell).toBeVisible();

  await bell.click();
  await expect(item).toBeVisible();
  await item.click();
  // Cross-project targets go to the project's own window (#423), not this one.
  await expect.poll(() => lastInvokeArgs(page, "open_project_window")).toMatchObject({
    id: "other",
    session: "s-other",
  });
  await expect(page.locator(".inbox-drop")).not.toBeVisible();
});

test("topbar groups chrome actions and hides an empty status hint", async ({ page }) => {
  await enterApp(page);
  const actions = page.locator(".topbar-actions");
  await expect(actions).toBeVisible();
  await expect(actions.getByTestId("share-topbar")).toHaveCount(1);
  await expect(actions.getByTestId("share-topbar")).toBeDisabled();
  await expect(actions.locator(".inbox-wrap")).toHaveCount(1);
  await expect(actions.getByRole("button", { name: "Open terminal" })).toHaveCount(1);
  await expect(actions.getByRole("button", { name: "Toggle panel" })).toHaveCount(1);
  // Idle sessions leave the status slot empty so tabs keep the width.
  await expect(page.locator(".topbar .hint")).toHaveCount(0);
});

test("context usage moves out of the topbar and opens a categorized detail panel", async ({ page }) => {
  await page.setViewportSize({ width: 1516, height: 671 });
  await enterApp(page);
  await page.locator("#composer-input").fill("CONTEXTUSAGE");
  await page.getByRole("button", { name: "Send", exact: true }).click();

  const trigger = page.getByTestId("context-usage-trigger");
  await expect(trigger).toHaveText("62%");
  await expect(trigger).toHaveAttribute("data-tone", "ok");
  await expect(trigger).toHaveAttribute("title", /79\.9K \/ 128K tokens/);
  await expect.poll(() => trigger.evaluate((el) =>
    getComputedStyle(el).getPropertyValue("--context-gauge-angle").trim())).toBe("-34.2deg");
  await expect.poll(() => trigger.locator("svg path").first().evaluate((el) =>
    getComputedStyle(el).transform)).not.toBe("none");
  const [triggerBox, modelBox] = await Promise.all([
    trigger.boundingBox(),
    page.locator(".model-picker-btn").boundingBox(),
  ]);
  expect(triggerBox).not.toBeNull();
  expect(modelBox).not.toBeNull();
  expect(triggerBox!.x + triggerBox!.width).toBeLessThanOrEqual(modelBox!.x);
  await expect(page.locator(".topbar .hint")).toHaveCount(0);

  await trigger.click();
  const panel = page.getByTestId("context-usage-panel");
  await expect(panel).toBeVisible();
  await expect(panel.getByRole("heading", { name: "Context Usage" })).toBeVisible();
  await expect(panel).toContainText("62% Full");
  await expect(panel.getByTestId("context-usage-nudge")).toHaveCount(0);
  // The limit tracks the session's bound model (128K), not the stale
  // max_context carried by the last turn's usage event (300K).
  await expect(panel).toContainText("~79.9K / 128K Tokens");
  await expect(panel.locator(".context-usage-row")).toHaveCount(7);
  await expect(panel.locator(".context-usage-segment")).toHaveCount(7);
  const firstUsageRow = panel.locator(".context-usage-row").first();
  const paletteColors = await firstUsageRow.evaluate((row) => {
    const root = document.documentElement;
    const originalTheme = root.getAttribute("data-theme");
    const originalLight = root.getAttribute("data-light-palette");
    const originalDark = root.getAttribute("data-dark-palette");
    (row as HTMLElement).style.transition = "none";
    root.setAttribute("data-theme", "light");
    root.setAttribute("data-light-palette", "paper");
    const paper = getComputedStyle(row).backgroundColor;
    root.setAttribute("data-light-palette", "codex");
    const codex = getComputedStyle(row).backgroundColor;
    root.setAttribute("data-theme", "dark");
    root.setAttribute("data-dark-palette", "charcoal");
    const charcoal = getComputedStyle(row).backgroundColor;
    for (const [attribute, value] of [
      ["data-theme", originalTheme],
      ["data-light-palette", originalLight],
      ["data-dark-palette", originalDark],
    ]) {
      if (value === null) root.removeAttribute(attribute);
      else root.setAttribute(attribute, value);
    }
    (row as HTMLElement).style.removeProperty("transition");
    return { paper, codex, charcoal };
  });
  expect(paletteColors.paper).not.toBe("rgb(239, 239, 239)");
  expect(new Set(Object.values(paletteColors)).size).toBe(3);
  await expect(panel.getByText("Conversation", { exact: true })).toBeVisible();
  await expect(panel.getByText("36.3K", { exact: true })).toBeVisible();
  await expect(panel.locator(".context-usage-row.expandable")).toHaveCount(6);

  await panel.getByText("System prompt", { exact: true }).click();
  await expect(panel.locator(".context-usage-detail")).toContainText("You are wisp-science");
  await panel.getByText("Tool definitions", { exact: true }).click();
  await expect(panel.locator(".context-usage-detail")).toContainText("read");
  await expect(panel.locator(".context-usage-detail")).toContainText("Read a file from disk");
  await expect(panel.getByText("Conversation", { exact: true }).locator("xpath=..")).toBeDisabled();

  // offsetWidth ignores the enter animation's scale transform; clientWidth is
  // the absolute containing block (composer padding box).
  const widths = await page.evaluate(() => {
    const panelEl = document.querySelector(
      "[data-testid='context-usage-panel']",
    ) as HTMLElement | null;
    const composerEl = document.querySelector(".composer-inner") as HTMLElement | null;
    return {
      panel: panelEl?.offsetWidth ?? 0,
      composerClient: composerEl?.clientWidth ?? 0,
    };
  });
  expect(Math.abs(widths.panel - widths.composerClient)).toBeLessThan(4);

  // Window-level Escape must work immediately; focus never moves into the panel.
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect(page.locator(".composer-inner")).toBeVisible();
  await expect(trigger).toBeVisible();
});

test("legacy native usage totals fall back to Conversation, not Agent-managed", async ({ page }) => {
  await enterApp(page);
  await page.locator("#composer-input").fill("CONTEXTUSAGELEGACY");
  await page.getByRole("button", { name: "Send", exact: true }).click();

  const trigger = page.getByTestId("context-usage-trigger");
  await expect(trigger).toHaveText("20%");
  await expect(trigger).toHaveAttribute("data-tone", "ok");
  await expect.poll(() => trigger.evaluate((el) =>
    getComputedStyle(el).getPropertyValue("--context-gauge-angle").trim())).toBe("-72.0deg");
  await trigger.click();
  const panel = page.getByTestId("context-usage-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("~25.4K / 128K Tokens");
  await expect(panel.locator(".context-usage-row")).toHaveCount(7);
  await expect(panel.getByText("Conversation", { exact: true })).toBeVisible();
  await expect(panel.getByText("25.4K", { exact: true })).toBeVisible();
  await expect(panel.getByText("Agent-managed context")).toHaveCount(0);
});

test("context usage limit follows the session's current model", async ({ page }) => {
  await enterApp(page);
  await page.locator("#composer-input").fill("CONTEXTUSAGE");
  await page.getByRole("button", { name: "Send", exact: true }).click();

  const trigger = page.getByTestId("context-usage-trigger");
  await expect.poll(() => trigger.evaluate((el) =>
    getComputedStyle(el).getPropertyValue("--context-gauge-angle").trim())).toBe("-34.2deg");
  await trigger.click();
  await expect(page.getByTestId("context-usage-panel")).toContainText("~79.9K / 128K Tokens");
  await page.keyboard.press("Escape");

  // Switching the session's model re-bases the limit immediately — no new turn.
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /opus-4\.8/ }).click();
  await page.getByTestId("model-switch-confirm")
    .getByRole("button", { name: "Yes, switch" }).click();
  await expect(page.locator(".model-picker-label")).toHaveText("opus-4.8");
  await expect.poll(() => trigger.evaluate((el) =>
    getComputedStyle(el).getPropertyValue("--context-gauge-angle").trim())).toBe("-54.0deg");
  await expect(trigger).toHaveText("40%");
  await trigger.click();
  await expect(page.getByTestId("context-usage-panel")).toContainText("~79.9K / 200K Tokens");
});

test("context usage trigger colors the live percent at warn and danger thresholds (#931)", async ({ page }) => {
  await enterApp(page);

  await page.locator("#composer-input").fill("CONTEXTUSAGEWARN");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  const trigger = page.getByTestId("context-usage-trigger");
  await expect(trigger).toHaveText("72%");
  await expect(trigger).toHaveAttribute("data-tone", "warn");
  await expect(trigger).toHaveClass(/is-warn/);
  await trigger.click();
  await expect(page.getByTestId("context-usage-panel")).toBeVisible();
  await expect(page.getByTestId("context-usage-nudge")).toHaveCount(0);
  await page.keyboard.press("Escape");

  await newSessionButton(page).click();
  await page.locator("#composer-input").fill("CONTEXTUSAGEDANGER");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(trigger).toHaveText("91%");
  await expect(trigger).toHaveAttribute("data-tone", "danger");
  await expect(trigger).toHaveClass(/is-danger/);
});

test("danger context usage panel offers compact and a new session (#931)", async ({ page }) => {
  await enterApp(page);
  await page.locator("#composer-input").fill("CONTEXTUSAGEDANGER");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  const trigger = page.getByTestId("context-usage-trigger");
  await expect(trigger).toHaveText("91%");
  await trigger.click();
  const panel = page.getByTestId("context-usage-panel");
  const nudge = panel.getByTestId("context-usage-nudge");
  await expect(nudge).toBeVisible();
  await expect(nudge).toContainText("Window is almost full");
  await nudge.getByRole("button", { name: "Compact" }).click();
  await expect(panel).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    message: "/compact",
    resume: false,
  });

  await page.locator("#composer-input").fill("CONTEXTUSAGEDANGER");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(trigger).toHaveText("91%");
  await trigger.click();
  await page.getByTestId("context-usage-new-session").click();
  await expect(page.getByTestId("context-usage-panel")).toHaveCount(0);
  await expect(page.locator(".empty")).toBeVisible();
});

test("context usage keeps the running agent window until a model switch boundary", async ({ page }) => {
  await enterApp(page);
  await page.locator("#composer-input").fill("CONTEXTUSAGERUNNING");
  await page.getByRole("button", { name: "Send", exact: true }).click();

  const trigger = page.getByTestId("context-usage-trigger");
  await expect.poll(() => trigger.evaluate((el) =>
    getComputedStyle(el).getPropertyValue("--context-gauge-angle").trim())).toBe("-34.2deg");

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /opus-4\.8/ }).click();
  await page.getByTestId("model-switch-confirm")
    .getByRole("button", { name: "Yes, switch" }).click();

  // The binding changes immediately, but the in-flight request remains on
  // the old 128K Agent until Done.
  await expect.poll(() => trigger.evaluate((el) =>
    getComputedStyle(el).getPropertyValue("--context-gauge-angle").trim())).toBe("-34.2deg");
  await expect(page.getByRole("button", { name: "Send", exact: true })).toBeVisible({ timeout: 3_000 });
  await expect.poll(() => trigger.evaluate((el) =>
    getComputedStyle(el).getPropertyValue("--context-gauge-angle").trim())).toBe("-54.0deg");
});

async function openContextUsagePanel(page: Page) {
  await page.locator("#composer-input").fill("CONTEXTUSAGE");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  const trigger = page.getByTestId("context-usage-trigger");
  await expect.poll(() => trigger.evaluate((el) =>
    getComputedStyle(el).getPropertyValue("--context-gauge-angle").trim())).toBe("-34.2deg");
  await trigger.click();
  const panel = page.getByTestId("context-usage-panel");
  await expect(panel).toBeVisible();
  await expect.poll(() => panel.evaluate((el) => (el as HTMLElement).offsetHeight)).toBeGreaterThan(80);
  return { trigger, panel };
}

test("context usage docks above the composer without covering the latest reply", async ({ page }) => {
  await page.setViewportSize({ width: 1516, height: 900 });
  await enterApp(page);
  const { trigger, panel } = await openContextUsagePanel(page);

  await expect(panel).toHaveAttribute("data-mode", "docked");
  await expect(page.locator(".context-usage-backdrop")).toHaveCount(0);
  await expect.poll(async () => page.evaluate(() => {
    const chat = document.getElementById("chat-scroller")?.getBoundingClientRect();
    const card = document.querySelector("[data-testid='context-usage-panel']")?.getBoundingClientRect();
    if (!chat || !card) return false;
    const stacked = card.top >= chat.bottom - 2;
    const chatStillOpen = chat.height > 40;
    return stacked && chatStillOpen;
  })).toBe(true);
  const lastBox = await page.getByText("Context usage is ready.").boundingBox();
  const panelBox = await panel.boundingBox();
  expect(lastBox).not.toBeNull();
  expect(panelBox).not.toBeNull();
  const covered = lastBox!.y + lastBox!.height > panelBox!.y + 2
    && lastBox!.y < panelBox!.y + panelBox!.height - 2;
  expect(covered).toBe(false);
  expect(panelBox!.height).toBeLessThanOrEqual(page.viewportSize()!.height * 0.46 + 4);

  const widths = await page.evaluate(() => {
    const panelEl = document.querySelector(
      "[data-testid='context-usage-panel']",
    ) as HTMLElement | null;
    const composerEl = document.querySelector(".composer-inner") as HTMLElement | null;
    return {
      panel: panelEl?.offsetWidth ?? 0,
      composerClient: composerEl?.clientWidth ?? 0,
    };
  });
  expect(Math.abs(widths.panel - widths.composerClient)).toBeLessThan(4);

  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect(page.getByText("Context usage is ready.")).toBeInViewport();
  await expect(trigger).toBeVisible();
});

test("docked context usage lets the first outside click land", async ({ page }) => {
  await enterApp(page);
  const { panel } = await openContextUsagePanel(page);

  const input = page.locator("#composer-input");
  await input.click();
  await expect(panel).toHaveCount(0);
  await expect(input).toBeFocused();

  await page.getByTestId("context-usage-trigger").click();
  await expect(page.getByTestId("context-usage-panel")).toBeVisible();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await expect(page.getByTestId("context-usage-panel")).toHaveCount(0);
  await expect(page.locator(".rightpane")).toBeVisible();
});

test("context usage can float, stay open while typing, and remember geometry", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await enterApp(page);
  const { trigger, panel } = await openContextUsagePanel(page);

  const head = page.getByTestId("context-usage-head");
  const start = await head.boundingBox();
  expect(start).not.toBeNull();
  await page.mouse.move(start!.x + 48, start!.y + start!.height / 2);
  await page.mouse.down();
  await page.mouse.move(start!.x + 220, start!.y - 120, { steps: 10 });
  await page.mouse.up();

  await expect.poll(() => panel.getAttribute("data-mode")).toBe("floating");
  await expect(page.getByTestId("context-usage-dock")).toBeVisible();
  await expect(page.getByTestId("context-usage-resize")).toBeVisible();
  const floated = await panel.boundingBox();
  expect(floated).not.toBeNull();
  expect(floated!.x).toBeGreaterThanOrEqual(0);
  expect(floated!.y).toBeGreaterThanOrEqual(0);
  expect(floated!.x + floated!.width).toBeLessThanOrEqual(1280);
  expect(floated!.y + floated!.height).toBeLessThanOrEqual(800);

  const input = page.locator("#composer-input");
  await input.click();
  await expect(panel).toBeVisible();
  await input.fill("still here");
  await expect(input).toHaveValue("still here");
  await expect(panel).toBeVisible();

  await page.locator(".inbox-wrap .icon-btn").click();
  await expect(page.locator(".inbox-drop")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".inbox-drop")).toHaveCount(0);
  await expect(panel).toBeVisible();

  const parked = await panel.boundingBox();
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  await trigger.click();
  await expect(panel).toBeVisible();
  await expect(panel).toHaveAttribute("data-mode", "floating");
  await expect.poll(async () => {
    const restored = await panel.boundingBox();
    return restored && Math.abs(restored.x - parked!.x) < 3 && Math.abs(restored.y - parked!.y) < 3;
  }).toBe(true);

  await page.getByTestId("context-usage-dock").click();
  await expect.poll(() => panel.getAttribute("data-mode")).toBe("docked");
  await expect(page.getByTestId("context-usage-resize")).toHaveCount(0);

  await page.mouse.move(start!.x + 48, start!.y + start!.height / 2);
  const headAgain = await head.boundingBox();
  await page.mouse.move(headAgain!.x + 48, headAgain!.y + headAgain!.height / 2);
  await page.mouse.down();
  await page.mouse.move(headAgain!.x + 180, headAgain!.y - 90, { steps: 8 });
  await page.mouse.up();
  await expect.poll(() => panel.getAttribute("data-mode")).toBe("floating");
  await head.dblclick();
  await expect.poll(() => panel.getAttribute("data-mode")).toBe("docked");
});

test("floating context usage resizes and reflows the bar and details", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await enterApp(page);
  const { panel } = await openContextUsagePanel(page);
  const head = page.getByTestId("context-usage-head");
  const start = await head.boundingBox();
  await page.mouse.move(start!.x + 40, start!.y + start!.height / 2);
  await page.mouse.down();
  await page.mouse.move(start!.x + 80, start!.y - 40, { steps: 6 });
  await page.mouse.up();
  await expect.poll(() => panel.getAttribute("data-mode")).toBe("floating");

  await panel.getByText("System prompt", { exact: true }).click();
  const detail = panel.locator(".context-usage-detail");
  await expect(detail).toBeVisible();

  // Park the floating card toward the top-left so the corner grip has room
  // to grow before hitting the window clamp.
  await page.evaluate(() => {
    const el = document.querySelector("[data-testid='context-usage-panel']") as HTMLElement;
    el.style.left = "24px";
    el.style.top = "24px";
    el.style.width = "360px";
    el.style.height = "280px";
    el.style.setProperty("--context-usage-h", "280px");
  });
  const before = await panel.boundingBox();
  const detailBefore = await detail.boundingBox();
  expect(before).not.toBeNull();
  expect(detailBefore).not.toBeNull();

  const grip = page.getByTestId("context-usage-resize");
  const gripBox = await grip.boundingBox();
  expect(gripBox).not.toBeNull();
  await page.mouse.move(gripBox!.x + gripBox!.width / 2, gripBox!.y + gripBox!.height / 2);
  await page.mouse.down();
  await page.mouse.move(gripBox!.x + 160, gripBox!.y + 120, { steps: 8 });
  await page.mouse.up();

  await expect.poll(async () => (await panel.boundingBox())!.width).toBeGreaterThan(before!.width + 40);
  await expect.poll(async () => (await panel.boundingBox())!.height).toBeGreaterThan(before!.height + 40);
  const after = await panel.boundingBox();
  const barAfter = await panel.locator(".context-usage-bar").boundingBox();
  const detailAfter = await detail.boundingBox();
  expect(after).not.toBeNull();
  expect(barAfter).not.toBeNull();
  expect(detailAfter).not.toBeNull();
  expect(Math.abs(after!.width - barAfter!.width)).toBeLessThan(56);
  expect(detailAfter!.height).toBeGreaterThan(detailBefore!.height);

  await page.mouse.move(gripBox!.x + 140, gripBox!.y + 110);
  await page.mouse.down();
  await page.mouse.move(after!.x + 10, after!.y + 10, { steps: 8 });
  await page.mouse.up();
  const clamped = await panel.boundingBox();
  expect(clamped!.width).toBeGreaterThanOrEqual(320);
  expect(clamped!.height).toBeGreaterThanOrEqual(220);
});

test("context usage stays docked and clamped in a narrow window", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 640 });
  await enterApp(page);
  const { panel } = await openContextUsagePanel(page);
  await expect(panel).toHaveAttribute("data-mode", "docked");
  const docked = await panel.boundingBox();
  expect(docked!.height).toBeLessThanOrEqual(640 * 0.46 + 4);

  const head = page.getByTestId("context-usage-head");
  const start = await head.boundingBox();
  await page.mouse.move(start!.x + 30, start!.y + start!.height / 2);
  await page.mouse.down();
  await page.mouse.move(20, 20, { steps: 8 });
  await page.mouse.up();
  await expect.poll(() => panel.getAttribute("data-mode")).toBe("floating");
  const floated = await panel.boundingBox();
  expect(floated!.x).toBeGreaterThanOrEqual(0);
  expect(floated!.y).toBeGreaterThanOrEqual(0);
  expect(floated!.x + floated!.width).toBeLessThanOrEqual(900);
  expect(floated!.y + floated!.height).toBeLessThanOrEqual(640);
});

test("artifact type badges stay neutral instead of rainbow pills", async ({ page }) => {
  await enterApp(page);
  await page
    .locator("#composer-input")
    .fill("show `figures/panel_I_heatmap_4genes_median.png/.pdf`");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();

  const badge = page.locator('.rp-tile[data-artifact-name="panel_I_heatmap_4genes_median.png"] .rp-badge');
  await expect(badge).toHaveText("image");
  await expect(badge).toHaveCSS("border-radius", "8px");
  // Neutral label: muted text on elevated surface, not a saturated type color.
  const color = await badge.evaluate((el) => getComputedStyle(el).color);
  expect(color).not.toMatch(/hsl\(\s*160/i);
});

test("Cmd+K opens search and the composer shows the macOS shortcut", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "userAgent", {
      configurable: true,
      value: "wisp-science/Tauri",
    });
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: "MacIntel",
    });
  });
  await enterApp(page);
  await expect(composer(page)).toHaveAttribute("placeholder", /Cmd\+K/);
  const shortcuts = page.locator(".sidebar .side-shortcut");
  await expect(shortcuts.nth(0)).toHaveText("⌘N");
  await expect(shortcuts.nth(1)).toHaveText("⌘K");
  await page.keyboard.press("Meta+k");
  await expect(commandPalette(page)).toBeVisible();
  await page.keyboard.press("Meta+p");
  const actionPalette = page.getByRole("dialog", { name: "Command Palette" });
  await expect(actionPalette.locator(".action-palette-row", { hasText: "New session" })
    .locator(".action-shortcut")).toHaveText("⌘N");
  await expect(actionPalette.locator(".action-palette-row", { hasText: "Search" })
    .locator(".action-shortcut")).toHaveText("⌘K");
  await expect(actionPalette.locator(".action-palette-row", { hasText: "Privacy mode" })
    .locator(".action-shortcut")).toHaveText("⌘⇧H");
});

test("Cmd+Enter sends when the modifier shortcut is selected on macOS", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "userAgent", {
      configurable: true,
      value: "wisp-science/Tauri",
    });
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: "MacIntel",
    });
    localStorage.setItem("wisp-send-with-modifier", "1");
  });
  await enterApp(page);
  await expect(page.locator(".composer-hint")).toContainText("Cmd+Enter to send · Enter for newline");
  await composer(page).fill("mac shortcut");
  await composer(page).press("Meta+Enter");
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    message: "mac shortcut",
  });
});

test("Ctrl+P command palette runs commands and switches themes", async ({ page }) => {
  await pinNonMacPlatform(page);
  await enterApp(page);
  await page.keyboard.press("Control+p");
  const palette = page.getByRole("dialog", { name: "Command Palette" });
  const input = page.locator("#action-palette-input");
  await expect(palette).toBeVisible();
  await expect(input).toBeFocused();
  await expect(input).toHaveAttribute("type", "text");
  await expect(input).toHaveAttribute("inputmode", "search");
  await expect(input).toHaveAttribute("autocomplete", "off");
  await expect(palette).toContainText("New session");
  await expect(palette.locator(".action-palette-row", { hasText: "New session" })
    .locator(".action-shortcut")).toHaveText("Ctrl+N");
  await expect(palette.locator(".action-palette-row", { hasText: "Search" })
    .locator(".action-shortcut")).toHaveText("Ctrl+K");
  await expect(palette).toContainText("Import and export");
  await expect(palette).toContainText("Import session archive");

  const rows = palette.locator(".project-search-row");
  await expect(rows.first()).toHaveClass(/active/);
  await input.press("ArrowDown");
  await expect(rows.nth(1)).toHaveClass(/active/);
  await expect(rows.nth(1)).toBeInViewport();
  // Arrow past the fold must keep the active row visible (same as Ctrl+K).
  for (let i = 0; i < 12; i++) await input.press("ArrowDown");
  await expect(palette.locator(".project-search-row.active")).toBeInViewport();
  await input.press("ArrowUp");
  await expect(palette.locator(".project-search-row.active")).toBeInViewport();

  // Typing must keep focus in the input; otherwise arrow keys hit the page behind.
  await input.fill("d");
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("d");
  await page.keyboard.press("ArrowDown");
  await expect(input).toBeFocused();
  await expect(palette.locator(".project-search-row.active")).toBeVisible();
  await input.fill("");

  // Filter by command name so inserting a new action does not change which row Enter runs.
  await input.fill("search projects");
  await expect(palette.locator(".project-search-row.active")).toContainText(
    "Search projects, artifacts, and sessions",
  );
  await input.press("Enter");
  await expect(page.getByPlaceholder("Search conversations, projects, or files…")).toBeVisible();
  await page.keyboard.press("Escape");

  await page.keyboard.press("Control+k");
  const search = commandPalette(page);
  await expect(search).toBeVisible();
  await search.pressSequentially("c");
  await expect(search).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(search).toBeFocused();
  await page.keyboard.press("Escape");

  await page.keyboard.press("Control+p");
  await input.fill("restore archive");
  await expect(palette.locator(".action-palette-row.active")).toContainText("Import session archive");
  await input.press("Enter");
  await expect.poll(() => lastInvokeArgs(page, "import_session_archive")).not.toBeNull();

  await page.keyboard.press("Control+p");
  await input.fill("definitely-not-a-command");
  await expect(page.getByTestId("action-palette-empty")).toHaveText("No matching commands");
  await input.fill("dark theme");
  await input.press("Enter");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect.poll(() => page.evaluate(() => localStorage.getItem("wisp-theme"))).toBe("dark");

  await page.keyboard.press("Control+p");
  await input.fill("open files");
  await input.press("Enter");
  await expect(page.locator(".rp-files")).toBeVisible();

  await page.keyboard.press("Control+p");
  await input.fill("system theme");
  await input.press("Enter");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "system");

  await page.keyboard.press("Control+b");
  await expect(page.locator(".sidebar")).toHaveClass(/collapsed/);
  await page.keyboard.press("Control+,");
  await expect(page.locator(".settings-page")).toBeVisible();
  await page.keyboard.press("Escape");
  const before = await page.evaluate(() => ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "new_session").length);
  await page.keyboard.press("Control+n");
  await expect.poll(() => page.evaluate(() => ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "new_session").length)).toBeGreaterThan(before);
});

test("privacy mode hides selected projects and recent sessions, then restores them", async ({ page }) => {
  await pinNonMacPlatform(page);
  await page.goto("/");
  const privateProject = page.locator(".proj-card", { hasText: "wisp-science" });
  const otherProject = page.locator(".proj-card", { hasText: "Other project" });
  await expect(privateProject).toBeVisible();
  await expect(page.getByTestId("recent-session-card")).toHaveCount(2);

  await page.keyboard.press("Control+p");
  const palette = page.getByRole("dialog", { name: "Command Palette" });
  await page.locator("#action-palette-input").fill("privacy mode");
  const privacyAction = palette.locator(".action-palette-row", { hasText: "Privacy mode" });
  await expect(privacyAction.locator(".action-shortcut")).toHaveText("Ctrl+Shift+H");
  await privacyAction.click();

  const modal = page.getByRole("dialog", { name: "Privacy mode" });
  await expect(modal).toBeVisible();
  // Root-owned Escape must work before focus moves into the modal.
  await page.keyboard.press("Escape");
  await expect(modal).toBeHidden();

  await page.keyboard.press("Control+Shift+h");
  await expect(modal).toBeVisible();
  const projectRow = modal.locator(".privacy-project-row", { hasText: "wisp-science" });
  await expect(projectRow).toHaveCSS("flex-direction", "row");
  await projectRow.locator('input[type="checkbox"]').check();
  await modal.getByRole("button", { name: "Hide selected" }).click();

  await expect(privateProject).toBeHidden();
  await expect(otherProject).toBeVisible();
  await expect(page.getByTestId("recent-session-card")).toHaveCount(0);
  await expect(page.locator(".privacy-mode-banner")).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => localStorage.getItem("wisp-privacy-mode-active"))).toBe("1");

  await page.reload();
  await expect(otherProject).toBeVisible();
  await expect(privateProject).toBeHidden();
  await expect(page.locator(".privacy-mode-banner")).toHaveCount(0);
  await expect(page.getByText("Privacy mode")).toHaveCount(0);
  await expect(page.getByTestId("recent-session-card")).toHaveCount(0);
  await page.keyboard.press("Control+k");
  const search = page.getByRole("dialog", { name: "Search" });
  await expect(search).not.toContainText("wisp-science");
  await expect(search).not.toContainText("Enumerate MCP bio-tools databases");
  await page.keyboard.press("Escape");

  await page.keyboard.press("Control+Shift+h");
  await expect(modal.locator(".privacy-project-row", { hasText: "wisp-science" })
    .locator('input[type="checkbox"]')).toBeChecked();
  await modal.getByRole("button", { name: "Restore all" }).click();
  await expect(privateProject).toBeVisible();
  await expect(page.getByTestId("recent-session-card")).toHaveCount(2);
  await expect.poll(() => page.evaluate(() => localStorage.getItem("wisp-privacy-mode-active"))).toBeNull();
});

test("privacy mode select all toggles every project at once", async ({ page }) => {
  await page.goto("/");
  const privateProject = page.locator(".proj-card", { hasText: "wisp-science" });
  const otherProject = page.locator(".proj-card", { hasText: "Other project" });
  await expect(privateProject).toBeVisible();
  await expect(otherProject).toBeVisible();

  await page.keyboard.press("Control+Shift+h");
  const modal = page.getByRole("dialog", { name: "Privacy mode" });
  await expect(modal).toBeVisible();

  const selectAll = modal.getByTestId("privacy-select-all");
  const projectBoxes = modal.locator('.privacy-project-row:not(.privacy-project-row-all) input[type="checkbox"]');
  await expect(projectBoxes).toHaveCount(2);
  await expect(selectAll).not.toBeChecked();

  await selectAll.check();
  await expect(selectAll).toBeChecked();
  for (const box of await projectBoxes.all()) {
    await expect(box).toBeChecked();
  }

  // Unchecking one project clears the select-all state.
  await projectBoxes.first().uncheck();
  await expect(selectAll).not.toBeChecked();
  await selectAll.check();
  await expect(selectAll).toBeChecked();

  await modal.getByRole("button", { name: "Hide selected" }).click();
  await expect(privateProject).toBeHidden();
  await expect(otherProject).toBeHidden();
  await expect(page.getByTestId("recent-session-card")).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => localStorage.getItem("wisp-privacy-mode-active"))).toBe("1");

  await page.keyboard.press("Control+Shift+h");
  await expect(selectAll).toBeChecked();
  await selectAll.uncheck();
  for (const box of await projectBoxes.all()) {
    await expect(box).not.toBeChecked();
  }
  await expect(modal.getByRole("button", { name: "Hide selected" })).toBeDisabled();

  await modal.getByRole("button", { name: "Restore all" }).click();
  await expect(privateProject).toBeVisible();
  await expect(otherProject).toBeVisible();
  await expect(page.getByTestId("recent-session-card")).toHaveCount(2);
});

test("Ctrl+P changes UI and code font sizes", async ({ page }) => {
  await enterApp(page);
  const input = page.locator("#action-palette-input");
  const storedSize = (key: string) => page.evaluate((name) => localStorage.getItem(name), key);

  await page.keyboard.press("Control+p");
  await input.fill("font");
  await expect(page.locator(".action-palette-row")).toHaveCount(4);

  for (const [command, key, expected] of [
    ["increase ui size", "wisp-ui-font-size", "15"],
    ["decrease ui size", "wisp-ui-font-size", "14"],
    ["increase code size", "wisp-code-font-size", "13"],
    ["decrease code size", "wisp-code-font-size", "12"],
  ] as const) {
    await input.fill(command);
    await input.press("Enter");
    await expect.poll(() => storedSize(key)).toBe(expected);
    await page.keyboard.press("Control+p");
  }
});

test("new session focuses the composer", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await expect(composer(page)).toBeFocused();
});

test("new session appears in the sidebar before its first message", async ({ page }) => {
  await enterApp(page);
  const sessions = page.locator(".side-item.ses");
  const countBefore = await sessions.count();
  const sendsBefore = (await invokeArgsList(page, "send_message")).length;

  await newSessionButton(page).click();

  await expect(sessions).toHaveCount(countBefore + 1);
  const newSession = sessions.filter({ hasText: "Untitled session" }).first();
  await expect(newSession).toBeVisible();
  await expect(newSession).toHaveClass(/active/);
  expect((await invokeArgsList(page, "send_message")).length).toBe(sendsBefore);
});

test("rename session modal autofocuses so Ctrl+A selects the title", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  await composer(page).fill("rename-me");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(assistantReplyQuoting(page, "rename-me")).toBeVisible({ timeout: 10_000 });

  await page.locator(".side-item.ses", { hasText: "rename-me" }).dblclick();
  const input = page.locator("#rename-session-input");
  await expect(input).toBeVisible();
  await expect(input).toBeFocused();
  await expect.poll(async () => input.evaluate((el: HTMLInputElement) =>
    el.selectionStart === 0 && el.selectionEnd === el.value.length && el.value.length > 0
  )).toBe(true);

  // Even after clearing selection, Ctrl+A must stay inside the field.
  await input.evaluate((el: HTMLInputElement) => el.setSelectionRange(0, 0));
  await page.keyboard.press("Control+a");
  await expect.poll(async () => input.evaluate((el: HTMLInputElement) =>
    el.selectionStart === 0 && el.selectionEnd === el.value.length
  )).toBe(true);
});

test("renaming a fresh session takes effect before its first message (#888)", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  await newSessionButton(page).click();
  const draft = page.locator(".side-item.ses", { hasText: "Untitled session" }).first();
  await expect(draft).toBeVisible();

  await draft.dblclick();
  const input = page.locator("#rename-session-input");
  await expect(input).toBeVisible();
  await input.fill("Named before first turn");
  await page.locator(".modal", { has: input }).getByRole("button", { name: "Save" }).click();

  // The rename must show up without sending any message first.
  await expect(page.locator(".side-item.ses", { hasText: "Named before first turn" })).toBeVisible();
  await expect(page.locator(".side-item.ses", { hasText: "Untitled session" })).toHaveCount(0);
  expect((await invokeArgsList(page, "send_message")).length).toBe(0);
});

test("conversation action button renames, transfers, and deletes sessions (#557)", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  await page.getByRole("button", { name: "New group" }).click();
  const folderInput = page.locator("#folder-modal-input");
  await folderInput.fill("Results");
  await page.locator(".modal", { has: folderInput }).getByRole("button", { name: "Save" }).click();
  await expect(page.locator(".side-folder", { hasText: "Results" })).toBeVisible();

  await composer(page).fill("actions-manage-me");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(assistantReplyQuoting(page, "actions-manage-me")).toBeVisible({ timeout: 10_000 });
  let session = page.locator(".side-item.ses", { hasText: "actions-manage-me" });
  await expect(session).toBeVisible({ timeout: 10_000 });

  const openActions = async () => {
    const row = session.locator("..");
    const actions = row.getByRole("button", { name: "Conversation actions" });
    // The menu button is hover/focus-revealed: rest at opacity 0.
    await row.hover();
    await expect.poll(() => actions.evaluate((el) => Number.parseFloat(getComputedStyle(el).opacity))).toBeGreaterThan(0);
    await actions.click();
  };

  await openActions();
  await expect.poll(() => page.locator(".ctx-menu").evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight;
  })).toBe(true);
  await expect(page.getByRole("button", { name: "Rename", exact: true })).toBeVisible();
  const moveTo = page.getByRole("button", { name: "Move to", exact: true });
  await expect(moveTo).toBeVisible();
  await expect(page.getByRole("button", { name: /^Move to:/ })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Copy to another project…", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Move to another project…", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Delete", exact: true })).toBeVisible();

  await moveTo.hover();
  const moveSubmenu = page.locator(".ctx-submenu-menu");
  await expect(moveSubmenu).toBeVisible();
  await expect(moveSubmenu.getByRole("button", { name: "Ungrouped", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(moveSubmenu).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Rename", exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Rename", exact: true }).hover();
  await moveTo.hover();
  await expect(moveSubmenu).toBeVisible();
  await moveSubmenu.getByRole("button", { name: "Results", exact: true }).click();
  await expect.poll(() => page.evaluate(() => {
    const calls = ((window as any).__sendInvokeLog ?? []).filter((call: any) => call.cmd === "move_session");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : args;
  })).toMatchObject({ folderId: "folder-1" });

  await openActions();
  await page.getByRole("button", { name: "Rename", exact: true }).click();
  const renameInput = page.locator("#rename-session-input");
  await renameInput.fill("Managed analysis");
  await page.locator(".modal", { has: renameInput }).getByRole("button", { name: "Save" }).click();
  session = page.locator(".side-item.ses", { hasText: "Managed analysis" });
  await expect(session).toBeVisible();

  await openActions();
  await page.getByRole("button", { name: "Copy to another project…", exact: true }).click();
  let transfer = page.locator(".session-transfer-modal");
  await expect(transfer.locator("select")).toHaveValue("other");
  await transfer.getByRole("button", { name: "Copy", exact: true }).click();
  await expect.poll(() => page.evaluate(() => {
    const calls = ((window as any).__sendInvokeLog ?? []).filter((call: any) => call.cmd === "transfer_session_to_project");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : args;
  })).toMatchObject({ targetProjectId: "other", mode: "copy" });
  await expect(session).toBeVisible();

  await openActions();
  await page.getByRole("button", { name: "Move to another project…", exact: true }).click();
  transfer = page.locator(".session-transfer-modal");
  await transfer.getByLabel("Target project").selectOption("archive");
  await expect(transfer.getByLabel("Target project")).toHaveValue("archive");
  await transfer.getByRole("button", { name: "Move", exact: true }).click();
  await expect.poll(() => page.evaluate(() => {
    const calls = ((window as any).__sendInvokeLog ?? []).filter((call: any) => call.cmd === "transfer_session_to_project");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : args;
  })).toMatchObject({ targetProjectId: "archive", mode: "move" });
  await expect(session).toHaveCount(0);

  await newSessionButton(page).click();
  await composer(page).fill("actions-delete-me");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(assistantReplyQuoting(page, "actions-delete-me")).toBeVisible({ timeout: 10_000 });
  session = page.locator(".side-item.ses", { hasText: "actions-delete-me" });
  await expect(session).toBeVisible({ timeout: 10_000 });
  await openActions();
  await page.getByRole("button", { name: "Delete", exact: true }).click();
  await page.locator(".confirm-modal").getByRole("button", { name: "Delete", exact: true }).click();
  await expect(session).toHaveCount(0);
  await expect.poll(() => page.evaluate(() =>
    ((window as any).__sendInvokeLog ?? []).some((call: any) => call.cmd === "delete_session")
  )).toBe(true);
});

test("stale project rules can be reloaded from the session context menu", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await enterApp(page, "/?mockStaleRules=1");
  const stale = page.locator(".side-item.ses", { hasText: "Outdated rules chat" });
  await expect(stale).toBeVisible({ timeout: 10_000 });
  await expect(stale).toHaveAttribute("data-session-stale", "true");
  await expect(stale.locator(".ses-stale")).toHaveCount(0);

  // A fresh session has no reload menu item.
  await newSessionButton(page).click();
  await composer(page).fill("fresh rules chat");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(assistantReplyQuoting(page, "fresh rules chat")).toBeVisible({ timeout: 10_000 });
  const fresh = page.locator(".side-item.ses", { hasText: "fresh rules chat" });
  await expect(fresh).toBeVisible({ timeout: 10_000 });
  await fresh.click({ button: "right" });
  await expect(page.locator(".ctx-menu")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Reload project rules…", exact: true }),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.locator(".ctx-menu")).toHaveCount(0);

  // The stale session offers the reload item; Escape closes only the confirm.
  await stale.click({ button: "right" });
  await page.getByRole("button", { name: "Reload project rules…", exact: true }).click();
  const modal = page.locator(".confirm-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("prompt cache");
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  await expect.poll(() => page.evaluate(() =>
    ((window as any).__sendInvokeLog ?? []).some((call: any) => call.cmd === "reload_project_rules")
  )).toBe(false);

  await stale.click({ button: "right" });
  await page.getByRole("button", { name: "Reload project rules…", exact: true }).click();
  await page
    .locator(".confirm-modal")
    .getByRole("button", { name: "Reload rules", exact: true })
    .click();
  await expect.poll(() => page.evaluate(() => {
    const calls = ((window as any).__sendInvokeLog ?? []).filter((call: any) => call.cmd === "reload_project_rules");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : args;
  })).toMatchObject({
    frameId: "stale-session",
  });
  await expect(page.locator(".copy-toast")).toHaveCount(0);
  await expect(stale).toHaveAttribute("data-session-stale", "false");
});

test("session context menu near the window bottom stays fully visible (#650)", async ({ page }) => {
  // Narrow + short window: labels wrap, so real item heights exceed the 38px
  // estimate the initial placement uses — the menu must re-clamp after measuring.
  await page.setViewportSize({ width: 180, height: 420 });
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  await composer(page).fill("ctx-bottom-edge");
  await page.getByRole("button", { name: "Send" }).click();
  const session = page.locator(".side-item.ses", { hasText: "ctx-bottom-edge" });
  await expect(session).toBeVisible({ timeout: 10_000 });

  // Right-click with the pointer at the very bottom edge of the window.
  await session.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    el.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      button: 2,
      clientX: Math.round(rect.left + 20),
      clientY: window.innerHeight - 12,
    }));
  });

  const menu = page.locator(".ctx-menu");
  await expect(menu).toBeVisible();
  await expect.poll(() => menu.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight;
  })).toBe(true);
  // The last (bottom-most) menu item must remain clickable.
  await expect(menu.getByRole("button", { name: "Delete", exact: true })).toBeVisible();
});

test("conversations can be selected and moved or deleted together", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();

  await page.getByRole("button", { name: "New group" }).click();
  const folderInput = page.locator("#folder-modal-input");
  await folderInput.fill("Bulk destination");
  await page.locator(".modal", { has: folderInput }).getByRole("button", { name: "Save" }).click();

  const titles = ["actions-bulk-one", "actions-bulk-two", "actions-bulk-keep"];
  for (const title of titles) {
    await newSessionButton(page).click();
    await composer(page).fill(title);
    await page.getByRole("button", { name: "Send" }).click();
    await expect(page.locator(".side-item.ses", { hasText: title })).toBeVisible({ timeout: 10_000 });
  }

  const sidebar = page.locator(".sidebar");
  let first = sidebar.locator(".side-item.ses", { hasText: titles[0] });
  let second = sidebar.locator(".side-item.ses", { hasText: titles[1] });
  const keep = sidebar.locator(".side-item.ses", { hasText: titles[2] });
  const firstId = await first.getAttribute("data-session-id");
  const secondId = await second.getAttribute("data-session-id");

  await sidebar.getByRole("button", { name: "Select", exact: true }).click();
  await first.click();
  await second.click();
  await expect(first).toHaveAttribute("aria-pressed", "true");
  await expect(second).toHaveAttribute("aria-pressed", "true");
  await expect(sidebar.getByText("2 selected", { exact: true })).toBeVisible();

  await sidebar.getByTestId("bulk-move-sessions").selectOption("folder-1");
  await expect.poll(() => page.evaluate(({ firstId, secondId }) => {
    const calls = ((window as any).__sendInvokeLog ?? [])
      .filter((call: any) => call.cmd === "move_session")
      .slice(-2)
      .map((call: any) => call.args instanceof Map ? Object.fromEntries(call.args) : call.args)
      .sort((a: any, b: any) => String(a.id).localeCompare(String(b.id)));
    return calls;
  }, { firstId, secondId })).toEqual([
    { id: firstId, folderId: "folder-1" },
    { id: secondId, folderId: "folder-1" },
  ].sort((a, b) => String(a.id).localeCompare(String(b.id))));
  await expect(sidebar.getByRole("button", { name: "Select", exact: true })).toBeVisible();

  first = sidebar.locator(".side-item.ses", { hasText: titles[0] });
  second = sidebar.locator(".side-item.ses", { hasText: titles[1] });
  await sidebar.getByRole("button", { name: "Select", exact: true }).click();
  await first.click();
  await second.click();
  await sidebar.getByTestId("bulk-delete-sessions").click();

  const confirm = page.locator(".confirm-modal");
  await expect(confirm).toContainText("Delete 2 conversations? This cannot be undone.");
  await page.keyboard.press("Escape");
  await expect(confirm).toHaveCount(0);
  await expect(sidebar.getByTestId("bulk-delete-sessions")).toBeVisible();
  await expect(first).toHaveAttribute("aria-pressed", "true");

  await sidebar.getByTestId("bulk-delete-sessions").click();
  await confirm.getByRole("button", { name: "Delete", exact: true }).click();
  await expect(first).toHaveCount(0);
  await expect(second).toHaveCount(0);
  await expect(keep).toBeVisible();
  await expect.poll(() => page.evaluate(({ firstId, secondId }) => {
    const ids = ((window as any).__sendInvokeLog ?? [])
      .filter((call: any) => call.cmd === "delete_session")
      .map((call: any) => call.args instanceof Map ? Object.fromEntries(call.args) : call.args)
      .map((args: any) => args.id);
    return [firstId, secondId].every((id) => ids.includes(id));
  }, { firstId, secondId })).toBe(true);
});

test("group action button visibly renames and deletes groups", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();

  await page.getByRole("button", { name: "New group" }).click();
  const folderInput = page.locator("#folder-modal-input");
  await folderInput.fill("Figures");
  await page.locator(".modal", { has: folderInput }).getByRole("button", { name: "Save" }).click();

  let folder = page.locator(".side-folder", { hasText: "Figures" });
  await expect(folder).toBeVisible();
  let actions = folder.getByRole("button", { name: "Group actions" });
  // The group menu button is hover/focus-revealed: rest at opacity 0.
  await folder.hover();
  await expect.poll(() => actions.evaluate((el) => Number.parseFloat(getComputedStyle(el).opacity))).toBeGreaterThan(0);
  await actions.click();
  await page.getByRole("button", { name: "Rename group" }).click();
  await folderInput.fill("Results");
  await page.locator(".modal", { has: folderInput }).getByRole("button", { name: "Save" }).click();

  folder = page.locator(".side-folder", { hasText: "Results" });
  await expect(folder).toBeVisible();
  actions = folder.getByRole("button", { name: "Group actions" });
  await folder.hover();
  await actions.click();
  await page.getByRole("button", { name: "Delete group" }).click();
  await page.locator(".confirm-modal").getByRole("button", { name: "Delete group", exact: true }).click();
  await expect(folder).toHaveCount(0);
});

test("user message renders before a delayed backend User event", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("DELAYUSER");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.locator(".user-bubble .body", { hasText: /^DELAYUSER$/ })).toBeVisible({ timeout: 500 });
  await expect(page.getByText("delayed reply")).toBeVisible({ timeout: 3_000 });
  await expect(page.locator(".user-bubble .body", { hasText: /^DELAYUSER$/ })).toHaveCount(1);
  await expect(page.locator(".msg.user .user-message-time")).toBeVisible();
  await expect(page.locator(".msg.assistant .assistant-message-time")).toBeVisible();
});

test("long unbroken user text wraps inside the chat column", async ({ page }) => {
  await enterApp(page);
  const seq = `${"MVGCHEQEAPSETTASSSSFERELVTGSSCVIDADANYSEMAVSDTAAGLTAPTARQRVSDEGKKPGPSSQHRPSPDRNYSQAVSENLQAVTSSSSEHRGISRIVQQQQPGQPFHRRHTTGATSPAMGTAEAAAVAAAASSSSAEEAALDVDCVEGHDEGLHSGREIPRCGLDNLDSSPDCGRHDASQGNSRHTCKVCKRPFSSGRALGGHMRAHGNGDPGTSSNADRKSEKQLISSSPRTQQASLHACNGVAENGIEHPGADGVARAQSLSPESRARARTREIQVRRAVGARRSKTNGKRRGSTTPKSSVEDAAALTKQQPHDEDDNAASRRQAERSSTSCSDNNSDGAHDDGAATDDAAGNICDVCREEFENEKQLNTHKKSHKPEYNLRECPRKSRRFIDQDYTEVAPPTIPTKKPPAPQEKQQSDSGCPYPGCTKKFHSSKALFGHMRCHPDRTWRGIHPPDENGASTSAAGERQHRRKKSRPNSHVPARVVSDSESEPEQKQSGKSASTEHESDTDSIEAAYIQGQEAHTNGDRQQSSTPGWWASGVTGKRSKRSRQTVRSLQAVHHGASTSSAAAPDNALEELNETAMVMMMLASNPSGAPKHEDPDEHMEDLFRNPNSADECPKDEPTEGCLEAALRAKDEEEDEEDEEEDKEEEGEDGDEKQGAAAATAAEVVEDLEQGPELVPKDEFMTAAAETAEVPMEVDEEPEASLSEDGVLQGEEAVQLEAGQQEASSSKHGQALGGHKRCHFDPTKKDAEKEGSSSNNGGKNPRSSNPAGRASYSQSRGRHESSDARGHSPRAKSDPGLQQQQQQQAAAPAESRSTGLLRPIEIDLNKPPTVTYDEEMEMAPSPASAKFSVENHEAQASASAEASSSPDDGEPMRNQPRDYQLILHLSPITLNLEDQLHAYYKRVTPA".repeat(2)} find homolog`;
  await composer(page).fill(seq);
  await page.getByRole("button", { name: "Send" }).click();
  const bubble = page.locator(".msg.user .body").first();
  await expect(bubble).toBeVisible({ timeout: 10_000 });
  const { bubbleWidth, threadWidth, scrollWidth, clientWidth } = await page.evaluate(() => {
    const body = document.querySelector(".msg.user .body") as HTMLElement | null;
    const thread = document.querySelector(".thread") as HTMLElement | null;
    const chat = document.querySelector(".chat") as HTMLElement | null;
    return {
      bubbleWidth: body?.getBoundingClientRect().width ?? 0,
      threadWidth: thread?.getBoundingClientRect().width ?? 0,
      scrollWidth: chat?.scrollWidth ?? 0,
      clientWidth: chat?.clientWidth ?? 0,
    };
  });
  expect(bubbleWidth).toBeGreaterThan(0);
  expect(bubbleWidth).toBeLessThanOrEqual(threadWidth + 1);
  // Column must not grow a horizontal scrollbar from the unbroken sequence.
  expect(scrollWidth).toBeLessThanOrEqual(clientWidth + 1);
});

test("side chat answers in a temporary side panel and can switch model", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("what did the main thread miss?");
  await page.getByRole("button", { name: "Message options" }).click();
  await page.getByRole("button", { name: "Side chat" }).click();

  const panel = page.locator(".rightpane");
  await expect(panel).toBeVisible();
  await expect(panel.locator(".sidechat-in-pane")).toBeVisible();
  await expect(panel.getByText("Side answer: what did the main thread miss?")).toBeVisible();
  const evidence = panel.getByTestId("sidechat-evidence");
  await expect(evidence).toContainText("1 conversation sources · snapshot 12");
  await evidence.locator("summary").click();
  await expect(evidence).toContainText("[S1] · Turn 2 · assistant · event 7");
  await expect(evidence).toContainText("The main thread recorded this evidence.");
  await expect(panel).not.toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  const closeBox = await panel.getByRole("button", { name: "Close tab" }).first().boundingBox();
  const panelBox = await panel.boundingBox();
  expect(closeBox && panelBox && closeBox.x + closeBox.width <= panelBox.x + panelBox.width).toBeTruthy();
  await expect.poll(() => lastInvokeArgs(page, "side_chat")).toMatchObject({
    question: "what did the main thread miss?",
  });
  await expect.poll(async () => {
    const args = await lastInvokeArgs(page, "send_message");
    return args?.message ?? null;
  }).toBeNull();

  await panel.getByRole("button", { name: /deepseek-v4-pro/ }).click();
  await panel.getByRole("button", { name: "opus-4.8" }).click();
  await expect(panel.getByRole("button", { name: /opus-4.8/ })).toBeVisible();

  // Side chat can route through an ACP Agent (#250).
  await panel.getByRole("button", { name: /opus-4.8/ }).click();
  await panel.getByRole("button", { name: "Test ACP Agent" }).click();
  await expect(panel.getByRole("button", { name: /Test ACP Agent/ })).toBeVisible();
  await panel.getByPlaceholder("Follow up…").fill("acp side question");
  await panel.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "side_chat")).toMatchObject({
    question: "acp side question", acpAgentId: "acp-test",
  });
});

test("side chat reports when the frozen conversation has no evidence", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("NO_EVIDENCE_TEST");
  await page.getByRole("button", { name: "Message options" }).click();
  await page.getByRole("button", { name: "Side chat" }).click();

  const panel = page.locator(".rightpane");
  await expect(panel.getByText(
    "The current conversation does not contain enough evidence to answer that.",
  )).toBeVisible();
  await expect(panel.getByTestId("sidechat-evidence")).toHaveCount(0);
  await expect.poll(async () => {
    const args = await lastInvokeArgs(page, "send_message");
    return args?.message ?? null;
  }).toBeNull();
});

test("side chat stays at the latest message after sending and switching tabs", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("SIDESCROLLTEST");
  await page.getByRole("button", { name: "Message options" }).click();
  await page.getByRole("button", { name: "Side chat" }).click();

  const panel = page.locator(".rightpane");
  const log = panel.locator(".sidechat-log");
  await expect(panel.getByText("Side answer line 40")).toBeVisible();
  await expect.poll(() => log.evaluate((element) =>
    element.scrollHeight > element.clientHeight,
  )).toBe(true);
  const bottomGap = () => log.evaluate((element) =>
    element.scrollHeight - element.clientHeight - element.scrollTop,
  );
  await expect.poll(bottomGap).toBeLessThan(8);

  await panel.locator(".rp-tab", { hasText: "Artifacts" }).click();
  await panel.getByRole("button", { name: "Side chat", exact: true }).click();
  await expect.poll(bottomGap).toBeLessThan(8);

  await panel.getByPlaceholder("Follow up…").fill("latest side follow-up");
  await panel.getByRole("button", { name: "Send" }).click();
  await expect(panel.getByText("Side answer: latest side follow-up")).toBeVisible();
  await expect.poll(bottomGap).toBeLessThan(8);
});

test("clicking a PNG path opens the image preview without the selection popup", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("CLIPBOARDIMAGE");
  await page.getByRole("button", { name: "Send" }).click();

  const reply = page.locator(".msg.assistant", { hasText: "Saved the screenshots as local files" });
  await expect(reply).toBeVisible({ timeout: 10_000 });
  const pathLink = reply.locator("a", { hasText: "clipboard-preview.png" }).first();
  await expect(pathLink).toBeVisible();

  // Clicking a long Windows path often selects the link label. mouseup used to
  // treat that leftover selection as a quote and stack the popup on the preview.
  await pathLink.evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    el.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
  });
  await expect(page.locator(".selection-popup")).toHaveCount(0);

  await pathLink.click();
  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".am-name")).toHaveText("clipboard-preview.png");
  await expect(page.locator(".selection-popup")).toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  await expect(reply).toBeVisible();
});

test("transcript selections add to the main composer without closing the right pane", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();
  const panel = page.locator(".rightpane");
  await expect(panel).toBeVisible();

  const selected = await selectAssistantReplyText(page);
  expect(selected.length).toBeGreaterThan(20);
  const popup = page.locator(".selection-popup");
  await expect(popup.getByRole("button", { name: "Add to chat" })).toBeVisible();
  await expect(popup.getByRole("button", { name: "Ask AI in the conversation" })).toHaveCount(0);
  await expect(popup.getByRole("button", { name: "Quote in side chat" })).toBeVisible();
  await popup.getByRole("button", { name: "Add to chat" }).click();
  await expect(page.locator(".composer-reference-chips .quote").last())
    .toContainText(selected.slice(0, 30));
  await expect(panel).toBeVisible();

  // The native-style context menu mirrors the popup and must use the same
  // explicit source check instead of treating an absent source as a center file.
  await selectAssistantReplyText(page, "contextmenu");
  const menu = page.locator(".ctx-menu");
  await expect(menu.getByRole("button", { name: "Add to chat" })).toBeVisible();
  await expect(menu.getByRole("button", { name: "Ask AI in the conversation" })).toHaveCount(0);
  await expect(menu.getByRole("button", { name: "Quote in side chat" })).toBeVisible();
  await menu.getByRole("button", { name: "Add to chat" }).click();
  await expect(page.locator(".composer-reference-chips .quote")).toHaveCount(2);
  await expect(panel).toBeVisible();
});

test("literature research prepares a skill-backed turn in the current conversation", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });

  const selected = await selectAssistantReplyText(page);
  const popup = page.locator(".selection-popup");
  const action = popup.getByRole("button", { name: "Research literature" });
  await expect(action).toBeVisible();
  await action.click();

  await expect.poll(() => lastInvokeArgs(page, "run_quick_action")).toBeNull();
  await expect(page.locator(".selection-popup")).toHaveCount(0);
  await expect(page.getByText("Added Research literature to this conversation"))
    .toBeVisible();
  await expect(composer(page)).toHaveValue(/Research the quoted passage/);
  await expect(page.locator(".composer-reference-chips .skill"))
    .toContainText("literature-review");
  await expect(page.locator(".composer-reference-chips .quote"))
    .toContainText(selected.slice(0, 30));

  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    sessionId: expect.any(String),
    message: expect.stringContaining(selected.slice(0, 30)),
    references: [{ kind: "skill", name: "literature-review" }],
  });
  await expect(page.locator(".msg.user").last()).toContainText("Research the quoted passage");
});

test("literature research from the right-click menu also stays in the composer", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });

  const selected = await selectAssistantReplyText(page, "contextmenu");
  const menu = page.locator(".ctx-menu");
  await menu.getByRole("button", { name: "Research literature" }).click();

  await expect.poll(() => lastInvokeArgs(page, "run_quick_action")).toBeNull();
  await expect(page.locator(".ctx-menu")).toHaveCount(0);
  await expect(composer(page)).toHaveValue(/Research the quoted passage/);
  await expect(page.locator(".composer-reference-chips .skill"))
    .toContainText("literature-review");
  await expect(page.locator(".composer-reference-chips .quote"))
    .toContainText(selected.slice(0, 30));
});

test("Quick Actions opens its bound graph in the standalone Workflow Studio", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Quick Actions");

  const row = page.getByTestId("quick-action-row")
    .filter({ hasText: "Research literature" });
  await expect(row).toBeVisible();
  await expect(row).toContainText("Literature evidence review");
  await row.getByTestId("quick-action-open-workflow").click();

  await expect(page.locator(".settings-page")).toHaveClass(/workflow-studio-mode/);
  await expect(page.locator(".settings-nav")).toBeHidden();
  const studio = page.getByTestId("workflow-studio");
  await expect(studio).toBeVisible();
  const libraryLayout = await studio.locator(".workflow-studio-library").evaluate((library) => {
    const bounds = library.getBoundingClientRect();
    const buttons = [...library.querySelectorAll<HTMLElement>(".workflow-studio-library-actions button")]
      .map((button) => button.getBoundingClientRect());
    return {
      inside: buttons.every((button) => button.left >= bounds.left
        && button.right <= bounds.right),
      stacked: buttons.length === 2 && buttons[1].top > buttons[0].bottom,
    };
  });
  expect(libraryLayout).toEqual({ inside: true, stacked: true });
  const studioBox = await studio.boundingBox();
  const viewport = page.viewportSize()!;
  expect(studioBox?.width ?? 0).toBeGreaterThan(viewport.width * 0.95);
  expect(studioBox?.height ?? 0).toBeGreaterThan(viewport.height * 0.85);
  await expect(studio.getByTestId("workflow-name"))
    .toHaveValue("Literature evidence review");
  const nodes = studio.getByTestId("workflow-graph-node");
  await expect(nodes).toHaveCount(3);
  await expect(nodes.nth(0)).toHaveAttribute("data-node-id", "supporting_evidence");
  await expect(nodes.nth(1)).toHaveAttribute("data-node-id", "challenging_evidence");
  await expect(nodes.nth(2)).toHaveAttribute("data-node-id", "synthesize");
  await expect(studio.getByTestId("workflow-graph-edge")).toHaveCount(2);
  const positions = await nodes.evaluateAll((items) => items.map((item) => {
    const box = item.getBoundingClientRect();
    return { id: item.getAttribute("data-node-id"), x: box.x, y: box.y };
  }));
  const supporting = positions.find((item) => item.id === "supporting_evidence")!;
  const challenging = positions.find((item) => item.id === "challenging_evidence")!;
  const synthesize = positions.find((item) => item.id === "synthesize")!;
  expect(Math.abs(supporting.x - challenging.x)).toBeLessThan(2);
  expect(supporting.y).not.toBe(challenging.y);
  expect(synthesize.x).toBeGreaterThan(supporting.x);

  await nodes.filter({ hasText: "synthesize" })
    .getByTestId("workflow-graph-node-select")
    .click();
  const inspector = studio.getByTestId("workflow-graph-inspector");
  await expect(inspector.getByTestId("dynamic-task-id")).toHaveValue("synthesize");
  await inspector.locator("details.dynamic-agent-advanced > summary").click();
  const timeout = inspector.getByTestId("dynamic-task-timeout-secs");
  await expect(timeout).toHaveAttribute("placeholder", "Empty = policy default; 0 = unlimited");
  await timeout.fill("0");
  await expect(timeout).toHaveValue("0");
  await expect(inspector.getByTestId("workflow-graph-remove-edge")).toHaveCount(2);
  const skillPicker = inspector.getByTestId("dynamic-task-skills");
  await expect(skillPicker.getByTestId("dynamic-task-skill-option")).toHaveCount(0);
  await skillPicker.getByTestId("dynamic-task-skill-search").fill("literature");
  await expect(skillPicker.getByTestId("dynamic-task-skill-option")).toHaveCount(1);
  await expect(skillPicker.getByTestId("dynamic-task-skill-option"))
    .toContainText("literature-review");

  const resizer = studio.getByTestId("workflow-graph-resizer");
  await expect(resizer).toHaveAttribute("role", "separator");
  const inspectorBeforeResize = await inspector.boundingBox();
  await resizer.evaluate((handle) => {
    const rect = handle.getBoundingClientRect();
    const startX = rect.left + rect.width / 2;
    const startY = rect.top + 60;
    handle.dispatchEvent(new PointerEvent("pointerdown", {
      bubbles: true,
      button: 0,
      pointerId: 17,
      clientX: startX,
      clientY: startY,
    }));
    handle.dispatchEvent(new PointerEvent("pointermove", {
      bubbles: true,
      buttons: 1,
      pointerId: 17,
      clientX: startX - 80,
      clientY: startY,
    }));
    handle.dispatchEvent(new PointerEvent("pointerup", {
      bubbles: true,
      button: 0,
      pointerId: 17,
      clientX: startX - 80,
      clientY: startY,
    }));
  });
  await expect.poll(async () => {
    const resized = await inspector.boundingBox();
    return inspectorBeforeResize && resized
      ? Math.round(resized.width - inspectorBeforeResize.width)
      : 0;
  }).toBeGreaterThan(60);
  await expect(studio.getByTestId("workflow-graph-minimap")).toBeVisible();
  await studio.getByTestId("workflow-graph-zoom-in").click();
  await expect(studio.getByTestId("workflow-graph-fit")).toHaveText("110%");
  await studio.getByTestId("workflow-graph-fit").click();
  await expect(studio.getByTestId("workflow-graph-fit")).toHaveText("100%");
  await expect(studio.getByTestId("workflow-save")).toHaveText("Save as copy");
  const typography = await studio.evaluate((root) => {
    const save = root.querySelector('[data-testid="workflow-save"]')!;
    const nodeId = root.querySelector(".workflow-graph-node-title strong")!;
    return {
      studio: getComputedStyle(root).fontFamily,
      save: getComputedStyle(save).fontFamily,
      saveWeight: Number(getComputedStyle(save).fontWeight),
      node: getComputedStyle(nodeId).fontFamily,
    };
  });
  expect(typography.save).toBe(typography.studio);
  expect(typography.saveWeight).toBeLessThanOrEqual(600);
  expect(typography.node).not.toBe(typography.studio);
});

test("Workflow library includes a built-in Roundtable DAG", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Workflows");
  const studio = page.getByTestId("workflow-studio");
  const roundtable = studio.getByTestId("workflow-template-card")
    .filter({ hasText: "Roundtable" });
  await expect(roundtable).toContainText("neutral chair synthesis");
  await roundtable.click();

  const nodes = studio.getByTestId("workflow-graph-node");
  await expect(nodes).toHaveCount(5);
  await expect(studio.getByTestId("workflow-graph-edge")).toHaveCount(6);
  const positions = await nodes.evaluateAll((items) => items.map((item) => {
    const box = item.getBoundingClientRect();
    return { id: item.getAttribute("data-node-id"), x: box.x };
  }));
  const opening = positions.filter((item) => item.id?.endsWith("_opening"));
  const reviews = positions.filter((item) => item.id?.endsWith("_review"));
  const chair = positions.find((item) => item.id === "chair_synthesis")!;
  expect(opening).toHaveLength(2);
  expect(reviews).toHaveLength(2);
  expect(Math.abs(opening[0].x - opening[1].x)).toBeLessThan(2);
  expect(Math.abs(reviews[0].x - reviews[1].x)).toBeLessThan(2);
  expect(reviews[0].x).toBeGreaterThan(opening[0].x);
  expect(chair.x).toBeGreaterThan(reviews[0].x);
});

test("Workflow library includes the Wisp-native seven-node method-search DAG", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Workflows");
  const studio = page.getByTestId("workflow-studio");
  const template = studio.getByTestId("workflow-template-card")
    .filter({ hasText: "Develop computational method" });
  await expect(template).toContainText("durable method search");
  await template.click();

  const nodes = studio.getByTestId("workflow-graph-node");
  await expect(nodes).toHaveCount(7);
  await expect(studio.getByTestId("workflow-graph-edge")).toHaveCount(8);
  const rootPositions = await nodes.evaluateAll((items) => items
    .filter((item) => ["literature_methods", "data_audit", "baseline_analysis"]
      .includes(item.getAttribute("data-node-id") ?? ""))
    .map((item) => item.getBoundingClientRect().x));
  expect(rootPositions).toHaveLength(3);
  expect(Math.max(...rootPositions) - Math.min(...rootPositions)).toBeLessThan(2);

  const activityNode = studio.locator(
    '[data-testid="workflow-graph-node"][data-node-id="method_search"]',
  );
  await expect(activityNode).toHaveClass(/run-activity/);
  await activityNode.getByTestId("workflow-graph-node-select").click();
  const inspector = studio.getByTestId("workflow-graph-inspector");
  await expect(inspector.getByTestId("dynamic-task-type"))
    .toHaveValue("run_activity");
  await expect(inspector.getByTestId("run-activity-config")).toBeVisible();
  await expect(inspector.getByTestId("run-activity-input-task"))
    .toHaveValue("prepare_contract");
  await expect(inspector.getByTestId("run-activity-max-candidates")).toHaveValue("20");
  await expect(inspector.getByTestId("dynamic-task-capabilities")).toBeHidden();
  await expect(inspector.getByTestId("dynamic-task-skills")).toBeHidden();
  await expect(inspector.getByTestId("dynamic-task-specialist")).toBeHidden();
  await expect(studio.getByTestId("workflow-save")).toHaveText("Save as copy");
});

test("Skill Portfolio Planner uses the selected model and opens an unbudgeted editable DAG", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Workflows");
  const studio = page.getByTestId("workflow-studio");

  await studio.getByTestId("portfolio-planner-open").click();
  await expect(page.getByTestId("portfolio-planner-overlay")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("portfolio-planner-overlay")).toBeHidden();

  await studio.getByTestId("portfolio-planner-open").click();
  await expect(page.getByTestId("portfolio-planner-overlay")).toContainText(
    "Ask a selected model to build an explainable workflow",
  );
  await expect(page.getByTestId("portfolio-tier")).toHaveCount(0);
  await expect(page.getByTestId("portfolio-total")).toHaveCount(0);
  await expect(page.getByTestId("portfolio-reserve")).toHaveCount(0);
  await page.getByTestId("portfolio-model").selectOption("opus");
  await page.getByTestId("portfolio-request").fill("Design an oncology omics study");
  await page.getByTestId("portfolio-generate").click();
  await expect.poll(() => lastInvokeArgs(page, "plan_skill_portfolio")).toEqual({
    request: {
      request: "Design an oncology omics study",
      model_id: "opus",
    },
  });
  const card = page.getByTestId("portfolio-plan-card");
  await expect(card).toContainText("3 tasks · 2 Skills · planned by opus-4.8");
  await expect(card).toContainText("Task budgets are unset");
  await card.getByTestId("portfolio-edit-studio").click();
  await expect(studio.getByTestId("workflow-graph-node")).toHaveCount(3);
  await expect(studio.getByTestId("workflow-graph-edge")).toHaveCount(2);
});

test("Workflow Studio reuses the roundtable generator and saves a Quick Action binding", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Workflows");
  const studio = page.getByTestId("workflow-studio");
  await expect(studio).toBeVisible();

  await studio.getByTestId("workflow-new").click();
  await studio.getByTestId("workflow-studio-config").locator(":scope > summary").click();
  await studio.getByTestId("workflow-name").fill("Architecture roundtable");
  await studio.getByTestId("workflow-goal").fill("Choose a website architecture");
  await studio.getByTestId("roundtable-template").locator("summary").click();
  await studio.getByTestId("roundtable-apply").click();
  await expect(studio.getByTestId("workflow-graph-node")).toHaveCount(5);
  await expect(studio.getByTestId("workflow-graph-edge")).toHaveCount(6);
  const skillPicker = studio.getByTestId("workflow-graph-inspector")
    .getByTestId("dynamic-task-skills");
  await skillPicker.getByTestId("dynamic-task-skill-search").fill("analysis");
  await skillPicker.getByTestId("dynamic-task-skill-option")
    .filter({ hasText: "analysis-workflow" })
    .click();
  await expect(skillPicker.getByTestId("dynamic-task-selected-skills"))
    .toContainText("analysis-workflow · bundled");
  await studio.getByTestId("workflow-save").click();

  await expect.poll(() => lastInvokeArgs(page, "save_workflow_template")).toMatchObject({
    template: {
      id: "",
      name: "Architecture roundtable",
      proposal: {
        goal: "Choose a website architecture",
        tasks: [
          { id: "seat_1_opening", depends_on: [], skill_ids: ["analysis-workflow"] },
          { id: "seat_2_opening", depends_on: [] },
          {
            id: "seat_1_review",
            depends_on: ["seat_1_opening", "seat_2_opening"],
          },
          {
            id: "seat_2_review",
            depends_on: ["seat_1_opening", "seat_2_opening"],
          },
          {
            id: "chair_synthesis",
            depends_on: ["seat_1_review", "seat_2_review"],
          },
        ],
      },
    },
  });
  await expect(studio.getByTestId("workflow-template-card")).toHaveCount(4);

  await studio.getByTestId("workflow-studio-back").click();
  await expect(page.locator(".settings-nav")).toBeVisible();
  await page.getByTestId("quick-action-new").click();
  await page.getByTestId("quick-action-name").fill("Discuss selection");
  await page.getByTestId("quick-action-workflow").selectOption("workflow_3");
  await page.getByTestId("quick-action-save").click();
  await expect.poll(() => lastInvokeArgs(page, "save_quick_action")).toMatchObject({
    action: {
      name: "Discuss selection",
      workflow_template_id: "workflow_3",
      context: "selection",
      enabled: true,
    },
  });
  await expect(page.getByTestId("quick-action-row")
    .filter({ hasText: "Discuss selection" })).toContainText("Architecture roundtable");
});

test("Workflow graph edits nodes and dependencies directly on the canvas", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Workflows");
  const studio = page.getByTestId("workflow-studio");
  await studio.getByTestId("workflow-new").click();
  await studio.getByTestId("workflow-studio-config").locator(":scope > summary").click();
  await studio.getByTestId("workflow-name").fill("Graph pipeline");
  await studio.getByTestId("workflow-goal").fill("Compare two branches");

  const inspector = studio.getByTestId("workflow-graph-inspector");
  await inspector.getByTestId("dynamic-task-id").fill("fetch_a");
  await inspector.getByTestId("dynamic-task-instruction").fill("Fetch branch A");
  await studio.getByTestId("workflow-graph-add-node").click();
  await inspector.getByTestId("dynamic-task-id").fill("fetch_b");
  await inspector.getByTestId("dynamic-task-instruction").fill("Fetch branch B");
  await studio.getByTestId("workflow-graph-add-after").click();
  await inspector.getByTestId("dynamic-task-id").fill("merge");
  await inspector.getByTestId("dynamic-task-instruction").fill("Merge both branches");

  const byId = (id: string) =>
    studio.locator(`[data-testid="workflow-graph-node"][data-node-id="${id}"]`);

  await expect(studio.getByTestId("workflow-graph-edge")).toHaveCount(1);
  await expect(studio.locator(
    '[data-testid="workflow-graph-edge"][data-source="fetch_b"][data-target="merge"]',
  )).toHaveCount(1);

  await byId("fetch_a").getByTestId("workflow-graph-connect").click();
  await expect(studio.getByTestId("workflow-graph-connect-hint")).toContainText("fetch_a");
  await byId("merge").getByTestId("workflow-graph-node-select").click();
  await expect(studio.getByTestId("workflow-graph-edge")).toHaveCount(2);
  await expect(studio.locator(
    '[data-testid="workflow-graph-edge"][data-source="fetch_a"][data-target="merge"]',
  )).toHaveCount(1);

  const positions = await studio.getByTestId("workflow-graph-node")
    .evaluateAll((items) => Object.fromEntries(items.map((item) => {
      const box = item.getBoundingClientRect();
      return [item.getAttribute("data-node-id"), { x: box.x, y: box.y }];
    })));
  expect(Math.abs(positions.fetch_a.x - positions.fetch_b.x)).toBeLessThan(2);
  expect(positions.merge.x).toBeGreaterThan(positions.fetch_a.x);

  await studio.locator(
    '[data-testid="workflow-graph-edge-group"][data-source="fetch_a"][data-target="merge"]',
  ).getByTestId("workflow-graph-edge-delete").click({ force: true });
  await expect(studio.getByTestId("workflow-graph-edge")).toHaveCount(1);

  await inspector.getByTestId("workflow-graph-remove-edge")
    .filter({ hasText: "fetch_b" })
    .click();
  await expect(studio.getByTestId("workflow-graph-edge")).toHaveCount(0);

  await byId("fetch_b").getByTestId("workflow-graph-connect").click();
  await byId("merge").getByTestId("workflow-graph-node-select").click();
  await expect(studio.getByTestId("workflow-graph-edge")).toHaveCount(1);

  await byId("merge").getByTestId("workflow-graph-connect").click();
  await byId("fetch_b").getByTestId("workflow-graph-node-select").click();
  await expect(studio.getByTestId("workflow-studio-error")).toContainText("cycle");
  await expect(studio.getByTestId("workflow-graph-edge")).toHaveCount(1);

  await byId("fetch_a").getByTestId("workflow-graph-connect").click();
  await expect(studio.getByTestId("workflow-graph-connect-hint")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(studio.getByTestId("workflow-graph-connect-hint")).toHaveCount(0);

  const nodeCountBeforeDblclick = await studio.getByTestId("workflow-graph-node").count();
  // Top-left padding is empty of nodes (nodes start ~58px down); use element-relative dblclick.
  await studio.getByTestId("workflow-graph-canvas").dblclick({ position: { x: 12, y: 12 } });
  await expect(studio.getByTestId("workflow-graph-node")).toHaveCount(nodeCountBeforeDblclick + 1);
  await inspector.getByTestId("dynamic-task-id").fill("fetch_c");
  await inspector.getByTestId("dynamic-task-instruction").fill("Fetch branch C");

  await byId("fetch_a").getByTestId("workflow-graph-delete-node").click();
  await expect(studio.getByTestId("workflow-graph-node")).toHaveCount(3);
  await studio.getByTestId("workflow-save").click();
  await expect.poll(() => lastInvokeArgs(page, "save_workflow_template")).toMatchObject({
    template: {
      name: "Graph pipeline",
      proposal: {
        tasks: [
          { id: "fetch_b", depends_on: [] },
          { id: "merge", depends_on: ["fetch_b"] },
          { id: "fetch_c", depends_on: [] },
        ],
      },
    },
  });
  expect(await byId("fetch_a").count()).toBe(0);
});

test("selected text can be staged as a removable side-chat quote", async ({ page }) => {
  await enterApp(page);
  await expect(page.getByTestId("session-runtime-strip")).toBeVisible();
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });

  await expect(page.getByRole("button", { name: "Hide follow-up questions" })).toBeVisible();
  const selected = await selectAssistantReplyText(page);
  const popup = page.locator(".selection-popup");
  await expect(popup.getByRole("button", { name: "Quote in side chat" })).toBeVisible();
  // Follow-scroll / composer resize must not steal the popup while the
  // selection is still live — that is what failed ui-e2e on #1027.
  await page.locator(".chat").evaluate((el) => {
    el.scrollTop = Math.max(0, el.scrollTop - 8);
  });
  await expect(popup.getByRole("button", { name: "Quote in side chat" })).toBeVisible();
  await popup.getByRole("button", { name: "Quote in side chat" }).click();

  const panel = page.locator(".rightpane");
  await expect(panel.locator(".sidechat-in-pane")).toBeVisible();
  const quote = panel.getByTestId("sidechat-quote");
  await expect(quote).toContainText(selected.slice(0, 30));
  const input = panel.getByPlaceholder("Follow up…");
  await expect(input).toBeFocused();

  // Removing the staged quote leaves the side-chat draft untouched.
  await quote.getByRole("button", { name: "Remove attachment" }).click();
  await expect(panel.getByTestId("sidechat-quote")).toHaveCount(0);

  await selectAssistantReplyText(page);
  await expect(popup.getByRole("button", { name: "Quote in side chat" })).toBeVisible();
  await popup.getByRole("button", { name: "Quote in side chat" }).click();
  await input.fill("Why is this important?");
  await panel.getByRole("button", { name: "Send" }).click();

  await expect.poll(() => lastInvokeArgs(page, "side_chat")).toMatchObject({
    question: expect.stringContaining("Why is this important?"),
  });
  const args = await lastInvokeArgs(page, "side_chat");
  expect(args.question).toContain(`> ${selected}`);
  expect(args.question).not.toContain("AI source-edit instruction");
  await expect(panel.getByTestId("sidechat-quote")).toHaveCount(0);
  await expect(input).toHaveValue("");
});

test("branch in new session starts a new frame from the current session", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("seed context");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await composer(page).fill("try another route");
  await page.getByRole("button", { name: "Message options" }).click();
  await page.getByRole("button", { name: "Branch in new session" }).click();

  await expect.poll(() => lastInvokeArgs(page, "branch_session")).toMatchObject({
    title: "try another route",
  });
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    sessionId: expect.stringMatching(/^branch-/),
    message: "try another route",
  });
});

test("branch on an earlier user message opens a new session from that point", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("first idea");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await composer(page).fill("second idea");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.locator(".msg.user", { hasText: "second idea" })).toBeVisible();

  const firstUser = page.locator(".msg.user", { hasText: "first idea" });
  await firstUser.getByRole("button", { name: "Branch" }).click();

  await expect.poll(() => lastInvokeArgs(page, "branch_session")).toMatchObject({
    sessionId: expect.stringMatching(/^s-/),
    title: "first idea",
    userIndex: 0,
  });
  await expect(composer(page)).toHaveValue("");
  await expect(page.locator(".msg.user", { hasText: "second idea" })).toHaveCount(0);

  await composer(page).fill("first idea, but normalize first");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    sessionId: expect.stringMatching(/^branch-/),
    message: "first idea, but normalize first",
  });
});

test("assistant actions are icon-only and can branch from the preceding user turn", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("branch from this answer");
  await page.getByRole("button", { name: "Send" }).click();
  const assistant = page.locator(".msg.assistant").filter({ hasText: "Hello from mock wisp-science." }).first();
  await expect(assistant).toBeVisible({ timeout: 10_000 });

  await composer(page).fill("a later turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.locator(".msg.assistant").filter({ hasText: "Hello from mock wisp-science." })).toHaveCount(2);

  for (const name of ["Memory", "Review", "Branch"]) {
    const action = assistant.getByRole("button", { name, exact: true });
    await expect(action).toBeVisible();
    await expect(action.locator("span")).toHaveCount(0);
  }

  await assistant.getByRole("button", { name: "Branch", exact: true }).click();
  await expect.poll(() => lastInvokeArgs(page, "branch_session")).toMatchObject({
    title: "branch from this answer",
    userIndex: 0,
  });
  await expect(composer(page)).toHaveValue("");
});

test("rewinding a middle message asks for confirmation", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("first idea");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await composer(page).fill("second idea");
  await page.getByRole("button", { name: "Send" }).click();
  const firstUser = page.locator(".msg.user", { hasText: "first idea" });
  await expect(page.locator(".msg.user", { hasText: "second idea" })).toBeVisible();
  await expect(page.locator(".msg.assistant")).toHaveCount(2, { timeout: 10_000 });
  await expect(firstUser.getByRole("button", { name: "Rewind" })).toBeEnabled();

  const modal = page.getByTestId("edit-confirm-modal");
  await firstUser.getByRole("button", { name: "Rewind" }).click();
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("permanently removes all conversation after this message");
  // While the modal is open nothing is rewound and the transcript is intact.
  expect(await lastInvokeArgs(page, "rewind_session")).toBeNull();
  await expect(page.locator(".msg.user", { hasText: "second idea" })).toBeVisible();

  // Confirming Rewind runs the destructive rewind to the first message.
  await modal.getByRole("button", { name: "Rewind", exact: true }).click();
  await expect(modal).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "rewind_session")).toMatchObject({
    sessionId: expect.stringMatching(/^s-/),
    userIndex: 0,
  });
  await expect(composer(page)).toHaveValue("first idea");
  await expect(page.locator(".msg.user", { hasText: "second idea" })).toHaveCount(0);
  await expect(page.locator(".msg.assistant")).toHaveCount(0);
});

test("rewind confirmation can branch instead", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("first idea");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await composer(page).fill("second idea");
  await page.getByRole("button", { name: "Send" }).click();
  const firstUser = page.locator(".msg.user", { hasText: "first idea" });
  await expect(page.locator(".msg.user", { hasText: "second idea" })).toBeVisible();
  await expect(page.locator(".msg.assistant")).toHaveCount(2, { timeout: 10_000 });
  await expect(firstUser.getByRole("button", { name: "Rewind" })).toBeEnabled();

  const modal = page.getByTestId("edit-confirm-modal");
  await firstUser.getByRole("button", { name: "Rewind" }).click();
  await expect(modal).toBeVisible();
  await modal.getByRole("button", { name: "Branch" }).click();
  await expect(modal).toHaveCount(0);

  await expect.poll(() => lastInvokeArgs(page, "branch_session")).toMatchObject({
    sessionId: expect.stringMatching(/^s-/),
    title: "first idea",
    userIndex: 0,
  });
  // Branching is non-destructive: no rewind happened.
  expect(await lastInvokeArgs(page, "rewind_session")).toBeNull();
  await expect(composer(page)).toHaveValue("");
});

test("Escape closes only the rewind confirmation modal and keeps the transcript", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("first idea");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await composer(page).fill("second idea");
  await page.getByRole("button", { name: "Send" }).click();
  const firstUser = page.locator(".msg.user", { hasText: "first idea" });
  await expect(page.locator(".msg.user", { hasText: "second idea" })).toBeVisible();
  await expect(page.locator(".msg.assistant")).toHaveCount(2, { timeout: 10_000 });
  await expect(firstUser.getByRole("button", { name: "Rewind" })).toBeEnabled();

  const modal = page.getByTestId("edit-confirm-modal");
  await firstUser.getByRole("button", { name: "Rewind" }).click();
  await expect(modal).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  // One Escape closed the modal only — no rewind, no branch, transcript intact.
  expect(await lastInvokeArgs(page, "rewind_session")).toBeNull();
  expect(await lastInvokeArgs(page, "branch_session")).toBeNull();
  await expect(page.locator(".msg.user", { hasText: "second idea" })).toBeVisible();
  await expect(page.locator(".msg.assistant")).toHaveCount(2);
});

test("generic content menus do not expose session export", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("hello there");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await page.getByText("Hello from mock wisp-science.").click({ button: "right" });
  await expect(page.getByRole("button", { name: "Export session" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator(".rightpane").click({ button: "right", position: { x: 5, y: 100 } });
  await expect(page.locator(".ctx-menu")).toHaveCount(0);
});

test("uploaded file shows up in the artifacts panel after send", async ({ page }) => {
  await enterApp(page);
  await page.setInputFiles("#composer-file-input", {
    name: "counts.csv",
    mimeType: "text/csv",
    buffer: Buffer.from("a,b\n1,2"),
  });
  await expect(page.locator(".composer-attachment.ready")).toHaveText("counts.csv");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "send_message");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : (args ?? null);
  })).toMatchObject({
    message: "Uploaded files: uploads/counts.csv",
    attachments: ["uploads/counts.csv"],
  });
  // One user bubble only — attachment suffix must not spawn a duplicate turn.
  await expect(page.locator(".msg.user")).toHaveCount(1);
  await expect(page.locator(".msg.user .user-attachment-file")).toContainText("counts.csv");
  await expect(page.locator(".msg.user")).not.toContainText("Uploaded files:");
  await expect(page.locator(".center-tab.active")).not.toContainText("Uploaded files:");
  // The right panel starts collapsed; open it to see the collected artifact.
  await page.getByRole("button", { name: "Toggle panel" }).click();
  // The upload path lives in the user turn; the panel must pick it up from there.
  const tile = page.locator('.rp-tile[data-artifact-name="counts.csv"]');
  await expect(tile).toBeVisible();
  await tile.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-tab.active")).toContainText("counts.csv");
  await expect(page.locator(".center-file-preview")).toContainText("a");
  await page.locator(".center-tabs > .center-tab").click();
  await tile.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Download" }).click();
  await expect.poll(() => lastInvokeArgs(page, "download_file")).toMatchObject({ path: "uploads/counts.csv" });
});

test("Generated artifacts survive follow-up tool commentary and ignore mentioned paths", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("ARTIFACTATTRIBUTION");
  await page.getByRole("button", { name: "Send" }).click();

  const reply = page.locator(".msg.assistant", {
    hasText: "I inspected old.csv and created the requested output",
  });
  await expect(reply).toBeVisible({ timeout: 10_000 });
  await expect(reply.locator(".message-artifacts-label")).toHaveText("Generated · 1");
  await expect(reply.locator('.message-artifact-card[data-artifact-name="new.png"]')).toBeVisible();
  await expect(reply.locator('.message-artifact-card[data-artifact-name="old.csv"]')).toHaveCount(0);
  await expect(reply.locator('.message-artifact-card[data-artifact-name="old.png"]')).toHaveCount(0);
  await expect(reply.locator('.message-artifact-card[data-artifact-name="old-report.md"]')).toHaveCount(0);
  const pathLink = reply.locator('a.workspace-path-link[href="notes/FIGURE_LEGEND.md"]');
  await expect(pathLink).toHaveText("notes/FIGURE_LEGEND.md");
  await expect.poll(async () => pathLink.evaluate((el) => {
    const style = getComputedStyle(el);
    return {
      decoration: style.textDecorationLine,
      shadow: style.boxShadow,
      display: style.display,
    };
  })).toEqual({
    decoration: "none",
    shadow: "none",
    display: "inline",
  });

  // Project paths in assistant replies own a file-focused context menu rather
  // than falling through to the generic whole-message menu.
  await pathLink.click({ button: "right" });
  const pathMenu = page.locator(".ctx-menu");
  await expect(pathMenu.getByRole("button", { name: "Open in center" })).toBeVisible();
  await expect(pathMenu.getByRole("button", { name: "Copy absolute path" })).toBeVisible();
  await expect(pathMenu.getByRole("button", { name: "Copy relative path" })).toBeVisible();
  await expect(pathMenu.getByRole("button", { name: "Copy message" })).toHaveCount(0);
  await pathMenu.getByRole("button", { name: "Show in file manager" }).click();
  await expect.poll(() => lastInvokeArgs(page, "reveal_in_file_manager")).toMatchObject({
    path: "notes/FIGURE_LEGEND.md",
  });
  await expect(page.locator(".artifact-modal")).toHaveCount(0);

  // A generated file mentioned in the answer is rendered as an artifact chip,
  // not a workspace-path anchor. It must own the same file menu and must not
  // fall through to "Copy message" (the real-world #1048 regression).
  const artifactPath = reply.locator('.art-ref[data-workspace-path="/mock/root/results/new.png"]');
  await expect(artifactPath).toHaveText("new.png");
  await artifactPath.click({ button: "right" });
  await expect(pathMenu.getByRole("button", { name: "Open with default app" })).toBeVisible();
  await expect(pathMenu.getByRole("button", { name: "Copy message" })).toHaveCount(0);
  await pathMenu.getByRole("button", { name: "Open with default app" }).click();
  await expect.poll(() => lastInvokeArgs(page, "open_workspace_path")).toMatchObject({
    path: "results/new.png",
  });

  // Ordinary project-directory Markdown links share the same route.
  const directoryPath = reply.locator('a[href="results/"]');
  await directoryPath.click({ button: "right" });
  await expect(pathMenu.getByRole("button", { name: "Open with default app" })).toBeVisible();
  await pathMenu.getByRole("button", { name: "Show in file manager" }).click();
  await expect.poll(() => lastInvokeArgs(page, "reveal_in_file_manager")).toMatchObject({
    path: "results/",
  });

  await pathLink.click();
  const linkedModal = page.locator('.artifact-modal:has(.am-figure[data-file-path="notes/FIGURE_LEGEND.md"])');
  await expect(linkedModal).toBeVisible();
  await expect(linkedModal.locator(".am-name")).toHaveText("FIGURE_LEGEND.md");
  await expect(page.locator(".center-file-preview")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(linkedModal).toHaveCount(0);
  await expect(reply).toBeVisible();

  // The backing artifact path remains absolute for loading, but project UI
  // must present the portable project-relative form.
  await reply.locator('.message-artifact-card[data-artifact-name="new.png"]').click();
  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await modal.getByRole("button", { name: "Open in center" }).click();
  const preview = page.locator('.center-file-preview[data-file-path="/mock/root/results/new.png"]');
  await expect(preview).toBeVisible();
  await expect(preview.locator(".center-file-head > span").first()).toHaveText("results/new.png");
});

test("links inside the artifact modal preview never navigate the app", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("ARTIFACTATTRIBUTION");
  await page.getByRole("button", { name: "Send" }).click();

  const reply = page.locator(".msg.assistant", {
    hasText: "I inspected old.csv and created the requested output",
  });
  await reply.locator('a.workspace-path-link[href="notes/FIGURE_LEGEND.md"]').click();
  const modal = page.locator('.artifact-modal:has(.am-figure[data-file-path="notes/FIGURE_LEGEND.md"])');
  await expect(modal).toBeVisible();

  const appUrl = page.url();
  await modal.locator('a[href="https://example.com/paper"]').click();
  await page.getByTestId("external-link-open").click();
  await expect.poll(() => lastInvokeArgs(page, "open_external_url")).toMatchObject({
    url: "https://example.com/paper",
  });
  expect(page.url()).toBe(appUrl);
  await expect(modal).toBeVisible();

  await modal.locator('a[href="results/new.png"]').click();
  await expect.poll(async () => page.locator(".artifact-modal").count()).toBe(1);
  expect(page.url()).toBe(appUrl);
  const openCalls = await page.evaluate(() => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      return value;
    };
    return ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "open_external_url").map((c: any) => plain(c.args));
  });
  expect(openCalls).toEqual([{ url: "https://example.com/paper" }]);
});

test("artifact category headers collapse and expand their tiles", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();

  const tile = page.locator('.rp-tile[data-artifact-name="volcano.png"]');
  await expect(tile).toBeVisible();
  const group = page.locator(".rp-art-group").filter({ has: tile });
  const header = group.locator(".rp-art-group-label");
  await expect(header).toHaveAttribute("aria-expanded", "true");

  await header.click();
  await expect(group).toHaveClass(/collapsed/);
  await expect(header).toHaveAttribute("aria-expanded", "false");
  await expect(tile).toBeHidden();

  await header.click();
  await expect(group).not.toHaveClass(/collapsed/);
  await expect(header).toHaveAttribute("aria-expanded", "true");
  await expect(tile).toBeVisible();
});

test("dropped local file uploads and attaches to the composer", async ({ page }) => {
  await enterApp(page);
  await page.locator(".composer-inner").evaluate((el) => {
    const data = new DataTransfer();
    data.items.add(new File(["gene,value\nBRCA1,2"], "dropped.csv", { type: "text/csv" }));
    el.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
    el.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
  });
  await expect(page.locator(".composer-attachment.ready")).toHaveText("dropped.csv");
});

test("workspace file context menu attaches its path to the composer", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  const file = page.locator('.fb-row[data-workspace-path="report.csv"]');
  await expect(file).toBeVisible();
  const json = page.locator('.fb-row[data-workspace-path="config.json"]');
  await json.click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-file-preview .rp-code")).toBeVisible();
  await expect(page.locator(".center-file-preview")).toContainText('"model"');
  await page.locator('.center-tab[data-center-path="config.json"]').click({ button: "right" });
  await page.getByRole("button", { name: "Close current" }).click();
  await file.click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-file-preview")).toContainText("a");
  await expect(page.locator(".center-tab.active")).toContainText("report.csv");

  const search = page.locator(".fb-search");
  await search.fill("counts");
  const counts = page.locator('.fb-row[data-workspace-path="counts.csv"]');
  await expect(counts).toBeVisible();
  await counts.click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  await page.locator('.center-tab[data-center-path="report.csv"]').click({ button: "right" });
  await page.getByRole("button", { name: "Close tabs to the right" }).click();
  await expect(page.locator('.center-tab[data-center-path="counts.csv"]')).toHaveCount(0);
  await page.locator('.center-tab[data-center-path="report.csv"]').click({ button: "right" });
  await page.getByRole("button", { name: "Close current" }).click();
  await expect(page.locator(".center-file-preview")).toHaveCount(0);

  await counts.click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  await page.locator('.center-tab[data-center-path="counts.csv"]').click({ button: "right" });
  await page.getByRole("button", { name: "Close all files" }).click();
  await expect(page.locator('.center-tab[data-center-path]')).toHaveCount(0);
  await expect(composer(page)).toBeVisible();
  await search.fill("");
  await expect(file).toBeVisible();
  await file.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Download" }).click();
  await expect.poll(() => lastInvokeArgs(page, "download_file")).toMatchObject({ path: "report.csv" });
  await file.click({ button: "right" });
  await page.getByRole("button", { name: "Attach to chat" }).click();
  await expect(page.locator(".composer-attachment.ready")).toHaveText("report.csv");
  await expect(composer(page)).toHaveValue("");
});

test("artifact tile attaches to the chat from its context and more menus", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();

  const tile = page.locator('.rp-tile[data-artifact-name="volcano.png"]');
  await expect(tile).toBeVisible();

  // Right-click → "Attach to chat" drops the artifact path into the composer.
  await tile.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Attach to chat" }).click();
  await expect(page.locator(".composer-attachment.ready")).toHaveText("volcano.png");
  await expect(composer(page)).toHaveValue("");

  // The "More" menu offers the same handoff; re-attaching dedupes to one chip.
  await tile.getByRole("button", { name: "More" }).click();
  await page.locator(".rp-tile-menu").getByRole("button", { name: "Attach to chat" }).click();
  await expect(page.locator(".composer-attachment.ready")).toHaveCount(1);
});

test("workspace Files panel navigates deeply nested analysis modules", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();

  await page.locator('.fb-row[data-workspace-path="DEG"]').click();
  await expect(page.locator(".fb-path")).toHaveText("DEG");
  await expect(page.locator('.fb-row[data-workspace-path="DEG/scripts"]')).toBeVisible();
  await expect(page.locator('.fb-row[data-workspace-path="DEG/output"]')).toBeVisible();

  await page.locator('.fb-row[data-workspace-path="DEG/output"]').click();
  await page.locator('.fb-row[data-workspace-path="DEG/output/figures"]').click();
  await expect(page.locator(".fb-path")).toHaveText("DEG/output/figures");
  await expect(
    page.locator('.fb-row[data-workspace-path="DEG/output/figures/volcano.png"]'),
  ).toBeVisible();

  await page.getByRole("button", { name: "Refresh" }).click();
  await expect(
    page.locator('.fb-row[data-workspace-path="DEG/output/figures/volcano.png"]'),
  ).toBeVisible();
});

test("workspace folder can be added to chat context (#694)", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();

  const folder = page.locator('.fb-row.dir[data-workspace-path="DEG"]');
  await folder.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Add folder to chat" }).click();

  await expect(page.locator(".composer-attachment.ready")).toHaveText("DEG");
  await composer(page).fill("Inspect this directory");
  await composer(page).press("Enter");
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    message: "Inspect this directory\n\nUploaded files: DEG",
  });
});

test("workspace file can be registered as an artifact", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  const file = page.locator('.fb-row[data-workspace-path="report.csv"]');
  await expect(file).toBeVisible();

  await file.click({ button: "right" });
  await page.getByRole("button", { name: "Register as artifact" }).click();

  await expect.poll(() => lastInvokeArgs(page, "register_artifact")).toMatchObject({
    path: "report.csv",
  });
  await expect(page.locator("#copy-toast")).toHaveText("Registered report.csv as an artifact");

  await page.getByRole("button", { name: /^Artifacts/ }).click();
  await expect(page.locator('.rp-tile[data-artifact-name="report.csv"]')).toBeVisible();
});

test("Files copies absolute, relative, and multi-selected workspace paths", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, "writeText", {
      configurable: true,
      value: async (text: string) => { (window as any).__copiedWorkspacePaths = text; },
    });
  });

  const files = page.locator(".rp-files");
  const report = files.locator('.fb-row[data-workspace-path="report.csv"]');
  const data = files.locator('.fb-row.dir[data-workspace-path="data"]');

  await report.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Copy absolute path" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__copiedWorkspacePaths)).toBe(
    "/mock/root/report.csv",
  );

  await data.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Copy relative path" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__copiedWorkspacePaths)).toBe("data");

  await files.getByRole("button", { name: "Select" }).click();
  await data.click();
  await report.click();
  await expect(data).toHaveAttribute("aria-pressed", "true");
  await expect(report).toHaveAttribute("aria-pressed", "true");

  await report.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Copy absolute path" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__copiedWorkspacePaths)).toBe(
    "/mock/root/data\n/mock/root/report.csv",
  );

  await report.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Copy relative path" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__copiedWorkspacePaths)).toBe(
    "data\nreport.csv",
  );
});

test("Files creates, renames, deletes, and refreshes local entries", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  const files = page.locator(".rp-files");

  await files.getByRole("button", { name: "New file" }).click();
  const entryInput = page.locator("#file-entry-modal-input");
  await expect(entryInput).toBeFocused();
  await entryInput.fill("notes.md");
  await page.locator(".file-entry-modal").getByRole("button", { name: "Create" }).click();
  await expect(files.locator('[data-workspace-path="notes.md"]')).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "create_file")).toMatchObject({ path: "notes.md" });

  await files.getByRole("button", { name: "New folder" }).click();
  await entryInput.fill("results");
  await page.locator(".file-entry-modal").getByRole("button", { name: "Create" }).click();
  const folder = files.locator('.fb-row.dir[data-workspace-path="results"]');
  await expect(folder).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "create_directory")).toMatchObject({ path: "results" });

  const file = files.locator('[data-workspace-path="notes.md"]');
  await file.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Rename file" }).click();
  await expect(entryInput).toHaveValue("notes.md");
  await entryInput.fill("research-notes.md");
  await page.locator(".file-entry-modal").getByRole("button", { name: "Rename" }).click();
  const renamedFile = files.locator('[data-workspace-path="research-notes.md"]');
  await expect(renamedFile).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "rename_entry")).toMatchObject({
    path: "notes.md",
    newPath: "research-notes.md",
  });

  await folder.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Rename folder" }).click();
  await entryInput.fill("outputs");
  await page.locator(".file-entry-modal").getByRole("button", { name: "Rename" }).click();
  const renamedFolder = files.locator('.fb-row.dir[data-workspace-path="outputs"]');
  await expect(renamedFolder).toBeVisible();

  await renamedFile.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Delete file" }).click();
  await page.locator(".confirm-modal").getByRole("button", { name: "Delete file" }).click();
  await expect(renamedFile).toHaveCount(0);

  await renamedFolder.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Delete folder" }).click();
  await page.locator(".confirm-modal").getByRole("button", { name: "Delete folder" }).click();
  await expect(renamedFolder).toHaveCount(0);

  await files.getByRole("button", { name: "Refresh" }).click();
  await expect.poll(() => lastInvokeArgs(page, "list_dir")).toMatchObject({ path: "." });
});

test("Files sorts by size and modified time and Escape closes only the sort menu", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  const files = page.locator(".rp-files");
  const fileRows = files.locator(".fb-row:not(.dir)");

  await expect(files.locator('.fb-row[data-workspace-path="report.csv"] .fb-size')).toHaveText("4.0 KB");

  await files.getByTestId("files-sort").click();
  const sortMenu = files.locator(".fb-sort-menu");
  await expect(sortMenu).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(sortMenu).toHaveCount(0);
  await expect(files).toBeVisible();

  await files.getByTestId("files-sort").click();
  await sortMenu.getByRole("menuitem", { name: "Size" }).click();
  await expect(fileRows.first()).toHaveAttribute("data-workspace-path", "manuscript.docx");
  await expect(fileRows.first().locator(".fb-size")).toHaveText("11.1 KB");

  await files.getByTestId("files-sort").click();
  await sortMenu.getByRole("menuitem", { name: "Modified" }).click();
  await expect(fileRows.first()).toHaveAttribute("data-workspace-path", "report.csv");
  await expect(fileRows.first().locator(".fb-size")).toHaveText(/^\d{2}:\d{2}$/);
  await expect(files.locator('.fb-row.dir[data-workspace-path="DEG"] .fb-size')).toHaveText(/\d/);
});

test("text entries keep the native context menu", async ({ page }) => {
  await enterApp(page);
  await page.locator(".proj-switch").click();
  await page.getByRole("button", { name: "Project settings" }).click();

  const modal = page.locator(".proj-settings-modal");
  const name = modal.locator("input").first();
  const description = modal.locator("textarea").first();
  for (const entry of [name, description]) {
    const defaultPrevented = await entry.evaluate((element) => {
      const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
      element.dispatchEvent(event);
      return event.defaultPrevented;
    });
    expect(defaultPrevented).toBe(false);
    await expect(page.locator(".ctx-menu")).toHaveCount(0);
  }
});

test("project settings save opt-in run workspace retention windows", async ({ page }) => {
  await enterApp(page);
  await page.locator(".proj-switch").click();
  await page.getByRole("button", { name: "Project settings" }).click();

  const settings = page.locator(".proj-settings-modal");
  // Each window is a labeled row: the first input's label used to be missing.
  await expect(settings.getByText("Succeeded runs", { exact: true })).toBeVisible();
  await expect(settings.getByText("Failed runs", { exact: true })).toBeVisible();
  await expect(settings.getByText("Orphaned files", { exact: true })).toBeVisible();
  const succeeded = settings.getByTestId("retention-succeeded");
  const failed = settings.getByTestId("retention-failed");
  const orphan = settings.getByTestId("retention-orphan");
  // Off by default.
  await expect(succeeded).toHaveValue("");
  await expect(failed).toHaveValue("");
  await expect(orphan).toHaveValue("");

  await succeeded.fill("7");
  await succeeded.blur();
  // Empty fields serialize as undefined (their sweeps stay off).
  await expect.poll(() => lastInvokeArgs(page, "set_project_run_retention")).toMatchObject({
    runRetentionDays: 7,
  });
  expect(
    (await lastInvokeArgs(page, "set_project_run_retention")).failedRunRetentionDays ?? null,
  ).toBeNull();
  expect(
    (await lastInvokeArgs(page, "set_project_run_retention")).orphanFileRetentionDays ?? null,
  ).toBeNull();
  await failed.fill("14");
  await failed.blur();
  await orphan.fill("30");
  await orphan.blur();
  await expect.poll(() => lastInvokeArgs(page, "set_project_run_retention")).toMatchObject({
    runRetentionDays: 7,
    failedRunRetentionDays: 14,
    orphanFileRetentionDays: 30,
  });

  // Reopening reflects the stored windows.
  await page.keyboard.press("Escape");
  await page.locator(".proj-switch").click();
  await page.getByRole("button", { name: "Project settings" }).click();
  await expect(settings.getByTestId("retention-succeeded")).toHaveValue("7");
  await expect(settings.getByTestId("retention-failed")).toHaveValue("14");
  await expect(settings.getByTestId("retention-orphan")).toHaveValue("30");
});

test("saving a changed agent context asks for confirmation", async ({ page }) => {
  await enterApp(page);
  await page.locator(".proj-switch").click();
  await page.getByRole("button", { name: "Project settings" }).click();

  const settings = page.locator(".proj-settings-modal");
  await expect(settings).toBeVisible();
  await settings.locator("textarea.ps-ctx").fill("Prefer the project UI setting.");
  await settings.getByRole("button", { name: "Save", exact: true }).click();

  const confirm = page.locator(".confirm-modal");
  await expect(confirm).toBeVisible();
  await expect(confirm).toContainText(".wisp/WISP.md");
  await expect(confirm).toContainText("Existing conversations");
  await page.keyboard.press("Escape");
  await expect(confirm).toHaveCount(0);
  await expect(settings).toBeVisible();
  await expect.poll(() => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "update_project")
  )).toBe(false);

  await settings.getByRole("button", { name: "Save", exact: true }).click();
  await confirm.getByRole("button", { name: "Save agent context", exact: true }).click();
  await expect.poll(() => lastInvokeArgs(page, "update_project")).toMatchObject({
    name: "wisp-science",
    agentContext: "Prefer the project UI setting.",
  });
  await expect(settings).toHaveCount(0);
  await expect(page.locator(".copy-toast")).toHaveCount(0);
});

test("center structure and FASTA previews fill the available height", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1200 });
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();

  const openInCenter = async (path: string) => {
    await page.locator(`[data-workspace-path="${path}"]`).click({ button: "right" });
    await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  };
  const heightRatio = (selector: string) => page.locator(".center-file-preview").evaluate((preview, childSelector) => {
    const child = preview.querySelector<HTMLElement>(childSelector);
    return child ? child.getBoundingClientRect().height / preview.getBoundingClientRect().height : 0;
  }, selector);

  await openInCenter("model.pdb");
  await expect(page.locator('.center-file-preview[data-preview-kind="structure"] .rp-3dmol')).toBeVisible();
  await expect(page.locator('.center-file-preview[data-preview-kind="structure"] .rp-3dmol canvas')).toBeVisible();
  await expect.poll(() => heightRatio(".rp-3dmol")).toBeGreaterThan(0.75);

  await openInCenter("sequences.fasta");
  await expect(page.locator('.center-file-preview[data-preview-kind="fasta"] .rp-fasta-wrap')).toBeVisible();
  await expect.poll(() => heightRatio(".rp-fasta-wrap")).toBeGreaterThan(0.75);
});

test("script previews show source while unknown file types are explicitly unsupported", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();

  const openInCenter = async (path: string) => {
    await page.locator(`[data-workspace-path="${path}"]`).click({ button: "right" });
    await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  };

  await openInCenter("analysis.R");
  await expect(page.locator(".center-file-preview")).toContainText("plot(1:3)");
  await expect.poll(() => lastInvokeArgs(page, "read_file")).toMatchObject({ path: "analysis.R" });

  // #307: the script rendered as one unhighlighted paragraph. It must come back
  // as R-tagged code, one line per line, with a matching line-number gutter.
  const rCode = page.locator(".center-file-preview .rp-code-body code");
  await expect(rCode).toHaveClass(/language-r/);
  await expect(rCode.locator(".hljs-string")).toHaveText('"data"');
  await expect(page.locator(".center-file-preview .rp-code-gutter")).toHaveText("1\n2\n3\n4");

  // An extension no mime claims (#307: pixi.toml) is still text — preview it.
  await openInCenter("pixi.toml");
  const tomlCode = page.locator(".center-file-preview .rp-code-body code");
  await expect(tomlCode).toHaveClass(/language-ini/);
  await expect(tomlCode.locator(".hljs-string")).toHaveText('"x"');
  await expect(page.locator(".center-file-preview")).toContainText("[project]");

  await openInCenter("protocol.rtf");
  await expect(page.locator('.center-file-preview[data-preview-kind="document"]')).toContainText(
    "Experimental protocol",
  );
  await expect(page.locator('.center-file-preview[data-preview-kind="document"] strong')).toHaveText("12000 g");
  await expect.poll(() => lastInvokeArgs(page, "read_file")).toMatchObject({ path: "protocol.rtf" });

  // Genuinely binary payloads stay explicitly unsupported.
  await openInCenter("analysis.unknown");
  await expect(page.locator(".center-file-preview .rp-error")).toHaveText(
    "Preview is not supported for this file type.",
  );
});

test("selected workspace code tells the agent to edit its source and refreshes after the tool writes", async ({ page }) => {
  await enterApp(page);
  // Establish the conversation before opening the file; center tabs are scoped
  // per session and intentionally reset when a brand-new session is created.
  await composer(page).fill("prepare source edit");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).not.toBeNull();
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('[data-workspace-path="analysis.R"]').click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();

  const preview = page.locator('.center-file-preview[data-file-path="analysis.R"]');
  const source = preview.locator(".rp-code-body code");
  await expect(source).toContainText("plot(1:3)");
  await source.evaluate((element) => {
    const range = document.createRange();
    range.selectNodeContents(element);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
  });
  await page.locator(".selection-popup")
    .getByRole("button", { name: "Ask AI in the conversation" })
    .click();
  await expect(page.locator(".composer-reference-chips .quote"))
    .toContainText("analysis.R");

  await composer(page).fill("改成一个散点图画图的例子");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(async () => {
    const calls = ((await page.evaluate(() => (window as any).__skillInvokeLog ?? [])) as any[])
      .filter((call) => call.cmd === "send_message");
    return calls.length;
  }).toBeGreaterThanOrEqual(2);
  const args = await lastInvokeArgs(page, "send_message");
  expect(args.message).toContain("Selected excerpt from workspace file `analysis.R`:");
  expect(args.message).toContain("改成一个散点图画图的例子");
  expect(args.message).toContain("read the selected workspace file first");
  expect(args.message).toContain("edit tool");
  // Agent-only transport guidance is persisted for reliable behavior but is
  // not shown as if the user had typed it.
  await expect(page.locator(".msg.user").last()).not.toContainText("AI source-edit instruction");

  await page.evaluate(() => {
    (window as any).__setMockWorkspaceR('df <- data.frame(x = 1:3, y = c(2, 5, 4))\nplot(df$x, df$y)\n');
    (window as any).__tauriEmit("agent", {
      kind: "FileChanged",
      frame_id: "t1",
      path: "/mock/root/analysis.R",
    });
  });
  await expect(preview).toHaveAttribute("data-file-revision", "1");
  await expect(preview.locator(".rp-code-body code")).toContainText("plot(df$x, df$y)");
});

test("notebook preview renders saved rich outputs without active content", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('[data-workspace-path="analysis.ipynb"]').click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();

  const preview = page.locator('.center-file-preview[data-preview-kind="notebook"]');
  await expect(preview.locator(".notebook-cell")).toHaveCount(2);
  await expect(preview.locator("h2")).toHaveText("Saved notebook output");
  await expect(preview.locator(".notebook-source")).toContainText("display(result)");

  const htmlFrame = preview.locator("iframe.rp-notebook-html");
  await expect(htmlFrame).toBeVisible();
  await expect(htmlFrame).toHaveAttribute("sandbox", "");
  await expect(htmlFrame).toHaveAttribute("referrerpolicy", "no-referrer");
  const htmlOutput = page.frameLocator("iframe.rp-notebook-html");
  await expect(htmlOutput.locator("#saved-table")).toContainText("safe HTML result");
  await expect(htmlOutput.locator("script")).toHaveCount(0);
  await expect(htmlOutput.locator("#external-image")).not.toHaveAttribute("src", /.+/);
  await expect(htmlOutput.locator("#external-image")).not.toHaveAttribute("onerror", /.+/);
  await expect(htmlOutput.locator("#external-image")).toHaveAttribute("loading", "lazy");

  const svg = preview.locator("img.rp-notebook-svg");
  await expect(svg).toBeVisible();
  await expect(svg).toHaveAttribute("loading", "lazy");
  const svgSource = await svg.getAttribute("src");
  expect(svgSource).toMatch(/^blob:/);
  const sanitizedSvg = await page.evaluate(async (src) => {
    return src ? await (await fetch(src)).text() : "";
  }, svgSource);
  expect(sanitizedSvg).not.toContain("<script");

  await expect(preview.locator(".nb-out-latex .katex")).toBeVisible();
  const raster = preview.locator(".notebook-output > img.rp-img");
  await expect(raster).toHaveAttribute("loading", "lazy");
  await expect.poll(() => page.evaluate(() => Boolean((window as any).__notebookPwned))).toBe(false);
});

test("R scripts expose variables and console and run selected code", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('[data-workspace-path="analysis.R"]').click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-file-preview")).toContainText("plot(1:3)");

  // Only contexts that can host an R runtime are offered. The mock's local
  // machine has no Rscript, so the binding resolves to the one host that does
  // rather than defaulting to a local runtime that could never run this.
  const binding = page.getByRole("combobox", { name: "Runtime this script runs in" });
  await expect(binding.locator("option")).toHaveText(["R · gpu-server"]);
  await expect(binding).toHaveValue("ssh:gpu-server");

  const filePreview = page.locator(".center-file-preview");
  await expect(filePreview.getByRole("button", { name: "Run script" })).toBeVisible();
  await expect(filePreview.getByRole("button", { name: "Rewind" })).toHaveCount(0);

  // The replacement control opens the RStudio-style workbench: variable rail,
  // an initially empty console, and an empty plots pane — without executing
  // the file.
  await expect(page.locator(".center-file-console")).toHaveCount(0);
  await page.getByRole("button", { name: "Show runtime variables and console" }).click();
  await expect(page.locator(".center-runtime-environment")).toBeVisible();
  await expect(page.locator(".center-file-console")).toContainText("Run selected code or type in the prompt below");
  await expect(page.locator(".center-runtime-plots")).toContainText("Plots from executed code appear here.");

  // Selecting code still offers the floating execution path from the source
  // preview. highlight.js splits the source into spans, so select one it produces.
  await page.locator(".center-file-preview .rp-code-body .hljs-string").evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
  });
  const popup = page.locator(".selection-popup");
  await expect(popup).toHaveClass(/selection-popup-code/);
  await expect(popup).toHaveCSS("flex-direction", "column");
  await expect(popup.getByRole("button")).toHaveText([
    "Run in runtime",
    "Ask AI in the conversation",
    "Quote in side chat",
    "Explain in side chat",
  ]);
  await expect(popup.getByRole("button", { name: "Research literature" })).toHaveCount(0);
  await expect(popup.getByRole("button", { name: "Add to review" })).toHaveCount(0);
  await popup.getByRole("button", { name: "Run in runtime" }).click();
  await expect.poll(() => lastInvokeArgs(page, "execute_runtime")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "r",
    code: '"data"',
  });
  const console_ = page.locator(".center-file-console pre");
  await expect(console_).toContainText('> "data"');
  await expect(console_).toContainText('[r @ ssh:gpu-server] "data"');
  await expect.poll(() => lastInvokeArgs(page, "inspect_runtime")).toMatchObject({
    projectId: "default",
    contextId: "ssh:gpu-server",
    language: "r",
  });
  await expect(page.locator(".center-runtime-environment")).toContainText("counts");
  await expect(page.locator(".center-file-preview")).toHaveCSS("overflow", "hidden");
  await expect(page.locator(".center-file-console")).not.toHaveCSS("position", "sticky");

  // RStudio quadrants: source top-left, console below it, variables top-right,
  // plots below the variables.
  const dockLayout = await page.locator(".center-file-preview").evaluate((preview) => {
    const source = preview.querySelector(".rp-code")!.getBoundingClientRect();
    const console_ = preview.querySelector(".center-file-console")!.getBoundingClientRect();
    const environment = preview.querySelector(".center-runtime-environment")!.getBoundingClientRect();
    const plots = preview.querySelector(".center-runtime-plots")!.getBoundingClientRect();
    return {
      sourceTop: Math.round(source.top),
      sourceBottom: Math.round(source.bottom),
      sourceRight: Math.round(source.right),
      consoleTop: Math.round(console_.top),
      consoleRight: Math.round(console_.right),
      environmentLeft: Math.round(environment.left),
      environmentTop: Math.round(environment.top),
      environmentBottom: Math.round(environment.bottom),
      plotsLeft: Math.round(plots.left),
      plotsTop: Math.round(plots.top),
    };
  });
  expect(dockLayout.consoleTop).toBeGreaterThanOrEqual(dockLayout.sourceBottom);
  expect(dockLayout.environmentLeft).toBeGreaterThanOrEqual(dockLayout.sourceRight);
  expect(dockLayout.environmentTop).toBeLessThanOrEqual(dockLayout.sourceTop);
  expect(dockLayout.plotsLeft).toBeGreaterThanOrEqual(dockLayout.consoleRight - 1);
  expect(dockLayout.plotsTop).toBeGreaterThanOrEqual(dockLayout.environmentBottom);

  // Clearing empties the log without closing the inspector or runtime.
  await page.getByRole("button", { name: "Clear console" }).click();
  await expect(page.locator(".center-file-console")).toContainText("Run selected code or type in the prompt below");
  await expect(page.locator(".center-runtime-environment")).toBeVisible();
});

test("the runtime console prompt executes typed code and captures plots", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('[data-workspace-path="analysis.R"]').click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  await page.getByRole("button", { name: "Show runtime variables and console" }).click();

  // Typing at the prompt runs against the bound runtime, echoes the input,
  // and appends the result — the RStudio console loop.
  const prompt = page.getByRole("textbox", { name: "Console input" });
  await expect(page.getByRole("button", { name: "Run console input (Enter)" })).toBeVisible();
  await prompt.fill("summary(x)");
  await prompt.press("Enter");
  await expect.poll(() => lastInvokeArgs(page, "execute_runtime")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "r",
    code: "summary(x)",
  });
  const console_ = page.locator(".center-file-console pre");
  await expect(console_).toContainText("> summary(x)");
  await expect(console_).toContainText("[r @ ssh:gpu-server] summary(x)");
  await expect(prompt).toHaveValue("");

  // A plotting command fills the bottom-right plots pane with the snapshot
  // the runtime captured; non-plotting commands leave it untouched.
  await expect(page.locator(".center-runtime-plots img")).toHaveCount(0);
  await prompt.fill("plot(1:3)");
  await prompt.press("Enter");
  await expect(page.locator(".center-runtime-plots img")).toHaveCount(1);
  await expect(page.locator(".center-runtime-plots-counter")).toHaveCount(0);

  // A second plot arrives selected; paging moves through the history.
  await prompt.fill("plot(4:6)");
  await prompt.press("Enter");
  await expect(page.locator(".center-runtime-plots-counter")).toHaveText("2 / 2");
  await page.getByRole("button", { name: "Previous plot" }).click();
  await expect(page.locator(".center-runtime-plots-counter")).toHaveText("1 / 2");
  await page.getByRole("button", { name: "Next plot" }).click();
  await expect(page.locator(".center-runtime-plots-counter")).toHaveText("2 / 2");

  // ArrowUp recalls submitted history for editing.
  await prompt.press("ArrowUp");
  await expect(prompt).toHaveValue("plot(4:6)");
  await prompt.press("ArrowUp");
  await expect(prompt).toHaveValue("plot(1:3)");
  await prompt.press("ArrowDown");
  await expect(prompt).toHaveValue("plot(4:6)");

  // The prompt is a real multi-line editor: Shift+Enter adds a line, while
  // the visible Run action submits the complete cell.
  await prompt.fill("x <- 1");
  await prompt.press("Shift+Enter");
  await prompt.type("x + 1");
  await expect(prompt).toHaveValue("x <- 1\nx + 1");
  await page.getByRole("button", { name: "Run console input (Enter)" }).click();
  await expect.poll(() => lastInvokeArgs(page, "execute_runtime")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "r",
    code: "x <- 1\nx + 1",
  });
  await expect(prompt).toHaveValue("");
  await prompt.press("ArrowUp");
  await expect(prompt).toHaveValue("plot(4:6)");
  await prompt.fill("");

  // Clearing the pane empties the history without touching the console.
  await page.getByRole("button", { name: "Clear plots" }).click();
  await expect(page.locator(".center-runtime-plots")).toContainText("Plots from executed code appear here.");
  await expect(console_).toContainText("[r @ ssh:gpu-server] plot(4:6)");
});

test("R sources are editable, save back to the workspace, and run selections", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('[data-workspace-path="analysis.R"]').click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();

  const editor = page.getByRole("textbox", { name: "Source editor" });
  await expect(editor).toHaveValue('library(Seurat)\nin_dir <- "data"\nplot(1:3)\n');
  // No save affordance until the source changes.
  await expect(page.locator("[data-editor-save]")).toHaveCount(0);

  await editor.fill("library(Seurat)\nplot(4:6)\n");
  // The highlighted mirror and the gutter follow the draft.
  await expect(page.locator(".center-file-preview .rp-code-body")).toContainText("plot(4:6)");
  await expect(page.locator(".center-file-preview .rp-code-gutter")).toHaveText("1\n2\n3");

  // Ctrl+S persists through the workspace-scoped save command.
  await editor.press("Control+s");
  await expect.poll(() => lastInvokeArgs(page, "save_file")).toMatchObject({
    path: "analysis.R",
    content: "library(Seurat)\nplot(4:6)\n",
  });
  await expect(page.locator("[data-editor-save]")).toHaveCount(0);

  // Selecting inside the editor still raises the quote popup with Run.
  await editor.evaluate((element) => {
    const textarea = element as HTMLTextAreaElement;
    textarea.focus();
    const start = textarea.value.indexOf("plot(4:6)");
    textarea.setSelectionRange(start, start + "plot(4:6)".length);
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
  });
  const selectedMark = page.locator(".rp-code-selection-layer mark");
  await expect(selectedMark).toHaveText("plot(4:6)");
  await expect(page.locator(".rp-code-selection-status")).toHaveText("Selected line 2");
  // Focused: native ::selection is the live paint so dragging is not delayed.
  const nativeSelection = await editor.evaluate((element) =>
    getComputedStyle(element, "::selection").backgroundColor);
  expect(nativeSelection).not.toBe("rgba(0, 0, 0, 0)");
  await page.locator(".selection-popup").getByRole("button", { name: "Run in runtime" }).click();
  await expect.poll(() => lastInvokeArgs(page, "execute_runtime")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "r",
    code: "plot(4:6)",
  });

  // Ctrl/⌘+Enter is the fast path. With a collapsed caret it runs the current
  // statement and advances to the next one; a persistent Run button exposes
  // the same behavior to users who have not learned the shortcut yet.
  await expect(page.locator("[data-editor-run]")).toBeVisible();
  await editor.evaluate((element) => {
    const textarea = element as HTMLTextAreaElement;
    textarea.focus();
    textarea.setSelectionRange(0, 0);
  });
  await editor.press("Control+Enter");
  await expect.poll(() => lastInvokeArgs(page, "execute_runtime")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "r",
    code: "library(Seurat)",
  });
  await expect.poll(() => editor.evaluate((element) =>
    (element as HTMLTextAreaElement).selectionStart)).toBe("library(Seurat)\n".length);

  await editor.evaluate((element) => {
    const textarea = element as HTMLTextAreaElement;
    const start = textarea.value.indexOf("plot(4:6)");
    textarea.focus();
    textarea.setSelectionRange(start, start + "plot(4:6)".length);
  });
  await editor.press("Control+Enter");
  await expect.poll(() => lastInvokeArgs(page, "execute_runtime")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "r",
    code: "plot(4:6)",
  });

  // A parenthesized call is one statement: caret on the first line sends both,
  // then advances past the whole call. Dragging across those lines updates the
  // toolbar before mouseup — the previous overlay only painted on select.
  await editor.fill("sce <- FindVariableFeatures(sce,\n  verbose = FALSE)\nplot(1)\n");
  await editor.evaluate((element) => {
    const textarea = element as HTMLTextAreaElement;
    textarea.focus();
    textarea.setSelectionRange(0, 0);
  });
  await editor.press("Control+Enter");
  await expect.poll(() => lastInvokeArgs(page, "execute_runtime")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "r",
    code: "sce <- FindVariableFeatures(sce,\n  verbose = FALSE)",
  });
  await expect.poll(() => editor.evaluate((element) =>
    (element as HTMLTextAreaElement).selectionStart)).toBe(
      "sce <- FindVariableFeatures(sce,\n  verbose = FALSE)\n".length,
    );

  const box = (await editor.boundingBox())!;
  await page.mouse.move(box.x + 24, box.y + 20);
  await page.mouse.down();
  await page.mouse.move(box.x + 80, box.y + 42, { steps: 6 });
  await expect.poll(() => editor.evaluate((element) => {
    const textarea = element as HTMLTextAreaElement;
    return textarea.selectionEnd - textarea.selectionStart;
  })).toBeGreaterThan(0);
  await expect(page.locator(".rp-code-selection-status")).toHaveText(/Selected lines 1–2/);
  await page.mouse.up();

  // The save button path: edit again and click Save.
  await editor.fill("plot(7:9)\n");
  await page.locator("[data-editor-save]").click();
  await expect.poll(() => lastInvokeArgs(page, "save_file")).toMatchObject({
    path: "analysis.R",
    content: "plot(7:9)\n",
  });

  // Whole-script run reads the saved file in the bound runtime. A dirty draft
  // is persisted first so the reported hash matches the buffer the user sees.
  await page.getByRole("button", { name: "Run script" }).click();
  await expect.poll(() => lastInvokeArgs(page, "execute_runtime_script")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "r",
    scriptPath: "analysis.R",
  });
  await expect(page.locator(".center-file-console")).toContainText("path=analysis.R");

  await editor.fill("plot(10:12)\n");
  await editor.press("Control+Shift+Enter");
  await expect.poll(() => lastInvokeArgs(page, "save_file")).toMatchObject({
    path: "analysis.R",
    content: "plot(10:12)\n",
  });
  await expect.poll(() => lastInvokeArgs(page, "execute_runtime_script")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "r",
    scriptPath: "analysis.R",
  });
});

test("runtime workbench dividers resize the quadrants by dragging", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('[data-workspace-path="analysis.R"]').click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  await page.getByRole("button", { name: "Show runtime variables and console" }).click();
  await expect(page.locator(".center-runtime-environment")).toBeVisible();

  // Dragging the vertical divider left widens the environment/plots column.
  const environment = page.locator(".center-runtime-environment");
  const environmentBefore = (await environment.boundingBox())!;
  const colDivider = (await page.locator(".center-runtime-col-resizer").boundingBox())!;
  await page.mouse.move(colDivider.x + 2, colDivider.y + colDivider.height / 2);
  await page.mouse.down();
  await page.mouse.move(colDivider.x - 120, colDivider.y + colDivider.height / 2);
  await page.mouse.up();
  const environmentAfter = (await environment.boundingBox())!;
  expect(environmentAfter.width).toBeGreaterThan(environmentBefore.width + 80);

  // Dragging the horizontal divider up makes the console/plots row taller.
  const console_ = page.locator(".center-file-console");
  const consoleBefore = (await console_.boundingBox())!;
  const rowDivider = (await page.locator(".center-runtime-row-resizer").boundingBox())!;
  await page.mouse.move(rowDivider.x + rowDivider.width / 2, rowDivider.y + 2);
  await page.mouse.down();
  await page.mouse.move(rowDivider.x + rowDivider.width / 2, rowDivider.y - 100);
  await page.mouse.up();
  const consoleAfter = (await console_.boundingBox())!;
  expect(consoleAfter.height).toBeGreaterThan(consoleBefore.height + 60);

  // Escape cancels an in-progress divider drag (topmost layer only).
  const divider = (await page.locator(".center-runtime-col-resizer").boundingBox())!;
  await page.mouse.move(divider.x + 2, divider.y + divider.height / 2);
  await page.mouse.down();
  await expect(page.locator(".drag-overlay")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".drag-overlay")).toHaveCount(0);
  await expect(page.locator(".center-runtime-environment")).toBeVisible();
  await page.mouse.up();
});

test("a Python script rebinds to another execution context", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('[data-workspace-path="qc.py"]').click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-file-preview")).toContainText("import scanpy");

  // Both mock contexts host Python, and local is the default binding.
  const binding = page.getByRole("combobox", { name: "Runtime this script runs in" });
  await expect(binding.locator("option")).toHaveText(["Python · Local machine", "Python · gpu-server"]);
  await expect(binding).toHaveValue("local");

  const runSelectedString = async () => {
    await page.locator(".center-file-preview .rp-code-body .hljs-string").evaluate((el) => {
      const range = document.createRange();
      range.selectNodeContents(el);
      const selection = window.getSelection()!;
      selection.removeAllRanges();
      selection.addRange(range);
      window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
    });
    await page.locator(".selection-popup").getByRole("button", { name: "Run in runtime" }).click();
  };

  await runSelectedString();
  await expect.poll(() => lastInvokeArgs(page, "execute_runtime")).toMatchObject({
    contextId: "local",
    language: "python",
  });

  // Rebinding sends the same script to the other context instead.
  await binding.selectOption("ssh:gpu-server");
  await runSelectedString();
  await expect.poll(() => lastInvokeArgs(page, "execute_runtime")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "python",
    code: '"counts.h5ad"',
  });
  await expect(page.locator(".center-file-console pre")).toContainText("[python @ ssh:gpu-server]");
});

test("files with no persistent runtime have no run control", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('[data-workspace-path="pixi.toml"]').click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-file-preview")).toContainText("[project]");

  // .toml highlights as code but has no runtime to bind to.
  await expect(page.getByRole("combobox", { name: "Runtime this script runs in" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Run script" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Run", exact: true })).toHaveCount(0);
});

test("Files browses registered SSH contexts without a real remote host", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();

  await page.getByRole("combobox", { name: "File location" }).selectOption("ssh:gpu-server");
  await expect(page.getByRole("combobox", { name: "File location" })).toHaveValue("ssh:gpu-server");
  await expect(page.getByRole("textbox", { name: "Remote path" })).toHaveValue("/home/research");
  await expect(page.locator('.remote-dir[data-remote-path="/home/research/projects"]')).toBeVisible();
  const remoteFile = page.locator('.remote-file[data-remote-path="/home/research/notes.txt"]');
  await expect(remoteFile).toContainText("notes.txt");
  await expect.poll(() => lastInvokeArgs(page, "list_remote_dir")).toMatchObject({
    contextId: "ssh:gpu-server",
    path: "~",
  });

  const remoteDownload = remoteFile.getByRole("button", { name: "Download" });
  await expect(remoteDownload).toBeVisible();
  await remoteDownload.click();
  await expect.poll(() => lastInvokeArgs(page, "download_file")).toMatchObject({
    path: "ssh://gpu-server/home/research/notes.txt",
  });

  // Keep secondary-click as an alternate path, but it is no longer the only one.
  await remoteFile.click({ button: "right" });
  await expect(page.locator(".ctx-menu").getByRole("button", { name: "Download" })).toBeVisible();
  await expect(page.locator(".ctx-menu").getByRole("button", { name: "Open in center" })).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.locator('.remote-dir[data-remote-path="/home/research/projects"]').click();
  await expect(page.getByRole("textbox", { name: "Remote path" })).toHaveValue("/home/research/projects");
  await expect(page.locator('.remote-file[data-remote-path="/home/research/projects/README.md"]')).toBeVisible();

  await page.getByRole("button", { name: "Parent directory" }).click();
  await expect(page.getByRole("textbox", { name: "Remote path" })).toHaveValue("/home/research");

  await page.getByRole("combobox", { name: "File location" }).selectOption("local");
  await expect(page.getByRole("combobox", { name: "File location" })).toHaveValue("local");
  await expect(page.getByRole("textbox", { name: "Remote path" })).toHaveCount(0);
  await expect(page.locator('[data-workspace-path="report.csv"]')).toBeVisible();
});

test("Files uploads local paths to the selected SSH folder", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.getByRole("combobox", { name: "File location" }).selectOption("ssh:gpu-server");
  await expect(page.getByRole("textbox", { name: "Remote path" })).toHaveValue("/home/research");

  const upload = page.getByTestId("files-remote-upload");
  await expect(upload).toBeVisible();
  await upload.click();
  await expect.poll(() => lastInvokeArgs(page, "upload_to_context")).toMatchObject({
    contextId: "ssh:gpu-server",
    destinationDir: "/home/research",
  });
  await expect(page.locator("#copy-toast")).toContainText("Uploading 1 item");
});

test("pasted image attaches to the composer", async ({ page }) => {
  await enterApp(page);
  await composer(page).evaluate((el) => {
    const data = new DataTransfer();
    data.items.add(new File([new Uint8Array([137, 80, 78, 71])], "clipboard.png", { type: "image/png" }));
    const event = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(event, "clipboardData", { value: data });
    el.dispatchEvent(event);
  });

  await expect(page.locator(".composer-attachment.ready")).toHaveText(/pasted_image_\d+_1\.png/);
  await expect(page.locator(".composer-attachment-row.image img")).toBeVisible();
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "send_message");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : (args ?? null);
  })).toMatchObject({
    message: expect.stringMatching(/^Uploaded files: uploads\/pasted_image_\d+_1\.png$/),
    attachments: [expect.stringMatching(/^uploads\/pasted_image_\d+_1\.png$/)],
  });
  await expect(page.locator(".msg.user .user-attachment-image img")).toBeVisible();
  await expect(page.locator(".msg.user")).not.toContainText("Uploaded files:");
});

test("compute menu selects remote resources per session", async ({ page }) => {
  await enterApp(page);

  const menu = await openComputeMenu(page);
  await expect(menu).toBeVisible();
  await expect(menu.getByText("Click a server to attach it to this chat.")).toBeVisible();
  await expect(menu.getByRole("button", { name: "Local", exact: true })).toHaveCount(0);
  await expect(menu.getByRole("button", { name: "Add SSH host…" })).toBeVisible();
  const defaultSelect = menu.getByTestId("compute-default-analysis");
  await expect(defaultSelect).toHaveValue("");
  await expect(defaultSelect).toContainText("Local by default");
  const search = menu.getByRole("searchbox", { name: "Search servers" });
  await search.fill("missing");
  await expect(menu.locator('[data-context-id="ssh:gpu-server"]')).toHaveCount(0);
  await search.fill("gpu");
  const server = menu.locator('[data-context-id="ssh:gpu-server"]');
  await expect(menu.locator(".compute-resource-list")).toHaveCSS("overflow-y", "auto");
  await expect(server).toHaveCSS("display", "grid");
  await expect(menu.getByRole("button", { name: "Manage environments in Settings" })).toBeVisible();
  await expect(server).not.toHaveClass(/enabled/);
  await expect(server).toContainText("Not in this chat");
  await server.click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_execution_context_enabled")).toMatchObject({
    sessionId: expect.any(String),
    contextId: "ssh:gpu-server",
    enabled: true,
  });
  const firstSession = (await lastInvokeArgs(page, "set_session_execution_context_enabled")).sessionId;
  await expect(server).toContainText("In this chat");
  await expect(page.locator(".composer-compute")).toHaveClass(/has-resource/);

  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await newSessionButton(page).click();
  const nextMenu = await openComputeMenu(page);
  await expect(nextMenu.locator('[data-context-id="ssh:gpu-server"]')).not.toHaveClass(/enabled/);
  await expect.poll(async () => (await lastInvokeArgs(page, "list_session_execution_context_ids"))?.sessionId)
    .not.toBe(firstSession);
});

test("compute menu sets and clears a default analysis environment", async ({ page }) => {
  await enterApp(page);

  const agentMenu = await openAgentMenu(page);
  await expect(agentMenu.getByRole("button", { name: /^Compute/ })).toContainText("Local by default");
  await agentMenu.getByRole("button", { name: /^Compute/ }).click();
  const menu = page.getByRole("menu", { name: "Compute" });
  const server = menu.locator('[data-context-id="ssh:gpu-server"]');
  await expect(server.locator(".compute-resource-default")).toHaveCount(0);
  await server.getByRole("button", { name: "Set as default analysis environment" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_default_execution_context")).toMatchObject({
    contextId: "ssh:gpu-server",
  });
  // Setting the default also selects it for the current session.
  await expect.poll(() => lastInvokeArgs(page, "set_session_execution_context_enabled")).toMatchObject({
    sessionId: expect.any(String),
    contextId: "ssh:gpu-server",
    enabled: true,
  });
  await expect(server).toHaveClass(/enabled/);
  await expect(server.locator(".compute-resource-default")).toHaveText("Default");
  await expect(server).toContainText("In this chat");
  await expect(agentMenu.getByRole("button", { name: /^Compute/ })).toContainText("Default gpu-server");

  await server.getByRole("button", { name: "Remove default" }).click();
  await expect.poll(async () =>
    (await lastInvokeArgs(page, "set_default_execution_context"))?.contextId ?? null
  ).toBeNull();
  await expect(server.locator(".compute-resource-default")).toHaveCount(0);
  await expect(agentMenu.getByRole("button", { name: /^Compute/ })).toContainText("1 remote");
});

test("starred default stays visible when the current chat has not attached it", async ({ page }) => {
  await enterApp(page);

  const menu = await openComputeMenu(page);
  await menu.locator('[data-context-id="ssh:gpu-server"]')
    .getByRole("button", { name: "Set as default analysis environment" })
    .click();
  await expect.poll(() => lastInvokeArgs(page, "set_default_execution_context")).toMatchObject({
    contextId: "ssh:gpu-server",
  });
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await newSessionButton(page).click();

  const agentMenu = await openAgentMenu(page);
  await expect(agentMenu.getByRole("button", { name: /^Compute/ })).toContainText("Default gpu-server");
  await expect(page.locator(".composer-compute")).toHaveClass(/has-resource/);
  await agentMenu.getByRole("button", { name: /^Compute/ }).click();
  const server = page.getByRole("menu", { name: "Compute" })
    .locator('[data-context-id="ssh:gpu-server"]');
  await expect(server).not.toHaveClass(/enabled/);
  await expect(server.locator(".compute-resource-default")).toHaveText("Default");
  await expect(server).toContainText("Auto-attaches");
  await expect(page.getByTestId("session-runtime-strip")
    .locator('[data-testid="session-runtime-group"][data-runtime-context="ssh:gpu-server"]'))
    .toBeVisible();
});

test("environment panel attaches and detaches remote servers", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();

  await expect(page.locator(".context-card", { hasText: "local" })).toBeVisible();
  await expect(page.locator(".context-card", { hasText: "ssh:gpu-server" })).toHaveCount(0);

  const attach = page.getByTestId("context-attach");
  await expect(attach.getByText("Attach server")).toBeVisible();
  const server = attach.locator('.context-attach-row[data-context-id="ssh:gpu-server"]');
  await expect(server).toBeVisible();
  await attach.getByRole("searchbox", { name: "Search servers" }).fill("missing");
  await expect(attach.locator('[data-context-id="ssh:gpu-server"]')).toHaveCount(0);
  await attach.getByRole("searchbox", { name: "Search servers" }).fill("gpu");
  await server.click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_execution_context_enabled")).toMatchObject({
    sessionId: expect.any(String),
    contextId: "ssh:gpu-server",
    enabled: true,
  });
  await expect(page.locator(".context-card", { hasText: "ssh:gpu-server" })).toBeVisible();
  await expect(attach.locator('[data-context-id="ssh:gpu-server"]')).toHaveCount(0);

  await page.locator(".context-card", { hasText: "ssh:gpu-server" })
    .getByRole("button", { name: "Remove from session" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_execution_context_enabled")).toMatchObject({
    contextId: "ssh:gpu-server",
    enabled: false,
  });
  await expect(page.locator(".context-card", { hasText: "ssh:gpu-server" })).toHaveCount(0);
  await expect(attach.locator('[data-context-id="ssh:gpu-server"]')).toBeVisible();
  await expect(page.locator(".context-card", { hasText: "local" })
    .getByRole("button", { name: "Remove from session" })).toHaveCount(0);
});

test("first server enable asks for storage locations and the rail can edit them", async ({ page }) => {
  await enterApp(page);
  // No saved preferences for this project × server → first enable prompts.
  await page.evaluate(() => {
    delete (window as any).__mockStoragePrefs["ssh:gpu-server"];
  });
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();

  const attach = page.getByTestId("context-attach");
  await attach.locator('.context-attach-row[data-context-id="ssh:gpu-server"]').click();
  const dialog = page.getByTestId("storage-prefs-modal");
  await expect(dialog).toBeVisible();
  await expect(dialog.locator("#storage-prefs-data-root")).toHaveValue("~/wisp/demo-project/data");
  await expect(dialog.locator("#storage-prefs-workdir-root")).toHaveValue(".wisp-science/runs");
  await expect(dialog.locator("#storage-prefs-results-dir")).toHaveValue("remote/gpu-server");

  // Escape immediately: one press closes only the dialog; the Environment
  // rail (its parent surface) stays open with the now-attached server.
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(page.locator(".context-card", { hasText: "ssh:gpu-server" })).toBeVisible();
  expect(await lastInvokeArgs(page, "set_context_storage_prefs")).toBeNull();

  // The rail's storage action reopens the dialog; saving persists the edits.
  await page.locator(".context-card", { hasText: "ssh:gpu-server" })
    .getByRole("button", { name: "Storage locations" }).click();
  await expect(dialog).toBeVisible();
  await dialog.locator("#storage-prefs-data-root").fill("/scratch/demo/data");
  await dialog.locator("#storage-prefs-results-dir").fill("results/from-gpu");
  await dialog.getByRole("button", { name: "Save" }).click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "set_context_storage_prefs")).toMatchObject({
    contextId: "ssh:gpu-server",
    remoteDataRoot: "/scratch/demo/data",
    remoteWorkdirRoot: ".wisp-science/runs",
    localResultsDir: "results/from-gpu",
  });

  // Saved preferences: re-enabling never prompts again.
  await page.locator(".context-card", { hasText: "ssh:gpu-server" })
    .getByRole("button", { name: "Remove from session" }).click();
  await attach.locator('.context-attach-row[data-context-id="ssh:gpu-server"]').click();
  await expect(page.locator(".context-card", { hasText: "ssh:gpu-server" })).toBeVisible();
  await expect(dialog).toHaveCount(0);
});

test("settings sets the default analysis environment", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Environments");

  const select = page.getByTestId("default-analysis-environment");
  await expect(select).toBeVisible();
  await expect(select).toHaveValue("");
  await expect(select).toContainText("Local by default");
  await expect(select).toContainText("gpu-server");
  await select.selectOption("ssh:gpu-server");
  await expect.poll(() => lastInvokeArgs(page, "set_default_execution_context")).toMatchObject({
    contextId: "ssh:gpu-server",
  });

  await page.getByRole("button", { name: "Back to app" }).click();
  const agentMenu = await openAgentMenu(page);
  await expect(agentMenu.getByRole("button", { name: /^Compute/ })).toContainText("Default gpu-server");
  await agentMenu.getByRole("button", { name: /^Compute/ }).click();
  await expect(page.getByTestId("compute-default-analysis")).toHaveValue("ssh:gpu-server");
});

test("settings manages servers and probes them with the default environment skill", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Environments");

  const server = page.locator('.environment-settings-row[data-context-id="ssh:gpu-server"]');
  const local = page.locator('.environment-settings-row[data-context-id="local"]');
  await expect(server).toBeVisible();
  await expect(local).toBeVisible();
  await expect(page.locator(".environment-resource-toggle")).toHaveCount(0);
  const rowHeights = await page.locator(".environment-settings-row").evaluateAll((rows) =>
    rows.map((row) => row.getBoundingClientRect().height),
  );
  expect(Math.max(...rowHeights) - Math.min(...rowHeights)).toBeLessThanOrEqual(1);
  const [localConfigure, serverConfigure] = await Promise.all([
    local.getByRole("button", { name: "Configure runtime interpreters" }).boundingBox(),
    server.getByRole("button", { name: "Configure runtime interpreters" }).boundingBox(),
  ]);
  expect(localConfigure?.x).toBe(serverConfigure?.x);

  await local.getByRole("button", { name: "Configure runtime interpreters" }).click();
  await expect(page.getByRole("heading", { name: "Runtime interpreters" })).toBeVisible();
  await expect(page.locator("#runtime-python-executable")).toBeVisible();
  await expect(page.locator("#runtime-rscript-executable")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toBeVisible();

  await server.getByRole("button", { name: "Probe context" }).click();
  await expect.poll(() => lastInvokeArgs(page, "probe_execution_context")).toMatchObject({
    contextId: "ssh:gpu-server",
  });
});

test("runtime interpreter dialog prefills probed paths and picks files (#651)", async ({ page }) => {
  // Simulate a successful probe that found interpreters outside PATH: the
  // context has no explicit config, only probe results. Patch the mock before
  // the app boots so every fetch of the contexts sees the probe data.
  await page.addInitScript(() => {
    const local = (window as any).__mockExecutionContexts.find((item: any) => item.id === "local");
    local.config_json = "{}";
    local.capabilities_json = JSON.stringify({
      python_executable: "/opt/conda/bin/python",
      rscript_executable: "D:\\R-4.5.2\\bin\\Rscript.exe",
      r_jsonlite: true,
    });
  });
  await enterApp(page);
  await openSettingsSection(page, "Environments");

  const local = page.locator('.environment-settings-row[data-context-id="local"]');
  await local.getByRole("button", { name: "Configure runtime interpreters" }).click();
  const python = page.locator("#runtime-python-executable");
  const rscript = page.locator("#runtime-rscript-executable");
  await expect(python).toHaveValue("/opt/conda/bin/python");
  await expect(rscript).toHaveValue("D:\\R-4.5.2\\bin\\Rscript.exe");

  // The local context offers a native file picker (mocked) that fills the field.
  const pythonPicker = page.locator(".runtime-config-picker", { has: python });
  await pythonPicker.getByRole("button", { name: "Browse…" }).click();
  await expect(python).toHaveValue("/mock/picked/Rscript");
  await page.keyboard.press("Escape");

  // Remote contexts have no local file picker, but still prefill probed paths.
  const server = page.locator('.environment-settings-row[data-context-id="ssh:gpu-server"]');
  await server.getByRole("button", { name: "Configure runtime interpreters" }).click();
  await expect(page.locator("#runtime-rscript-executable")).toHaveValue("/opt/R/bin/Rscript");
  await expect(page.getByRole("button", { name: "Browse…" })).toHaveCount(0);
});

test("environment probe shows progress and classifies password authentication failure", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Environments");
  await page.evaluate(() => {
    (window as any).__delayNextProbe(350);
    const context = (window as any).__mockExecutionContexts.find(
      (item: any) => item.id === "ssh:gpu-server",
    );
    context.last_probe_status = "error";
    context.last_probe_error =
      "SSH password authentication failed for `gpu-server`: the server rejected the saved password. Check the password, user name, and whether the server allows password login.";
  });

  const server = page.locator('.environment-settings-row[data-context-id="ssh:gpu-server"]');
  await server.getByRole("button", { name: "Probe context" }).click();
  await expect(server.getByRole("button", { name: "Probing…" })).toBeDisabled();
  await expect(server.getByRole("status")).toContainText(
    "Connecting, verifying SSH authentication, and reading environment information…",
  );

  const modal = page.getByTestId("ssh-connectivity-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("SSH password authentication failed");
  await expect(modal).toContainText("saved password may be wrong or outdated");
  await expect(server.getByRole("status")).toHaveCount(0);
});

test("missing optional uname output does not fail an otherwise usable SSH probe", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Environments");
  await page.evaluate(() => {
    const context = (window as any).__mockExecutionContexts.find(
      (item: any) => item.id === "ssh:gpu-server",
    );
    context.last_probe_status = "ok";
    context.last_probe_error = null;
    context.capabilities_json = JSON.stringify({ arch: "x86_64", hostname: "gpu-server" });
  });

  const server = page.locator('.environment-settings-row[data-context-id="ssh:gpu-server"]');
  await server.getByRole("button", { name: "Probe context" }).click();

  await expect(page.locator(".copy-toast")).toHaveText(
    "SSH connection confirmed. Some optional system information was unavailable, but the host can be used.",
  );
  await expect(page.getByTestId("ssh-connectivity-modal")).toHaveCount(0);
});

test("settings edits an existing SSH server with its saved values", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Environments");

  const server = page.locator('.environment-settings-row[data-context-id="ssh:gpu-server"]');
  await server.getByRole("button", { name: "Edit server" }).click();

  const modal = page.locator(".host-modal");
  await expect(modal).toBeVisible();
  await expect(modal.getByRole("heading", { name: "Edit SSH host" })).toBeVisible();
  await expect(modal.locator("#add-host-alias")).toHaveValue("gpu-server");
  await expect(modal.locator("#add-host-alias")).toBeDisabled();
  await expect(modal.locator("#host-user")).toHaveValue("researcher");
  await expect(modal.locator("#host-port")).toHaveValue("22");
  await expect(modal.locator("#host-notes")).toHaveValue("Mock GPU host");

  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  await expect(page.locator(".settings-page")).toBeVisible();
});

test("Escape closes the topmost environment modal before settings", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Environments");
  await page.getByRole("button", { name: "Add SSH host" }).click();
  await expect(page.locator(".host-modal")).toBeVisible();

  await page.keyboard.press("Escape");

  await expect(page.locator(".host-modal")).toHaveCount(0);
  await expect(page.locator(".settings-page")).toBeVisible();
});

test("Escape closes the SSH connectivity dialog", async ({ page }) => {
  await enterApp(page);
  await page.evaluate(() => {
    const context = (window as any).__mockExecutionContexts.find(
      (item: any) => item.id === "ssh:gpu-server",
    );
    context.last_probe_status = "error";
    context.last_probe_error =
      "SSH authentication succeeded, but the remote account did not execute Wisp's non-interactive probe commands. Check for a restricted shell, forced command, or a login startup script that exits early.";
  });

  const menu = await openComputeMenu(page);
  await menu.locator('[data-context-id="ssh:gpu-server"]').click();
  const modal = page.getByTestId("ssh-connectivity-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("SSH connected — environment information unavailable");
  await expect(modal).toContainText("successful password check alone is not enough");

  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  await expect(newSessionButton(page)).toBeVisible();
});

test("Escape closes the compute submenu before Agent options", async ({ page }) => {
  await enterApp(page);
  await openComputeMenu(page);
  await expect(page.getByRole("menu", { name: "Compute" })).toBeVisible();

  await page.keyboard.press("Escape");

  await expect(page.getByRole("menu", { name: "Compute" })).toHaveCount(0);
  await expect(page.getByRole("menu", { name: "Agent options" })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu", { name: "Agent options" })).toHaveCount(0);
});

test("Escape closes the sidebar sort menu without closing the sidebar", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Sort and group" }).click();
  await expect(page.locator(".side-sort-menu")).toBeVisible();

  await page.keyboard.press("Escape");

  await expect(page.locator(".side-sort-menu")).toHaveCount(0);
  await expect(newSessionButton(page)).toBeVisible();
});

test("agent menu updates review, reviewer model, and memory preferences", async ({ page }) => {
  await enterApp(page);
  let menu = await openAgentMenu(page);

  const delegation = menu.locator("label.agent-menu-row", { hasText: "Delegation" });
  await expect(delegation.locator('input[type="checkbox"]')).not.toBeChecked();
  await delegation.click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_delegation_enabled"))
    .toMatchObject({ enabled: true });

  const completion = page.getByTestId("agent-completion-policy");
  await expect(completion).toHaveValue("inline");
  await expect(menu.locator("label.agent-menu-row", { hasText: "Auto-resume parent" })).toHaveCount(0);
  await completion.selectOption("background");
  await expect.poll(() => lastInvokeArgs(page, "set_session_agent_completion"))
    .toMatchObject({ policy: "background", autoResume: false });
  const autoResume = menu.locator("label.agent-menu-row", { hasText: "Auto-resume parent" });
  await expect(autoResume).toBeVisible();
  await autoResume.click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_agent_completion"))
    .toMatchObject({ policy: "background", autoResume: true });
  await completion.selectOption("inline");
  await expect.poll(() => lastInvokeArgs(page, "set_session_agent_completion"))
    .toMatchObject({ policy: "inline", autoResume: false });
  await expect(autoResume).toHaveCount(0);

  await menu.locator("label.agent-menu-row", { hasText: "Auto-review" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_auto_review_enabled")).toMatchObject({ enabled: true });

  await menu.getByRole("button", { name: /^Reviewer model/ }).click();
  await page.getByRole("menu", { name: "Reviewer model" })
    .getByRole("button", { name: "opus-4.8" }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_specialist_cmd")).toMatchObject({
    spec: {
      id: "reviewer",
      model_id: "opus",
      review_backend: { kind: "http_model", profile_id: "opus" },
    },
  });
  menu = await openAgentMenu(page);
  await expect(menu.getByRole("button", { name: /Reviewer model opus-4\.8/ })).toBeVisible();
  await menu.getByRole("button", { name: /^Reviewer model/ }).click();
  const reviewerMenu = page.getByRole("menu", { name: "Reviewer model" });
  await expect(reviewerMenu).toBeVisible();
  await expect.poll(async () => {
    const [mainBox, reviewerBox] = await Promise.all([menu.boundingBox(), reviewerMenu.boundingBox()]);
    return mainBox && reviewerBox ? Math.round(reviewerBox.x - (mainBox.x + mainBox.width)) : null;
  }).toBeGreaterThan(5);
  await reviewerMenu.getByRole("button", { name: /Test ACP Agent/ }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_specialist_cmd")).toMatchObject({
    spec: {
      id: "reviewer",
      review_backend: { kind: "acp_agent", profile_id: "acp-test" },
    },
  });
  menu = await openAgentMenu(page);
  await expect(menu.getByRole("button", { name: /Reviewer model Test ACP Agent/ })).toBeVisible();
  await menu.getByRole("button", { name: /^Reviewer model/ }).click();
  await page.getByRole("menu", { name: "Reviewer model" })
    .getByRole("button", { name: "Follow session backend" }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_specialist_cmd")).toMatchObject({
    spec: { id: "reviewer", review_backend: { kind: "follow_session" } },
  });
  menu = await openAgentMenu(page);

  await menu.locator("label.agent-menu-row", { hasText: "Memory" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_memory_enabled")).toMatchObject({ enabled: false });

  await menu.locator("label.agent-menu-row", { hasText: "Auto-review" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_auto_review_enabled")).toMatchObject({ enabled: false });
  await menu.getByRole("button", { name: /^Reviewer model/ }).click();
  await page.getByRole("menu", { name: "Reviewer model" })
    .getByRole("button", { name: "Default" }).click();
  menu = await openAgentMenu(page);
  await menu.getByRole("button", { name: /^Reviewer model/ }).click();
  await expect(page.getByRole("menu", { name: "Reviewer model" })).toBeVisible();
});

test("project research graph opens from the sidebar in list and graph views", async ({ page }) => {
  await enterApp(page);

  const sidebar = page.locator(".sidebar");
  const navLabels = await sidebar.locator(".nav > .side-btn").allTextContents();
  expect(navLabels.indexOf("Research graph")).toBe(navLabels.indexOf("Publication") - 1);
  expect(navLabels.indexOf("Publication")).toBe(navLabels.indexOf("Library") - 1);

  await sidebar.getByRole("button", { name: "Research graph", exact: true }).click();
  const modal = page.getByTestId("research-graph-modal");
  await expect(modal).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);

  await sidebar.getByRole("button", { name: "Research graph", exact: true }).click();
  await expect(modal).toBeVisible();
  await expect(modal.locator(".research-graph-heading h2")).toHaveCSS("font-family", /Source Serif/);
  await expect(modal).toContainText("5 nodes · 3 relationships");
  await expect(modal.getByTestId("research-graph-list")).toBeVisible();
  await expect(modal.getByText("Use DESeq2 over edgeR")).toBeVisible();
  await expect(modal).toContainText("applies to");
  await expect(modal).toContainText("confidence: high");
  await modal.getByRole("button", { name: "cites: Love et al. 2014" }).click();
  const edgeDetail = modal.getByTestId("research-edge-detail");
  await expect(edgeDetail).toContainText("Use DESeq2 over edgeR → Love et al. 2014");
  await expect(edgeDetail).toContainText("confidence");
  await expect(edgeDetail).toContainText("high");
  await page.keyboard.press("Escape");
  await expect(edgeDetail).toHaveCount(0);
  await expect(modal).toBeVisible();
  await expect.poll(async () => (await invokeArgsList(page, "get_research_graph")).length).toBe(2);

  await modal.getByRole("tab", { name: "Graph", exact: true }).click();
  const canvas = modal.getByTestId("research-graph-canvas");
  await expect(canvas).toBeVisible();
  await expect(canvas.locator(".research-graph-node")).toHaveCount(5);
  await expect(canvas.locator(".research-graph-edge")).toHaveCount(3);
  const metadataEdge = canvas.getByRole("button", { name: /cites.*confidence: high/ });
  await expect(metadataEdge.locator("title")).toContainText("evidence: Methods section");
  await metadataEdge.click();
  await expect(modal.getByTestId("research-edge-detail")).toContainText("Methods section");

  await page.keyboard.press("Escape");
  await expect(modal.getByTestId("research-edge-detail")).toHaveCount(0);
  await expect(modal).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);

  await page.getByRole("button", { name: "Toggle panel" }).click();
  const rightPanel = page.locator(".rightpane");
  await rightPanel.getByRole("button", { name: "Add panel" }).click();
  await expect(rightPanel.locator(".rp-tab-add-menu")
    .getByRole("button", { name: /Research graph/ })).toHaveCount(0);
});

test("registered Artifact opens publication binding and Escape keeps Workspace open", async ({ page }) => {
  await enterApp(page, "/?mockPublication=draft");
  await page.locator('[data-session-id="publication-session"]').click();
  await page.getByRole("button", { name: "Toggle panel" }).click();

  const artifact = page.locator('.rp-tile[data-artifact-name="plddt_profile.png"]');
  await expect(artifact).toBeVisible();
  await artifact.getByRole("button", { name: "More" }).click();
  await page.locator(".rp-tile-menu").getByRole("button", { name: "Use in publication" }).click();

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("publication-binding-dialog")).toHaveCount(0);
  await expect(page.getByTestId("publication-workspace")).toBeVisible();
  await expect(page.getByTestId("publication-manuscript-tree")).toContainText("Figure 2B");
});

test("Run surface binds an exact publication evidence source", async ({ page }) => {
  await enterApp(page, "/?mockPublication=draft");
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();
  await page.locator(".context-card", { hasText: "ssh:gpu-server" })
    .getByRole("button", { name: "View runs" }).click();

  const runsModal = page.locator(".context-details-modal.runs-details");
  await expect(runsModal).toBeVisible();
  const run = page.locator(".run-card", { hasText: "Kinase screen QC" });
  await expect(run.locator(".run-use-publication")).toBeVisible();
  await run.getByRole("button", { name: "Use in publication" }).click();
  const dialog = page.getByTestId("publication-binding-dialog");
  await expect(dialog).toContainText("run-kinase-001");
  await dialog.locator("textarea").fill("Methods parameters and QC evidence");
  await dialog.getByRole("button", { name: "Bind exact evidence" }).click();

  await expect.poll(() => lastInvokeArgs(page, "bind_publication_evidence")).toMatchObject({
    input: {
      sourceKind: "run",
      sourceId: "run-kinase-001",
      purpose: "Methods parameters and QC evidence",
      selectionState: "selected",
      visibility: "public",
    },
  });
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("publication-workspace")).toContainText("run-kinase-001");
});

test("finished run offers server workspace cleanup and shows the cleaned state", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();
  await page.locator(".context-card", { hasText: "ssh:gpu-server" })
    .getByRole("button", { name: "View runs" }).click();

  const run = page.locator(".run-card", { hasText: "Kinase screen QC" });
  await expect(run).toBeVisible();
  // The running local run offers no cleanup; the finished SSH run does.
  await run.getByRole("button", { name: "Clean up server workspace" }).click();
  await expect.poll(() => lastInvokeArgs(page, "cleanup_run_workspace"))
    .toMatchObject({ runId: "run-kinase-001" });
  await expect(run.getByTestId("run-cleaned")).toHaveText("workspace cleaned");
  await expect(run.getByRole("button", { name: "Clean up server workspace" })).toHaveCount(0);
});

test("remote files view lists ledgered files and deletes retracted ones", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();
  await page.locator(".context-card", { hasText: "ssh:gpu-server" })
    .getByRole("button", { name: "Remote files" }).click();

  const pane = page.getByTestId("remote-files-pane");
  await expect(pane).toBeVisible();
  const rows = pane.getByTestId("remote-file-row");
  await expect(rows).toHaveCount(3);
  const active = rows.filter({ hasText: "plates.csv" });
  await expect(active).toContainText("active");
  // Active entries stay protected — no delete action.
  await expect(active.getByRole("button", { name: "Delete from server" })).toHaveCount(0);

  const replaced = rows.filter({ hasText: "matrix.tsv" }).filter({ hasText: "replaced" });
  await replaced.getByRole("button", { name: "Delete from server" }).click();
  await expect.poll(() => lastInvokeArgs(page, "remove_remote_files")).toMatchObject({
    contextId: "ssh:gpu-server",
    ids: ["stage-old-upload"],
  });
  await expect(rows).toHaveCount(2);

  // Escape closes only this modal; the Environment rail stays open.
  await page.keyboard.press("Escape");
  await expect(pane).toHaveCount(0);
  await expect(page.locator(".context-card", { hasText: "ssh:gpu-server" })).toBeVisible();
});

test("run review modal browses the workspace and downloads or deletes selections", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();
  await page.locator(".context-card", { hasText: "ssh:gpu-server" })
    .getByRole("button", { name: "View runs" }).click();

  const runCard = page.locator(".run-card", { hasText: "Kinase screen QC" });
  await runCard.getByTestId("run-review-open").click();
  const review = page.getByTestId("run-review-modal");
  await expect(review).toBeVisible();
  await expect(review.getByTestId("run-review-subtitle")).toHaveText("Kinase screen QC");
  const rows = review.getByTestId("run-review-row");
  await expect(rows).toHaveCount(2);
  await expect(rows.filter({ hasText: "results" })).toContainText("132481 files");
  const table = rows.filter({ hasText: "qc_table.tsv" });
  await expect(table.getByTestId("run-review-name")).toHaveText("qc_table.tsv");
  await expect(table.getByTestId("run-review-size")).toHaveText("2.0 KB");
  const nameBox = await table.getByTestId("run-review-name").boundingBox();
  const sizeBox = await table.getByTestId("run-review-size").boundingBox();
  expect(nameBox).not.toBeNull();
  expect(sizeBox).not.toBeNull();
  expect(nameBox!.x + nameBox!.width).toBeLessThan(sizeBox!.x);
  const cleanupBox = await review.getByRole("button", { name: "Clean entire workspace" }).boundingBox();
  expect(cleanupBox).not.toBeNull();
  expect(cleanupBox!.height).toBeLessThan(40);
  await expect(review.getByRole("button", { name: "Download selected" })).toBeDisabled();
  await review.getByTestId("run-review-select-all").check();
  await expect(review.getByTestId("run-review-count")).toHaveText("2 selected");
  await expect(review.getByRole("button", { name: "Download selected" })).toBeEnabled();
  await review.getByTestId("run-review-select-all").uncheck();

  // Escape immediately: one press closes only the review modal — the runs
  // modal underneath stays open.
  await page.keyboard.press("Escape");
  await expect(review).toHaveCount(0);
  await expect(page.locator(".context-details-modal.runs-details")).toBeVisible();

  // Reopen, drill into the directory and back, then download a selection.
  await runCard.getByTestId("run-review-open").click();
  await expect(review).toBeVisible();
  await review.getByRole("button", { name: "results" }).click();
  await expect(review.getByTestId("run-review-row")).toContainText("summary.tsv");
  await review.getByRole("button", { name: "Up" }).click();
  await expect(review.getByTestId("run-review-row")).toHaveCount(2);
  await rows.filter({ hasText: "qc_table.tsv" }).locator("input[type=checkbox]").check();
  await rows.filter({ hasText: "132481 files" }).locator("input[type=checkbox]").check();
  await review.getByRole("button", { name: "Download selected" }).click();
  await expect.poll(() => lastInvokeArgs(page, "download_run_files")).toMatchObject({
    runId: "run-kinase-001",
    files: ["qc_table.tsv"],
    dirs: ["results"],
  });
  await expect(review.getByTestId("run-review-status")).toContainText("downloaded");

  // Delete only the selected directory; the file entry survives.
  await rows.filter({ hasText: "132481 files" }).locator("input[type=checkbox]").check();
  await review.getByRole("button", { name: "Delete selected" }).click();
  await expect.poll(() => lastInvokeArgs(page, "delete_run_files")).toMatchObject({
    runId: "run-kinase-001",
    paths: ["results"],
  });
  await expect(review.getByTestId("run-review-row")).toHaveCount(1);

  // Whole-workspace cleanup is user-explicit (force) and closes the modal.
  await review.getByRole("button", { name: "Clean entire workspace" }).click();
  await expect.poll(() => lastInvokeArgs(page, "cleanup_run_workspace")).toMatchObject({
    runId: "run-kinase-001",
    force: true,
  });
  await expect(review).toHaveCount(0);
});

test("dropping a server audits abandoned remote files before removal", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Environments");

  const server = page.locator('.environment-settings-row[data-context-id="ssh:gpu-server"]');
  await server.locator(".settings-list-remove").click();
  const confirm = page.getByTestId("host-remove-confirm");
  await expect(confirm).toBeVisible();
  await expect(confirm.getByTestId("host-disposal-detail"))
    .toContainText("3 ledgered file(s)");
  // Cancel keeps the server.
  await confirm.getByRole("button", { name: "Cancel" }).click();
  await expect(server).toBeVisible();
  expect(await lastInvokeArgs(page, "remove_ssh_host")).toBeNull();

  // Confirming abandons the remote files and removes the host.
  await server.locator(".settings-list-remove").click();
  await page.getByTestId("host-remove-confirm")
    .getByRole("button", { name: "Remove server" }).click();
  await expect.poll(() => lastInvokeArgs(page, "remove_ssh_host"))
    .toMatchObject({ alias: "gpu-server" });
});

test("method-search Run reviews the frozen contract before start and exposes controls", async ({ page }) => {
  await enterApp(page, "/?mockMethodSearch=1");
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();
  await page.locator(".context-card", { hasText: "Local machine" })
    .getByRole("button", { name: "View runs" }).click();

  const card = page.locator(".run-card", { hasText: "Develop computational method" });
  await expect(card).toContainText("awaiting_approval");
  await card.getByTestId("method-search-inspect").click();
  const details = card.getByTestId("method-search-details");
  await expect(details).toContainText("analysis/model.py::fit_model");
  await expect(details).toContainText("0.537200");
  await expect(details).toContainText("Candidate reachability");
  await expect(details).toContainText("runtime_seconds lte 120");
  await expect(details.getByTestId("method-search-start")).toBeVisible();
  await expect(details.getByTestId("method-search-lineage")).toContainText("Candidate lineage (2)");
  await expect(details.getByTestId("method-search-outputs")).toContainText("selected_method");

  await details.getByTestId("method-search-start").click();
  await expect.poll(() => lastInvokeArgs(page, "start_method_search"))
    .toMatchObject({ runId: "method-search-001" });
  await expect(details.getByTestId("method-search-pause")).toBeVisible();
  await details.getByTestId("method-search-pause").click();
  await expect.poll(() => lastInvokeArgs(page, "pause_method_search"))
    .toMatchObject({ runId: "method-search-001" });
  await expect(details.getByTestId("method-search-resume")).toBeVisible();
  await details.getByTestId("method-search-resume").click();
  await expect.poll(() => lastInvokeArgs(page, "resume_method_search"))
    .toMatchObject({ runId: "method-search-001" });
  await details.getByTestId("method-search-cancel").click();
  await expect.poll(() => lastInvokeArgs(page, "cancel_method_search"))
    .toMatchObject({ runId: "method-search-001" });
  await expect(details).toContainText("Cancelled");
});

test("precise message evidence uses a stable locator and Escape closes only the top layer", async ({ page }) => {
  await enterApp(page, "/?mockPublication=draft");
  await page.locator(".sidebar").getByRole("button", { name: "Publication", exact: true }).click();

  const workspace = page.getByTestId("publication-workspace");
  await workspace.getByTestId("add-precise-publication-evidence").click();
  await expect(page.getByTestId("publication-anchor-dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("publication-anchor-dialog")).toHaveCount(0);
  await expect(workspace).toBeVisible();

  await workspace.getByTestId("add-precise-publication-evidence").click();
  const anchor = page.getByTestId("publication-anchor-dialog");
  await anchor.getByTestId("publication-anchor-frame").fill("publication-session");
  await anchor.getByLabel("Message sequence").fill("7");
  await anchor.getByLabel("UTF-8 byte start").fill("4");
  await anchor.getByLabel("UTF-8 byte end").fill("19");
  await anchor.getByTestId("publication-anchor-continue").click();

  const binding = page.getByTestId("publication-binding-dialog");
  await binding.locator("textarea").fill("Exact sentence supporting the main claim");
  await binding.getByRole("button", { name: "Bind exact evidence" }).click();
  await expect.poll(() => lastInvokeArgs(page, "bind_publication_evidence")).toMatchObject({
    input: {
      sourceKind: "message_span",
      purpose: "Exact sentence supporting the main claim",
    },
  });
  const args = await lastInvokeArgs(page, "bind_publication_evidence");
  expect(JSON.parse(args.input.sourceId)).toEqual({
    byte_end: 19,
    byte_start: 4,
    frame_id: "publication-session",
    message_seq: 7,
  });
});

test("Frozen Publication is read-only and exposes exact source plus late-capture readiness", async ({ page }) => {
  await enterApp(page, "/?mockPublication=frozen");
  await page.locator(".sidebar").getByRole("button", { name: "Publication", exact: true }).click();

  const workspace = page.getByTestId("publication-workspace");
  await expect(workspace).toBeVisible();
  await expect(workspace.getByTestId("publication-exact-source"))
    .toHaveText("artifact-version-late-v4");
  await expect(workspace).toContainText("historical_content_unverified");
  await expect(workspace).toContainText("Historical bytes were unavailable");
  await expect(workspace).toContainText("Late capture");
  await expect(workspace).toContainText("A newer result exists");
  await expect(workspace.getByRole("button", { name: "Freeze", exact: true })).toHaveCount(0);
  await expect(workspace.getByRole("button", { name: "Add item", exact: true })).toHaveCount(0);
  await expect(workspace.locator(".publication-binding-controls")).toHaveCount(0);
  expect(await invokeArgsList(page, "update_publication_evidence_binding")).toHaveLength(0);
});

test("Frozen Publication verifies a Run and surfaces environment plus comparator results", async ({ page }) => {
  await enterApp(page, "/?mockPublication=frozen");
  await page.locator(".sidebar").getByRole("button", { name: "Publication", exact: true }).click();

  const workspace = page.getByTestId("publication-workspace");
  await workspace.getByTestId("verify-publication-run").click();
  await expect.poll(() => lastInvokeArgs(page, "verify_publication_revision")).toMatchObject({
    input: {
      revisionId: "publication-revision-1",
      sourceRunId: "run-kinase-001",
      comparisons: [],
    },
  });
  const reports = workspace.getByTestId("publication-reproduction-runs");
  await expect(reports).toContainText("Environment matched");
  await expect(reports).toContainText("results/figure2b.png");
  await expect(reports).toContainText("sha256");
  await expect(workspace.locator(".publication-capability")).toContainText("Reproduced");
});

test("Frozen Publication builds a selective Capsule and shows its immutable hashes", async ({ page }) => {
  await enterApp(page, "/?mockPublication=frozen");
  await page.locator(".sidebar").getByRole("button", { name: "Publication", exact: true }).click();

  const workspace = page.getByTestId("publication-workspace");
  await workspace.getByTestId("build-publication-capsule").click();
  await expect.poll(() => lastInvokeArgs(page, "build_publication_capsule")).toEqual({
    revisionId: "publication-revision-1",
  });
  const builds = workspace.getByTestId("publication-capsule-builds");
  await expect(builds).toContainText("Succeeded");
  await expect(builds).toContainText("sha256:cccccccccccc");
  await expect(builds).toContainText("/exports/publication-capsule.zip");
});

test("right panel shows execution contexts and runs", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  const rightPanel = page.locator(".rightpane");
  await expect(rightPanel.locator(".rp-tab")).toHaveCount(4);
  await expect(rightPanel.getByRole("button", { name: "Artifacts", exact: true })).toBeVisible();
  await expect(rightPanel.getByRole("button", { name: "Agents", exact: true })).toBeVisible();
  await expect(rightPanel.getByRole("button", { name: "Files", exact: true })).toBeVisible();
  await expect(rightPanel.getByRole("button", { name: "Environment", exact: true })).toBeVisible();
  await expect(rightPanel.getByRole("button", { name: /^Notebook/ })).toHaveCount(0);
  await rightPanel.getByRole("button", { name: "Environment", exact: true }).click();

  await expect(page.locator(".context-card", { hasText: "local" })).toBeVisible();
  await expect(page.locator(".context-card", { hasText: "ssh:gpu-server" })).toContainText("NVIDIA A100");
  const sshContext = page.locator(".context-card", { hasText: "ssh:gpu-server" });
  // View runtimes uses runtime-panel icon, open terminal keeps terminal icon.
  const runtimeSvg = await sshContext.getByRole("button", { name: "View runtimes" }).locator("svg").innerHTML();
  const terminalSvg = await sshContext.getByRole("button", { name: "Open terminal" }).locator("svg").innerHTML();
  expect(runtimeSvg).not.toEqual(terminalSvg);
  await sshContext.getByRole("button", { name: "Probe context" }).click();
  await expect.poll(() => lastInvokeArgs(page, "probe_execution_context")).toMatchObject({
    contextId: "ssh:gpu-server",
  });
  await sshContext.getByRole("button", { name: "Open terminal" }).click();
  await expect.poll(() => lastInvokeArgs(page, "open_terminal")).toMatchObject({
    contextId: "ssh:gpu-server",
  });
  const terminalDock = page.getByTestId("terminal-dock");
  await expect(terminalDock).toBeVisible();
  await expect(terminalDock).toContainText("ssh:gpu-server — Terminal");
  await expect(terminalDock.locator("iframe")).toHaveCount(0);
  const firstTerminal = terminalDock.locator('.terminal-dock-frame[data-terminal-session="terminal-mock-1"]');
  await expect(firstTerminal).toHaveClass(/active/);
  await expect(firstTerminal.locator(".xterm-rows")).toContainText("terminal ready");
  await expect.poll(() => firstTerminal.locator(".xterm-viewport").evaluate((viewport) => ({
    standardWidth: getComputedStyle(viewport).scrollbarWidth,
    themedWidth: getComputedStyle(viewport, "::-webkit-scrollbar").width,
    thumbInset: getComputedStyle(viewport, "::-webkit-scrollbar-thumb").borderTopWidth,
    backgroundMatches: getComputedStyle(viewport).backgroundColor
      === getComputedStyle(viewport.closest(".terminal-dock-frame")!).backgroundColor,
  }))).toEqual({
    standardWidth: "auto",
    themedWidth: "10px",
    thumbInset: "2px",
    backgroundMatches: true,
  });
  await expect.poll(async () => (await invokeArgsList(page, "resize_terminal")).some((args: any) =>
    args.sessionId === "terminal-mock-1" && args.rows > 0 && args.cols > 0,
  )).toBe(true);

  await firstTerminal.click();
  await page.keyboard.type("echo hello");
  await expect.poll(async () => (await invokeArgsList(page, "write_terminal"))
    .filter((args: any) => args.sessionId === "terminal-mock-1")
    .map((args: any) => args.data)
    .join(""),
  ).toContain("echo hello");

  await terminalDock.getByRole("button", { name: "New terminal" }).click();
  await expect(terminalDock.locator(".terminal-dock-add-menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(terminalDock.locator(".terminal-dock-add-menu")).toHaveCount(0);
  await expect(terminalDock).toBeVisible();

  await terminalDock.getByRole("button", { name: "New terminal" }).click();
  await terminalDock.getByRole("button", { name: /Local machine/ }).click();
  await expect.poll(() => lastInvokeArgs(page, "open_terminal")).toMatchObject({
    contextId: "local",
  });
  await expect(terminalDock.getByRole("tab")).toHaveCount(2);
  await expect(terminalDock.locator(".terminal-dock-frame")).toHaveCount(2);
  await expect(terminalDock.locator(".terminal-dock-frame.active"))
    .toHaveAttribute("data-terminal-session", "terminal-mock-2");

  await terminalDock.getByRole("tab", { name: "ssh:gpu-server — Terminal" }).click();
  await expect(terminalDock.locator(".terminal-dock-frame.active"))
    .toHaveAttribute("data-terminal-session", "terminal-mock-1");
  await terminalDock.getByRole("button", { name: "Collapse terminal panel" }).click();
  await expect(terminalDock).toBeHidden();
  await sshContext.getByRole("button", { name: "Open terminal" }).click();
  await expect(terminalDock).toBeVisible();
  await expect(terminalDock.getByRole("tab")).toHaveCount(3);
  await expect(firstTerminal.locator(".xterm-rows")).toContainText("terminal ready");
  await expect(terminalDock.getByRole("button", { name: "Terminate", exact: true })).toHaveCount(0);
  await terminalDock.locator(".terminal-dock-tab.active")
    .getByRole("button", { name: "Close and terminate terminal" }).click();
  await expect.poll(() => lastInvokeArgs(page, "close_terminal")).toMatchObject({
    sessionId: "terminal-mock-3",
  });
  await expect(terminalDock.getByRole("tab")).toHaveCount(2);
  await expect(terminalDock.locator(".terminal-dock-frame")).toHaveCount(2);
  await expect(terminalDock.locator(".terminal-dock-frame.active"))
    .toHaveAttribute("data-terminal-session", "terminal-mock-2");
  await sshContext.getByRole("button", { name: "View runs" }).click();
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toContainText("succeeded");
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toContainText("ssh:gpu-server");
  await expect(page.locator(".run-card", { hasText: "Local normalization" })).toHaveCount(0);
  const remoteRun = page.locator(".run-card", { hasText: "Kinase screen QC" });
  await expect(remoteRun).toContainText("~/.wisp-science/runs/run-kinase-001");
  await remoteRun.getByText("Latest output").click();
  await expect(remoteRun).toContainText("wrote qc table");

  await page.getByRole("button", { name: "Refresh runs" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "list_runs").length,
  )).toBeGreaterThan(1);
});

test("active SSH transfer shows a live progress card and can be cancelled", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      frame_id: "s-complete",
      context_id: "ssh:gpu-server",
      title: "Upload DW14-2",
      kind: "ssh_direct",
      status: "submitted",
      progress_json: JSON.stringify({
        phase: "uploading",
        direction: "upload",
        completed_bytes: 3 * 1024 ** 3,
        total_bytes: 4 * 1024 ** 3,
        files_completed: 1,
        files_total: 2,
        current_file: "DW14-2_2.fq.gz",
        bytes_per_second: 64 * 1024 ** 2,
        eta_seconds: 16,
        updated_at: Math.floor(Date.now() / 1000),
      }),
    });
  });
  await page.getByTestId("recent-session-card").nth(1).click();
  await expect(newSessionButton(page)).toBeVisible();

  const card = page.locator('.transfer-card[data-run-id="run-local-002"]');
  await expect(card).toBeVisible();
  await expect(card).toContainText("Uploading");
  await expect(card).toContainText("DW14-2_2.fq.gz");
  await expect(card).toContainText("3.00 GB / 4.00 GB · 75%");
  await expect(card).toContainText("64.00 MB/s");
  await expect(card).toContainText("ETA 16s");

  await card.getByRole("button", { name: "Cancel run" }).click();
  await expect.poll(() => lastInvokeArgs(page, "cancel_run")).toMatchObject({
    runId: "run-local-002",
  });
});

test("completed SSH transfer cards leave the composer tray promptly", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      frame_id: "s-complete",
      context_id: "ssh:gpu-server",
      title: "Download result.csv",
      kind: "file_transfer",
      status: "running",
      ended_at: null,
      exit_code: null,
      progress_json: JSON.stringify({
        phase: "downloading",
        direction: "download",
        completed_bytes: 512,
        total_bytes: 1024,
        files_completed: 0,
        files_total: 1,
        current_file: "result.csv",
        bytes_per_second: 1024,
        eta_seconds: 1,
        updated_at: Math.floor(Date.now() / 1000),
      }),
    });
  });
  await page.getByTestId("recent-session-card").nth(1).click();
  await expect(newSessionButton(page)).toBeVisible();

  const card = page.locator('.transfer-card[data-run-id="run-local-002"]');
  await expect(card).toBeVisible();
  await expect(card).toContainText("Downloading");
  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      status: "succeeded",
      ended_at: Math.floor(Date.now() / 1000),
      exit_code: 0,
      progress_json: JSON.stringify({
        ...JSON.parse(run.progress_json),
        phase: "downloaded",
        completed_bytes: 1024,
        files_completed: 1,
        eta_seconds: null,
        updated_at: Math.floor(Date.now() / 1000),
      }),
    });
  });
  await expect(card).toContainText("Download complete");
  await expect(card).toBeHidden({ timeout: 5_000 });
  // Crossing another shared-clock tick must not remount an expired card after
  // the transfer tray has dropped its clock dependency.
  await page.waitForTimeout(1_200);
  await expect(card).toBeHidden();
});

test("monitor_run renders a live Run card from summary polls and on-demand detail", async ({ page }) => {
  await enterApp(page);
  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      context_id: "ssh:gpu-server",
      title: "Five-sample reseq pipeline",
      kind: "ssh_direct",
      status: "running",
      created_at: Math.floor(Date.now() / 1000) - 3_725,
      started_at: Math.floor(Date.now() / 1000) - 3_720,
      stdout_tail: "8 of 16 steps complete (50%)\nMapping sample D1",
      progress_json: "{}",
    });
  });

  await composer(page).fill("MONITORRUN");
  await page.getByRole("button", { name: "Send" }).click();

  const card = page.getByTestId("run-monitor-card");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Five-sample reseq pipeline");
  await expect(card).toContainText("Running");
  await expect(card).toContainText("Elapsed 1h");
  await expect(card).toContainText("8 of 16 steps complete (50%)");
  const reasoning = page.locator(".rz");
  await expect(reasoning).toContainText("thinking");
  await reasoning.locator("summary").click();
  await expect(reasoning).toContainText("Attach the existing Run monitor.");
  await expect(page.locator('.step-name:text-is("monitor_run")')).toHaveCount(0);

  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    run.stdout_tail = "9 of 16 steps complete (56%)\nMarking duplicates";
  });
  await expect(card).toContainText("9 of 16 steps complete (56%)", { timeout: 3_000 });

  await card.getByRole("button", { name: "Cancel run" }).click();
  await expect.poll(() => lastInvokeArgs(page, "cancel_run")).toMatchObject({
    runId: "run-local-002",
  });
  await expect(card).toContainText("Cancelled");
  await card.getByRole("button", { name: "Dismiss completed run card" }).click();
  await expect(card).toHaveCount(0);
  await expect(page.locator(".run-monitor-wrap")).toBeHidden();

  // Starting another turn remounts settled transcript rows. A manual
  // dismissal must survive that remount instead of flashing back until the
  // next run-list refresh.
  const pollsBeforeRemount = await runListPollCount(page);
  await composer(page).fill("continue after dismiss");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(card).toHaveCount(0);
  // The flash-back regression appeared on the run-list refresh after the
  // remount, so wait for that refresh to actually happen before re-checking.
  await expect
    .poll(() => runListPollCount(page), { timeout: 15_000 })
    .toBeGreaterThan(pollsBeforeRemount);
  await expect(card).toHaveCount(0);
});

test("review prompt waits for turn end and dismissal persists (#897)", async ({ page }) => {
  await enterApp(page);
  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      context_id: "ssh:gpu-server",
      title: "Genome assembly",
      kind: "ssh_direct",
      status: "running",
      created_at: Math.floor(Date.now() / 1000) - 300,
      started_at: Math.floor(Date.now() / 1000) - 295,
      remote_workdir: "~/.wisp-science/runs/run-local-002",
      remote_handle_json: '{"kind":"ssh_direct"}',
      progress_json: "{}",
    });
    (window as any).__mockRunWorkspaceFiles["run-local-002"] = {
      "": [{ path: "assembly.fasta", kind: "file", size_bytes: 4096, file_count: null }],
    };
  });

  await composer(page).fill("MONITORRUN");
  await page.getByRole("button", { name: "Send" }).click();
  const card = page.getByTestId("run-monitor-card");
  await expect(card).toBeVisible();

  // The run succeeds while the turn is still working: no interruption yet.
  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      status: "succeeded",
      ended_at: Math.floor(Date.now() / 1000),
      exit_code: 0,
    });
  });
  await expect(card).toContainText("Succeeded", { timeout: 7_000 });
  const review = page.getByTestId("run-review-modal");
  await expect(review).toHaveCount(0);

  // Turn ends: the prompt opens for the unresolved server-side files.
  await page.evaluate(() => (window as any).__finishMonitorRun());
  await expect(review).toBeVisible({ timeout: 7_000 });
  await expect(review).toContainText("assembly.fasta");

  // Escape immediately: one press closes the prompt and persists the
  // dismissal so this run never auto-prompts again.
  await page.keyboard.press("Escape");
  await expect(review).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "dismiss_run_review")).toMatchObject({
    runId: "run-local-002",
  });
});

test("unmonitored exploratory run success never auto-opens the review prompt (#897)", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => {
    const now = Math.floor(Date.now() / 1000);
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      frame_id: "s-complete",
      context_id: "ssh:gpu-server",
      title: "Quick check",
      kind: "ssh_direct",
      status: "running",
      created_at: now - 30,
      started_at: now - 29,
      remote_workdir: "~/.wisp-science/runs/run-local-002",
      remote_handle_json: '{"kind":"ssh_direct"}',
    });
    (window as any).__mockRunWorkspaceFiles["run-local-002"] = {
      "": [{ path: "notes.txt", kind: "file", size_bytes: 128, file_count: null }],
    };
  });

  await page.getByTestId("recent-session-card").nth(1).click();
  const automatic = page.getByTestId("auto-run-monitor");
  await expect(automatic).toBeVisible();

  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      status: "succeeded",
      ended_at: Math.floor(Date.now() / 1000),
      exit_code: 0,
    });
  });
  await expect(automatic).toContainText("Succeeded", { timeout: 7_000 });
  // Exploratory command runs never nominate themselves for review, even with
  // files present and the session idle: cleanup stays on the manual entry
  // points and retention.
  await expect(page.getByTestId("run-review-modal")).toHaveCount(0);
});

test("run monitor output stays pinned to the tail across poll rebuilds (#654)", async ({ page }) => {
  // Eight long logical lines wrap far past the 150px pre, so it scrolls.
  const longOutput = (tag: string) =>
    Array.from({ length: 8 }, (_, index) => `${tag} line ${index} ` + "x".repeat(180)).join("\n");
  const setOutput = (tag: string) =>
    page.evaluate((stdout) => {
      const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
      run.stdout_tail = stdout;
    }, longOutput(tag));
  const bottomGap = () =>
    page
      .locator('[data-run-id="run-local-002"] .run-monitor-output pre')
      .evaluate((el) => el.scrollHeight - el.clientHeight - el.scrollTop);

  await enterApp(page);
  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      context_id: "local",
      title: "Scrolling pipeline",
      kind: "local",
      status: "running",
      created_at: Math.floor(Date.now() / 1000) - 30,
      started_at: Math.floor(Date.now() / 1000) - 29,
      // Non-empty progress keeps the one-second run refresh active.
      progress_json: "{\"tick\":1}",
    });
  });
  await setOutput("batch-1");

  await composer(page).fill("MONITORRUN");
  await page.getByRole("button", { name: "Send" }).click();

  const output = page.locator('[data-run-id="run-local-002"] .run-monitor-output pre');
  await expect(output).toBeVisible();
  await expect.poll(bottomGap, { timeout: 5_000 }).toBeLessThanOrEqual(2);

  // Fresh output on the next poll rebuilds the card; the panel must re-pin.
  await setOutput("batch-2");
  await expect(output).toContainText("batch-2");
  await expect.poll(bottomGap, { timeout: 5_000 }).toBeLessThanOrEqual(2);

  // A scrolled-up user keeps their place instead of being yanked back down.
  await output.evaluate((el) => {
    el.scrollTop = 0;
  });
  const pollsBeforeBatch3 = await runListPollCount(page);
  await setOutput("batch-3");
  await expect(output).toContainText("batch-3");
  // The yank would happen on a poll rebuild after the content update, so wait
  // for another refresh to complete before asserting the scroll position held.
  await expect
    .poll(() => runListPollCount(page), { timeout: 10_000 })
    .toBeGreaterThan(pollsBeforeBatch3 + 1);
  expect(await output.evaluate((el) => el.scrollTop)).toBeLessThanOrEqual(2);

  // Scrolling back to the bottom re-engages follow.
  await output.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });
  await setOutput("batch-4");
  await expect(output).toContainText("batch-4");
  await expect.poll(bottomGap, { timeout: 5_000 }).toBeLessThanOrEqual(2);

  // Leave the run settled so its one-second refresh stops with this page.
  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, { status: "succeeded", ended_at: Math.floor(Date.now() / 1000), exit_code: 0 });
  });
});

test("a settled run card stops rebuilding itself on every poll (#654)", async ({ page }) => {
  const longOutput = Array.from(
    { length: 8 },
    (_, index) => `settled line ${index} ` + "x".repeat(180),
  ).join("\n");

  await enterApp(page);
  // The MONITORRUN turn never resolves, so the agent stays busy and the run
  // list is polled once a second even though this run is already finished.
  await page.evaluate((stdout) => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      context_id: "local",
      title: "Settled pipeline",
      kind: "local",
      status: "failed",
      created_at: Math.floor(Date.now() / 1000) - 30,
      started_at: Math.floor(Date.now() / 1000) - 29,
      ended_at: Math.floor(Date.now() / 1000) - 5,
      exit_code: 1,
      stdout_tail: stdout,
      progress_json: "{}",
    });
  }, longOutput);

  await composer(page).fill("MONITORRUN");
  await page.getByRole("button", { name: "Send" }).click();

  const output = page.locator('[data-run-id="run-local-002"] .run-monitor-output pre');
  await expect(output).toBeVisible();
  const bottomGap = () =>
    output.evaluate((el) => el.scrollHeight - el.clientHeight - el.scrollTop);
  await expect.poll(bottomGap, { timeout: 5_000 }).toBeLessThanOrEqual(2);

  // Tag the live node. Identical poll results must leave it in place: replacing
  // it is what reset the panel to its top edge for a frame, once per second.
  const pollsBeforeProbe = await runListPollCount(page);
  await output.evaluate((el) => {
    (el as any).__stableProbe = true;
  });
  // Wait for several identical poll results to come back, then verify none of
  // them replaced the tagged node.
  await expect
    .poll(() => runListPollCount(page), { timeout: 15_000 })
    .toBeGreaterThanOrEqual(pollsBeforeProbe + 3);
  expect(await output.evaluate((el) => (el as any).__stableProbe === true)).toBe(true);
  expect(await bottomGap()).toBeLessThanOrEqual(2);
});

test("reasoning details stays open while more thinking streams in", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("RZSTREAM");
  await page.getByRole("button", { name: "Send" }).click();

  const rz = page.locator("details.rz");
  await expect(rz).toBeVisible();
  await rz.locator("summary").click();
  await expect(rz).toHaveAttribute("open", "open");
  await rz.evaluate((element) => {
    (element as any).__stableProbe = true;
  });

  // The next streaming delta updates the body without replacing the live row.
  await expect(rz).toContainText("More reasoning arrives.");
  await expect(rz).toHaveAttribute("open", "open");
  await expect(rz).toContainText("First thought.");
  expect(await rz.evaluate((element) => (element as any).__stableProbe === true)).toBe(true);
});

test("active session Runs appear automatically with elapsed time and heartbeat (#593)", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => {
    const now = Math.floor(Date.now() / 1000);
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      frame_id: "s-complete",
      title: "TF activity pipeline",
      status: "running",
      created_at: now - 95,
      started_at: now - 90,
      last_polled_at: now - 3,
      stdout_tail: "Loading regulons\n2 of 3 stages complete",
    });
  });

  await page.getByTestId("recent-session-card").nth(1).click();
  const automatic = page.getByTestId("auto-run-monitor");
  await expect(automatic).toBeVisible();
  await expect(automatic).toContainText("TF activity pipeline");
  await expect(automatic).toContainText("Elapsed 1m");
  await expect(automatic).toContainText("Heartbeat");
  await expect(automatic).toContainText("2 of 3 stages complete");
  await expect(automatic.getByRole("button", { name: "Dismiss completed run card" }))
    .toHaveCount(0);

  await composer(page).fill("MDLIST later turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.locator(".msg.user", { hasText: "MDLIST later turn" })).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const card = document.querySelector('[data-testid="auto-run-monitor"]');
    const laterTurn = [...document.querySelectorAll(".msg.user")]
      .find((element) => element.textContent?.includes("MDLIST later turn"));
    return !!card && !!laterTurn
      && !!(card.compareDocumentPosition(laterTurn) & Node.DOCUMENT_POSITION_FOLLOWING);
  })).toBe(true);

  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-local-002");
    Object.assign(run, {
      status: "succeeded",
      ended_at: Math.floor(Date.now() / 1000),
      exit_code: 0,
    });
  });
  const dismiss = automatic.getByRole("button", { name: "Dismiss completed run card" });
  await expect(dismiss).toBeVisible({ timeout: 7_000 });
  await dismiss.click();
  await expect(automatic.getByTestId("run-monitor-card")).toHaveCount(0);
  await expect(automatic).toBeHidden();
});

test("active Run elapsed time advances without waiting for a backend refresh (#663)", async ({ page }) => {
  await page.goto("/?mockLiveRunClock=1");
  await page.getByTestId("recent-session-card").nth(1).click();

  const card = page.getByTestId("auto-run-monitor").locator(".run-monitor-card");
  const meta = card.locator(".run-monitor-meta");
  const elapsed = async () => (await meta.textContent())?.match(/Elapsed ([^·]+)/)?.[1].trim();
  await expect.poll(elapsed).toMatch(/\d+s/);
  await card.evaluate((element) => ((element as any).__clockStableProbe = true));
  const initial = await elapsed();
  await expect.poll(elapsed, { timeout: 3_000 }).not.toBe(initial);
  expect(await card.evaluate((element) => (element as any).__clockStableProbe === true)).toBe(true);
});

test("image generation shows a placeholder and replaces it with the PNG", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("IMAGEGENPLACEHOLDER");
  await page.getByRole("button", { name: "Send" }).click();

  const card = page.getByTestId("image-generation-card");
  await expect(card).toHaveAttribute("data-status", "running");
  await expect(card.locator(".image-generation-spinner")).toBeVisible();
  await expect(card.locator("img")).toHaveCount(0);
  await expect(card).toContainText("figures/pathway.png");
  await expect(page.locator('.step-name:text-is("generate_image")')).toHaveCount(0);

  await expect(card).toHaveAttribute("data-status", "completed", { timeout: 3_000 });
  const image = card.locator("img");
  await expect(image).toBeVisible();
  await expect(image).toHaveAttribute("src", /^blob:/);
  await expect(card.locator(".image-generation-spinner")).toHaveCount(0);
});

test("video generation shows a placeholder and replaces it with the MP4", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("VIDEOGENPLACEHOLDER");
  await page.getByRole("button", { name: "Send" }).click();

  const card = page.getByTestId("video-generation-card");
  await expect(card).toHaveAttribute("data-status", "running");
  await expect(card.locator(".video-generation-spinner")).toBeVisible();
  await expect(card.locator("video")).toHaveCount(0);
  await expect(card).toContainText("media/demo.mp4");
  await expect(page.locator('.step-name:text-is("generate_video")')).toHaveCount(0);

  await expect(card).toHaveAttribute("data-status", "completed", { timeout: 3_000 });
  const video = card.locator("video");
  await expect(video).toBeVisible();
  await expect(video).toHaveAttribute("src", /^blob:/);
  await expect(video).toHaveAttribute("preload", "metadata");
  await expect(card.locator(".video-generation-spinner")).toHaveCount(0);
});

test("SSH failures show that automatic retry was stopped", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();

  await page.evaluate(() => {
    const context = (window as any).__mockExecutionContexts.find(
      (item: any) => item.id === "ssh:gpu-server",
    );
    context.last_probe_status = "error";
    context.last_probe_error = "Permission denied (publickey).";
  });
  const remote = page.locator(".context-card", { hasText: "ssh:gpu-server" });
  await remote.getByRole("button", { name: "Probe context" }).click();
  await expect(page.locator(".copy-toast-warning")).toHaveText(
    "SSH probe failed. Automatic retry was stopped to protect the server; check the connection and retry manually.",
  );
  await expect(page.locator(".copy-toast-warning")).toBeHidden({ timeout: 3_000 });

  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-kinase-001");
    run.status = "failed";
    run.exit_code = 69;
    run.last_poll_error =
      "SSH automatic retry stopped after the first failed attempt to protect the server. Manual retry is required. Connection reset by peer.";
  });
  await remote.getByRole("button", { name: "View runs" }).click();
  await page.getByRole("dialog", { name: "Runs" })
    .getByRole("button", { name: "Refresh runs" }).click();
  await expect(page.locator(".copy-toast-warning")).toHaveText(
    "SSH failed. Automatic retry was stopped to protect the server; check the connection and retry manually.",
  );
});

test("context cards open machine, runtime, and runs details in modals", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();

  await expect(page.locator(".context-detail-pane")).toHaveCount(0);
  await expect(page.locator(".runtime-card")).toHaveCount(0);
  await expect(page.locator(".run-card")).toHaveCount(0);
  const remote = page.locator(".context-card", { hasText: "ssh:gpu-server" });
  await remote.locator(".context-card-select").click();
  await expect(page.getByRole("dialog", { name: "Machine information" })).toContainText("gpu-server");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Machine information" })).toHaveCount(0);
  await expect(page.locator(".rightpane")).toBeVisible();

  await remote.getByRole("button", { name: "View runtimes" }).click();
  const runtimeDialog = page.getByRole("dialog", { name: "Runtimes" });
  await expect(runtimeDialog).toBeVisible();
  await expect(page.locator('.runtime-card[data-runtime-context="ssh:gpu-server"]')).toHaveCount(2);
  await runtimeDialog.evaluate((dialog) => dialog.setAttribute("data-refresh-stable", "true"));
  await runtimeDialog.getByRole("button", { name: "Refresh runtimes" }).click();
  await expect(runtimeDialog).toHaveAttribute("data-refresh-stable", "true");
  await page.getByRole("button", { name: "Close details" }).click();

  await remote.getByRole("button", { name: "View runs" }).click();
  const runsDialog = page.getByRole("dialog", { name: "Runs" });
  await expect(runsDialog).toBeVisible();
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toBeVisible();
  await runsDialog.evaluate((dialog) => dialog.setAttribute("data-refresh-stable", "true"));
  await runsDialog.getByRole("button", { name: "Refresh runs" }).click();
  await expect(runsDialog).toHaveAttribute("data-refresh-stable", "true");
});

test("execution contexts remember Python and R interpreter paths", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();

  const remote = page.locator(".context-card", { hasText: "ssh:gpu-server" });
  await remote.getByRole("button", { name: "Configure runtime interpreters" }).click();
  const runtimeModal = page.locator(".runtime-config-modal");
  await expect.poll(() => runtimeModal.locator(".ps-close").evaluate((button) => ({
    headDisplay: getComputedStyle(button.parentElement!).display,
    buttonDisplay: getComputedStyle(button).display,
    width: getComputedStyle(button).width,
    border: getComputedStyle(button).borderTopWidth,
  }))).toEqual({ headDisplay: "flex", buttonDisplay: "flex", width: "30px", border: "0px" });
  const python = page.locator("#runtime-python-executable");
  const rscript = page.locator("#runtime-rscript-executable");
  const pastedPython = String.raw`C:\Tools\Python\python.exe`;
  await runtimeModal.evaluate((modal) => modal.setAttribute("data-paste-stable", "true"));
  await python.evaluate((element, value) => {
    const input = element as HTMLInputElement;
    const clipboard = new DataTransfer();
    clipboard.setData("text/plain", value);
    input.focus();
    input.dispatchEvent(new ClipboardEvent("paste", {
      bubbles: true,
      cancelable: true,
      clipboardData: clipboard,
    }));
    input.value = value;
    input.dispatchEvent(new InputEvent("input", {
      bubbles: true,
      data: value,
      inputType: "insertFromPaste",
    }));
  }, pastedPython);
  await expect(python).toHaveValue(pastedPython);
  await expect(python).toBeFocused();
  await expect(runtimeModal).toHaveAttribute("data-paste-stable", "true");
  await rscript.fill(String.raw`C:\Program Files\R\R-4.5.2\bin\Rscript.exe`);
  await page.getByRole("button", { name: "Save", exact: true }).click();

  await expect.poll(() => lastInvokeArgs(page, "update_execution_context_interpreters")).toMatchObject({
    contextId: "ssh:gpu-server",
    pythonExecutable: String.raw`C:\Tools\Python\python.exe`,
    rscriptExecutable: String.raw`C:\Program Files\R\R-4.5.2\bin\Rscript.exe`,
  });
  await expect(page.getByRole("heading", { name: "Runtime interpreters" })).toBeHidden();

  await remote.getByRole("button", { name: "Configure runtime interpreters" }).click();
  await expect(python).toHaveValue(String.raw`C:\Tools\Python\python.exe`);
  await expect(rscript).toHaveValue(String.raw`C:\Program Files\R\R-4.5.2\bin\Rscript.exe`);
});

test("runtime panel shows lifecycle state and controls start stop restart", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();

  await expect(page.locator(".runtime-card")).toHaveCount(0);
  await page.locator(".context-card", { hasText: "ssh:gpu-server" }).getByRole("button", { name: "View runtimes" }).click();

  const localPython = page.locator('.runtime-card[data-runtime-language="python"][data-runtime-context="local"]');
  const localR = page.locator('.runtime-card[data-runtime-language="r"][data-runtime-context="local"]');
  const remotePython = page.locator('.runtime-card[data-runtime-language="python"][data-runtime-context="ssh:gpu-server"]');
  const remoteR = page.locator('.runtime-card[data-runtime-language="r"][data-runtime-context="ssh:gpu-server"]');

  await expect(localPython).toHaveCount(0);
  await expect(localR).toHaveCount(0);
  await expect(remotePython).toContainText("Busy");
  await expect(remotePython).toContainText("10.0 GB");
  await expect(remoteR).toContainText("Not started");

  await remoteR.getByRole("button", { name: "Configure path" }).click();
  await page.locator("#runtime-rscript-executable").fill("/data/apps/R/4.5/bin/Rscript");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect.poll(() => lastInvokeArgs(page, "update_execution_context_interpreters")).toMatchObject({
    contextId: "ssh:gpu-server",
    rscriptExecutable: "/data/apps/R/4.5/bin/Rscript",
  });

  await remoteR.getByRole("button", { name: "Start" }).click();
  await expect(remoteR).toContainText("Ready");
  await expect.poll(() => lastInvokeArgs(page, "start_runtime")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "r",
  });
});

test("runtime inspector lists object metadata without loading object contents", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();
  await page.locator(".context-card", { hasText: "ssh:gpu-server" }).getByRole("button", { name: "View runtimes" }).click();

  const runtime = page.locator('.runtime-card[data-runtime-language="python"][data-runtime-context="ssh:gpu-server"]');
  await runtime.getByRole("button", { name: "Stop" }).click();
  await runtime.getByRole("button", { name: "Start" }).click();
  await runtime.getByRole("button", { name: "View Python environment" }).click();

  const environment = page.getByRole("region", { name: "Python Environment" });
  await expect(environment).toBeVisible();
  const runtimeDialog = page.getByRole("dialog", { name: "Runtimes" });
  const runtimeList = runtimeDialog.locator(".context-modal-section");
  await expect.poll(async () => {
    const [listBox, environmentBox] = await Promise.all([
      runtimeList.boundingBox(),
      environment.boundingBox(),
    ]);
    return listBox && environmentBox
      ? Math.round(environmentBox.x - listBox.x - listBox.width)
      : -1;
  }).toBeGreaterThan(0);
  await expect(environment.locator(".runtime-environment-row", { hasText: "counts" })).toContainText("DataFrame");
  await expect(environment.locator(".runtime-environment-row", { hasText: "counts" })).toContainText("12000000 × 48");
  await expect(environment.locator(".runtime-environment-row", { hasText: "counts" })).toContainText("4.0 GB");
  await expect(environment.locator(".runtime-environment-row", { hasText: "model" })).toContainText("RandomForestClassifier");
  await expect.poll(() => lastInvokeArgs(page, "inspect_runtime")).toMatchObject({
    projectId: "default",
    contextId: "ssh:gpu-server",
    language: "python",
  });

  await environment.getByRole("button", { name: "Close runtime environment" }).click();
  const rRuntime = page.locator('.runtime-card[data-runtime-language="r"][data-runtime-context="ssh:gpu-server"]');
  await rRuntime.getByRole("button", { name: "Start" }).click();
  await rRuntime.getByRole("button", { name: "View R environment" }).click();
  const rEnvironment = page.getByRole("region", { name: "R Environment" });
  await expect(rEnvironment).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "inspect_runtime")).toMatchObject({
    projectId: "default",
    contextId: "ssh:gpu-server",
    language: "r",
  });

  await rEnvironment.getByRole("button", { name: "Pin environment to conversation" }).click();
  await expect(runtimeDialog).toHaveCount(0);
  await expect(rEnvironment).toBeVisible();
  await expect(rEnvironment.getByRole("button", { name: "Unpin environment" }))
    .toHaveAttribute("aria-pressed", "true");

  const beforeDrag = await rEnvironment.boundingBox();
  const dragHandle = rEnvironment.locator(".runtime-environment-title");
  const dragBox = await dragHandle.boundingBox();
  expect(dragBox).not.toBeNull();
  const startX = dragBox!.x + dragBox!.width / 2;
  const startY = dragBox!.y + dragBox!.height / 2;
  // Dispatch each pointer phase separately and wait for the drag state before
  // moving. Sending all three events in one evaluate callback lets Leptos batch
  // the state transitions on a busy CI worker, so the move can observe no
  // active drag even though the same sequence passes locally.
  await dragHandle.dispatchEvent("pointerdown", {
    button: 0,
    pointerId: 7,
    clientX: startX,
    clientY: startY,
  });
  await expect(rEnvironment).toHaveClass(/is-dragging/);
  // Re-dispatch the move inside the poll: a single pointermove right after the
  // drag state flips can still be batched away by Leptos on a busy CI worker,
  // leaving the panel unmoved even though is-dragging is set.
  await expect.poll(async () => {
    await dragHandle.dispatchEvent("pointermove", {
      buttons: 1,
      pointerId: 7,
      clientX: startX - 120,
      clientY: startY + 48,
    });
    const afterDrag = await rEnvironment.boundingBox();
    return beforeDrag && afterDrag ? Math.round(beforeDrag.x - afterDrag.x) : 0;
  }).toBeGreaterThan(100);
  await expect.poll(async () => {
    const afterDrag = await rEnvironment.boundingBox();
    return beforeDrag && afterDrag ? Math.round(afterDrag.y - beforeDrag.y) : 0;
  }).toBeGreaterThan(30);
  await dragHandle.dispatchEvent("pointerup", {
    button: 0,
    pointerId: 7,
    clientX: startX - 120,
    clientY: startY + 48,
  });
  await expect(rEnvironment).not.toHaveClass(/is-dragging/);

  await page.keyboard.press("Escape");
  await expect(rEnvironment).toHaveCount(0);
  await expect(page.locator(".rightpane")).toBeVisible();
});

test("pinned runtime environment stays on screen after the window shrinks", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await enterApp(page);
  await page.getByTestId("session-runtime-strip")
    .locator('[data-runtime-language="r"][data-runtime-context="local"]')
    .click();
  const environment = page.getByRole("region", { name: "R Environment" });
  await expect(environment).toBeVisible();
  await environment.getByRole("button", { name: "Pin environment to conversation" }).click();
  await expect(environment).toHaveClass(/is-pinned/);
  await expectInsideViewport(environment, 1600, 900);

  // Restoring a maximized window used to leave the pin at the old right-edge
  // coordinates, so it vanished until the window was maximized again.
  await page.setViewportSize({ width: 900, height: 640 });
  await expect(environment).toBeVisible();
  await expect.poll(async () => {
    const box = await environment.boundingBox();
    if (!box) return false;
    return box.x >= 0
      && box.y >= 0
      && box.x + box.width <= 900
      && box.y + box.height <= 640;
  }).toBe(true);

  await page.setViewportSize({ width: 1600, height: 900 });
  await expect(environment).toBeVisible();
  await expectInsideViewport(environment, 1600, 900);
});

test("conversation runtime strip shows bound servers and opens R/Python environments", async ({ page }) => {
  await enterApp(page);
  const strip = page.getByTestId("session-runtime-strip");
  await expect(strip).toBeVisible();
  const local = strip.locator('[data-testid="session-runtime-group"][data-runtime-context="local"]');
  await expect(local.getByTestId("session-runtime-chip")).toHaveCount(2);
  await expect(local.locator('[data-runtime-language="python"]')).toContainText("Ready");
  await expect(local.locator('[data-runtime-language="r"]')).toContainText("Dead");
  await expect(strip.locator('[data-testid="session-runtime-group"][data-runtime-context="ssh:gpu-server"]')).toHaveCount(0);

  await selectRemoteContext(page);
  const remote = strip.locator('[data-testid="session-runtime-group"][data-runtime-context="ssh:gpu-server"]');
  await expect(remote).toBeVisible();
  await expect(remote.locator('[data-runtime-language="python"]')).toContainText("Busy");

  await local.locator('[data-runtime-language="python"]').click();
  const dialog = page.getByRole("dialog", { name: "Runtimes" });
  await expect(dialog).toBeVisible();
  await expect(page.getByRole("region", { name: "Python Environment" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(strip).toBeVisible();

  await local.locator('[data-runtime-language="r"]').click();
  await expect(page.getByRole("dialog", { name: "Runtimes" })).toBeVisible();
  await expect(page.getByRole("region", { name: "R Environment" })).toBeVisible();
});

test("an agent r cell refreshes the open R memory environment without manual sync", async ({ page }) => {
  await enterApp(page);
  const strip = page.getByTestId("session-runtime-strip");
  const rChip = strip.locator('[data-runtime-language="r"][data-runtime-context="local"]');
  await expect(rChip).toContainText("Dead");
  await rChip.click();
  const environment = page.getByRole("region", { name: "R Environment" });
  await expect(environment).toBeVisible();
  // Dead runtime: nothing to inspect yet, so the table stays empty.
  await expect(environment.locator(".runtime-environment-row")).toHaveCount(0);
  const inspectsBeforeTurn = await invokeCount(page, "inspect_runtime");

  // Pin the panel so it floats next to the conversation, then let the agent
  // run an R cell that lazily restarts the runtime.
  await environment.getByRole("button", { name: "Pin environment to conversation" }).click();
  await composer(page).fill("RAUTOREFRESH seed the session");
  await page.getByRole("button", { name: "Send" }).click();

  // The finished tool call alone must refresh the status chip and re-inspect
  // the open panel — no manual sync click.
  await expect(rChip).toContainText("Ready");
  await expect(environment.locator(".runtime-environment-row", { hasText: "counts" })).toBeVisible();
  expect(await invokeCount(page, "inspect_runtime")).toBeGreaterThan(inspectsBeforeTurn);
  await expect.poll(() => lastInvokeArgs(page, "inspect_runtime")).toMatchObject({
    projectId: "default",
    contextId: "local",
    language: "r",
  });
});

test("runtime environment switches language and filters objects", async ({ page }) => {
  await enterApp(page);
  await page.getByTestId("session-runtime-strip")
    .locator('[data-runtime-language="python"][data-runtime-context="local"]')
    .click();
  const environment = page.getByRole("region", { name: "Python Environment" });
  await expect(environment).toBeVisible();
  await expect(environment.getByTestId("runtime-environment-lang")).toHaveCount(2);
  await environment.getByTestId("runtime-object-filter").fill("model");
  await expect(environment.locator(".runtime-environment-row", { hasText: "model" })).toBeVisible();
  await expect(environment.locator(".runtime-environment-row", { hasText: "counts" })).toHaveCount(0);
  await environment.getByRole("tab", { name: "Show R environment" }).click();
  await expect(page.getByRole("region", { name: "R Environment" })).toBeVisible();
});

test("Windows environment settings imports installed WSL distributions", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "userAgent", {
      configurable: true,
      value: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
    });
  });
  await enterApp(page);
  await openSettingsSection(page, "Environments");

  await page.getByRole("button", { name: "Import WSL" }).click();

  await expect.poll(() => lastInvokeArgs(page, "import_wsl_contexts")).not.toBeNull();
});

test("environment panel shows runs only for the selected context", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Environment", exact: true }).click();
  await page.locator(".context-card", { hasText: "ssh:gpu-server" }).getByRole("button", { name: "View runs" }).click();
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toBeVisible();
  await expect(page.locator(".run-card", { hasText: "Local normalization" })).toHaveCount(0);
});

test("clicking a figure opens the artifact modal with provenance", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await enterApp(page);
  // A file path in the user turn is collected as an artifact; a .png name maps
  // to the "image" kind, which opens directly in the modal viewer on click.
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  // Clicking an image artifact opens the modal viewer directly (no expand step).
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  await expect(page.locator(".artifact-modal")).toBeVisible();
  const overlay = page.locator(".overlay", { has: page.locator(".artifact-modal") });
  await expect.poll(async () => overlay.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      top: Math.round(rect.top),
      left: Math.round(rect.left),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    };
  })).toEqual({ top: 0, left: 0, width: 1280, height: 720 });
  await expect.poll(() => page.evaluate(() =>
    document.elementFromPoint(innerWidth - 4, innerHeight / 2)?.closest(".overlay") !== null,
  )).toBe(true);
  const artifactModal = page.locator(".artifact-modal");
  await artifactModal.evaluate(async (el) => {
    await Promise.all(el.getAnimations().map((animation) => animation.finished));
  });
  const modalBoundsAt100 = await artifactModal.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    };
  });
  const modalFigure = page.locator(".artifact-modal .am-figure");
  const figureHeightAt100 = await modalFigure.evaluate((el) =>
    Math.round(el.getBoundingClientRect().height),
  );
  const modalImage = page.locator(".artifact-modal .rp-img");
  const modalWidthAt100 = await modalImage.evaluate((el) => el.getBoundingClientRect().width);
  for (let i = 0; i < 3; i += 1) {
    await page.getByRole("button", { name: "Zoom out" }).click();
  }
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("25%");
  const modalBoundsAt25 = await artifactModal.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    };
  });
  expect(Math.abs(modalBoundsAt25.width - modalBoundsAt100.width)).toBeLessThanOrEqual(12);
  expect(Math.abs(modalBoundsAt25.height - modalBoundsAt100.height)).toBeLessThanOrEqual(12);
  await expect.poll(async () => Math.abs(
    await modalFigure.evaluate((el) => Math.round(el.getBoundingClientRect().height))
      - figureHeightAt100,
  )).toBeLessThanOrEqual(12);
  await page.getByRole("button", { name: "Reset zoom" }).click();
  for (let i = 0; i < 8; i += 1) {
    await page.getByRole("button", { name: "Zoom in" }).click();
  }
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("300%");
  await expect.poll(() => modalImage.evaluate((el) => el.getBoundingClientRect().width))
    .toBeGreaterThan(modalWidthAt100);
  await expect.poll(() => artifactModal.evaluate((el) =>
    Math.round(el.getBoundingClientRect().width),
  )).toBeGreaterThan(0);
  const modalBoundsAt300 = await artifactModal.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    };
  });
  expect(Math.abs(modalBoundsAt300.width - modalBoundsAt100.width)).toBeLessThanOrEqual(12);
  expect(Math.abs(modalBoundsAt300.height - modalBoundsAt100.height)).toBeLessThanOrEqual(12);
  const modalViewport = page.locator(".artifact-modal .file-preview-zoom-viewport");
  await modalViewport.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    const y = rect.top + rect.height * 0.5;
    const startX = rect.left + rect.width * 0.7;
    const endX = rect.left + rect.width * 0.25;
    el.dispatchEvent(new PointerEvent("pointerdown", {
      bubbles: true,
      button: 0,
      pointerId: 1,
      clientX: startX,
      clientY: y,
    }));
    el.dispatchEvent(new PointerEvent("pointermove", {
      bubbles: true,
      button: 0,
      buttons: 1,
      pointerId: 1,
      clientX: endX,
      clientY: y,
    }));
    el.dispatchEvent(new PointerEvent("pointerup", {
      bubbles: true,
      button: 0,
      pointerId: 1,
      clientX: endX,
      clientY: y,
    }));
  });
  await expect.poll(() => modalViewport.evaluate((el) => el.scrollLeft)).toBeGreaterThan(0);
  await page.getByRole("button", { name: "Reset zoom" }).click();
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("100%");
  // Code tab renders the recorded source (from get_artifact_provenance).
  await page.locator(".am-tab", { hasText: "Code" }).click();
  await expect(page.locator(".artifact-modal")).toContainText("savefig");
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, "writeText", {
      configurable: true,
      value: async (text: string) => { (window as any).__copiedProvenanceCode = text; },
    });
  });
  await artifactModal.getByRole("button", { name: "Copy code" }).click();
  await expect(page.locator(".copy-toast")).toHaveText("Copied");
  await expect.poll(() => page.evaluate(() => (window as any).__copiedProvenanceCode)).toBe(
    "import matplotlib\nplt.savefig('volcano.png')",
  );
  const codeScrollOwners = await page.locator(".artifact-modal .am-panel").evaluate((panel) => {
    const code = panel.querySelector<HTMLElement>(".rp-code")!;
    code.querySelector("code")!.textContent = Array.from({ length: 200 }, (_, i) => `line ${i + 1}`).join("\n");
    const scrollsVertically = (element: HTMLElement) => {
      const overflow = getComputedStyle(element).overflowY;
      return (overflow === "auto" || overflow === "scroll") && element.scrollHeight > element.clientHeight;
    };
    return {
      panel: scrollsVertically(panel as HTMLElement),
      code: scrollsVertically(code),
    };
  });
  expect(codeScrollOwners).toEqual({ panel: true, code: false });
  // Environment tab renders the captured package list.
  await page.locator(".am-tab", { hasText: "Environment" }).click();
  await expect(page.locator(".am-env")).toContainText("matplotlib");
  await page.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".artifact-modal")).toHaveCount(0);
  await expect(page.locator(".center-tab.active")).toContainText("volcano.png");
  const centerImage = page.locator(".center-file-preview .rp-img");
  const centerWidthAt100 = await centerImage.evaluate((el) => el.getBoundingClientRect().width);
  await centerImage.hover();
  await page.mouse.wheel(0, -100);
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("125%");
  await expect.poll(() => centerImage.evaluate((el) => el.getBoundingClientRect().width))
    .toBeGreaterThan(centerWidthAt100);
  const centerViewport = page.locator(".center-file-preview .file-preview-zoom-viewport");
  await centerViewport.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    const y = rect.top + rect.height * 0.5;
    const startX = rect.left + rect.width * 0.7;
    const endX = rect.left + rect.width * 0.3;
    el.dispatchEvent(new PointerEvent("pointerdown", {
      bubbles: true,
      button: 0,
      pointerId: 2,
      clientX: startX,
      clientY: y,
    }));
    el.dispatchEvent(new PointerEvent("pointermove", {
      bubbles: true,
      button: 0,
      buttons: 1,
      pointerId: 2,
      clientX: endX,
      clientY: y,
    }));
    el.dispatchEvent(new PointerEvent("pointerup", {
      bubbles: true,
      button: 0,
      pointerId: 2,
      clientX: endX,
      clientY: y,
    }));
  });
  await expect.poll(() => centerViewport.evaluate((el) => el.scrollLeft)).toBeGreaterThan(0);
});

test("PDF artifacts render inside the app without a browser PDF plugin", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open paper.pdf");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="paper.pdf"] .rp-tile-main').click();

  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  // Single-page viewer: one page is rendered at a time, navigated with controls.
  await expect(modal.locator('.rp-pdf[data-page-count="2"][data-current-page="1"]')).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "read_file_bytes"))
    .toMatchObject({ path: "paper.pdf", maxBytes: 100 * 1024 * 1024 });
  const renderedPage = modal.locator('.rp-pdf-page[data-page="1"][data-rendered="true"]');
  await expect(renderedPage).toBeVisible();
  await expect(modal.locator(".rp-pdf-page")).toHaveCount(1);
  const canvas = renderedPage.locator("canvas");
  await expect(canvas).toBeVisible();
  await expect.poll(() => canvas.evaluate(
    (el: HTMLCanvasElement) => el.width * el.height,
  )).toBeGreaterThan(0);
  const pageWidthAt100 = await renderedPage.evaluate((el) => el.getBoundingClientRect().width);
  const textSpan = renderedPage.locator(".rp-pdf-textlayer span").first();
  const textWidthAt100 = await textSpan.evaluate((el) => el.getBoundingClientRect().width);
  // A fit-width page can still be taller than the modal. It must be pannable at
  // 100%; panning depends on actual overflow, not on zoom being above 100%.
  const viewport = modal.locator(".file-preview-zoom-viewport");
  await expect.poll(() => viewport.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
  await viewport.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    const x = rect.left + rect.width * 0.5;
    const startY = rect.top + rect.height * 0.7;
    const endY = rect.top + rect.height * 0.25;
    el.dispatchEvent(new PointerEvent("pointerdown", {
      bubbles: true,
      button: 0,
      pointerId: 3,
      clientX: x,
      clientY: startY,
    }));
    el.dispatchEvent(new PointerEvent("pointermove", {
      bubbles: true,
      button: 0,
      buttons: 1,
      pointerId: 3,
      clientX: x,
      clientY: endY,
    }));
    el.dispatchEvent(new PointerEvent("pointerup", {
      bubbles: true,
      button: 0,
      pointerId: 3,
      clientX: x,
      clientY: endY,
    }));
  });
  await expect.poll(() => viewport.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
  await page.getByRole("button", { name: "Zoom in" }).click();
  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("150%");
  await expect.poll(() => renderedPage.evaluate((el) => el.getBoundingClientRect().width))
    .toBeGreaterThan(pageWidthAt100 * 1.4);
  await expect.poll(() => textSpan.evaluate((el) => el.getBoundingClientRect().width))
    .toBeGreaterThan(textWidthAt100 * 1.4);
  await page.getByRole("button", { name: "Reset zoom" }).click();
  await page.getByRole("button", { name: "Zoom out" }).click();
  await page.getByRole("button", { name: "Zoom out" }).click();
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("50%");
  await expect.poll(() => renderedPage.evaluate((el) => el.getBoundingClientRect().width))
    .toBeLessThan(pageWidthAt100 * 0.6);
  await expect.poll(() => textSpan.evaluate((el) => el.getBoundingClientRect().width))
    .toBeLessThan(textWidthAt100 * 0.6);
  await expect(page.getByRole("button", { name: "Previous page" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Next page" }).locator("svg")).toBeVisible();
  await expect(modal.locator('embed[type="application/pdf"]')).toHaveCount(0);
  // A selectable text layer sits over the canvas so PDF text can be added to chat.
  await expect(renderedPage.locator(".rp-pdf-textlayer")).toContainText("PDF preview works");
});

test("PDF artifacts switch pages with toolbar buttons, arrow keys, and Page Up/Down", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open paper.pdf");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="paper.pdf"] .rp-tile-main').click();

  const modal = page.locator(".artifact-modal");
  await expect(modal.locator('.rp-pdf[data-current-page="1"]')).toBeVisible();
  await expect(modal.locator('.rp-pdf-page[data-page="1"][data-rendered="true"]')).toBeVisible();

  await page.getByRole("button", { name: "Zoom in" }).click();
  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("150%");

  // Toolbar button steps forward.
  await page.getByRole("button", { name: "Next page" }).click();
  await expect(modal.locator('.rp-pdf[data-current-page="2"]')).toBeVisible();
  const secondPage = modal.locator('.rp-pdf-page[data-page="2"][data-rendered="true"]');
  await expect(secondPage).toBeVisible();
  await expect.poll(() => secondPage.evaluate((el) => Math.abs(
    el.getBoundingClientRect().width
      - el.querySelector(".rp-pdf-textlayer")!.getBoundingClientRect().width,
  ))).toBeLessThan(2);
  await expect(page.getByRole("button", { name: "Next page" })).toBeDisabled();

  // Page Up steps back. Wait for the page to finish rendering (rendered="true")
  // before the next key — stepPage is a no-op while a render is in flight.
  await page.keyboard.press("PageUp");
  await expect(modal.locator('.rp-pdf-page[data-page="1"][data-rendered="true"]')).toBeVisible();

  // Arrow keys also navigate: Right → next, Left → previous.
  await page.keyboard.press("ArrowRight");
  await expect(modal.locator('.rp-pdf[data-current-page="2"]')).toBeVisible();
  await expect(modal.locator('.rp-pdf-page[data-page="2"][data-rendered="true"]')).toBeVisible();
  await page.keyboard.press("ArrowLeft");
  await expect(modal.locator('.rp-pdf[data-current-page="1"]')).toBeVisible();
  await expect(modal.locator('.rp-pdf-page[data-page="1"][data-rendered="true"]')).toBeVisible();
});

test("PDF text can be selected and added to chat", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open paper.pdf");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="paper.pdf"] .rp-tile-main').click();

  const layer = page.locator(".artifact-modal .rp-pdf-textlayer");
  await expect(layer).toContainText("PDF preview works");

  // Drag-select a text-layer span with real pointer input. The zoom viewport
  // must leave glyph drags to text selection while blank-page drags pan.
  const span = layer.locator("span").first();
  await span.scrollIntoViewIfNeeded();
  const box = await span.boundingBox();
  if (!box) throw new Error("PDF text span has no bounding box");
  const y = box.y + box.height * 0.5;
  await page.mouse.move(box.x + 2, y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width - 2, y, { steps: 5 });
  await page.mouse.up();
  const popup = page.locator(".selection-popup");
  await expect(popup).toBeVisible();
  await popup.getByRole("button", { name: "Add to chat" }).click();
  await expect(page.locator(".composer-reference-chips .quote")).toContainText("PDF preview works");
});

test("DOCX text in the modal (Files browser) can be added to chat", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('.fb-row[data-workspace-path*="manuscript.docx"]').click();

  const docx = page.locator(".artifact-modal .rp-docx");
  await expect(docx).toContainText("Differential expression of FX-cell markers");
  const heading = docx.getByText("Differential expression of FX-cell markers").first();
  // Modal preview text must stay selectable despite the overlay's user-select:none.
  await heading.evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
  });
  await page.locator(".selection-popup").getByRole("button", { name: "Add to chat" }).click();
  await expect(page.locator(".composer-reference-chips .quote")).toContainText("Differential expression");
});

test("composer quote card ellipsizes a long source path instead of overflowing", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('.fb-row[data-workspace-path*="manuscript.docx"]').click();

  const docx = page.locator(".artifact-modal .rp-docx");
  await expect(docx).toContainText("Differential expression of FX-cell markers");
  const heading = docx.getByText("Differential expression of FX-cell markers").first();
  await heading.evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
  });
  await page.locator(".selection-popup").getByRole("button", { name: "Add to chat" }).click();
  const card = page.locator(".composer-reference-chips .quote");
  await expect(card).toContainText("Differential expression");

  // A long Windows-style source path must stay on one clipped line inside the
  // fixed-width card instead of wrapping out of it or spilling past its edge.
  const meta = card.locator(".composer-attachment-meta");
  await meta.evaluate((el) => {
    el.textContent = "D:\\New PHD\\depmap\\results\\reports\\DepMap_26Q1_Full_Data_Inventory.md";
  });
  const metrics = await card.evaluate((el) => {
    const cardBox = el.getBoundingClientRect();
    const metaEl = el.querySelector(".composer-attachment-meta")!;
    const metaBox = metaEl.getBoundingClientRect();
    return {
      cardRight: cardBox.right,
      metaRight: metaBox.right,
      metaHeight: metaBox.height,
      metaWhiteSpace: getComputedStyle(metaEl).whiteSpace,
    };
  });
  expect(metrics.metaWhiteSpace).toBe("nowrap");
  expect(metrics.metaHeight).toBeLessThan(20); // single line (~13px), not wrapped
  expect(metrics.metaRight).toBeLessThanOrEqual(metrics.cardRight + 1);
});

test("DOCX artifacts render offline with headings, tables, and equations", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open manuscript.docx");
  await page.getByRole("button", { name: "Send" }).click();
  // Opening the artifact while its streaming turn still owns the artifact
  // projection can replace the clicked tile before the selection signal lands.
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  const manuscriptTile = page.locator('.rp-tile[data-artifact-name="manuscript.docx"] .rp-tile-main');
  await expect(manuscriptTile).toBeVisible();
  await manuscriptTile.click();

  // docx-preview renders a `.docx-wrapper` of `section.docx` pages, fully offline.
  const docx = page.locator(".rp-docx");
  await expect(docx.locator(".docx-wrapper section.docx").first()).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "read_file_bytes"))
    .toMatchObject({ path: "manuscript.docx", maxBytes: 32 * 1024 * 1024 });
  await expect(docx).toContainText("Differential expression of FX-cell markers");
  await expect(docx).toContainText("FOXA2"); // a table cell
  // The OMML equations convert to MathML — this is the #274 formula concern.
  await expect(docx.locator("math").first()).toBeAttached();
  // The wrapping preview carries data-file-path so P2 selection/annotate works here too.
  await expect(page.locator('.rp-file-preview[data-file-path*="manuscript.docx"]')).toBeVisible();

  // #274/#951: a tall docx must remain scrollable without carrying the preview
  // header and its close action out of view. The document owns the scroll while
  // the surrounding .rp-view stays fixed.
  const view = page.locator(".rp-view");
  await expect(view).toHaveAttribute("data-preview-kind", "docx");
  const closePreview = view.getByRole("button", { name: "Close preview" });
  await expect(closePreview).toBeVisible();
  await docx.locator(".docx-wrapper").evaluate((el) => {
    (el as HTMLElement).style.minHeight = "4000px";
  });
  await expect.poll(() => docx.evaluate((el) => el.scrollHeight - el.clientHeight)).toBeGreaterThan(100);
  await docx.evaluate((el) => { el.scrollTop = 500; });
  await expect.poll(() => docx.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
  await expect.poll(() => view.evaluate((el) => el.scrollTop)).toBe(0);
  await expect(closePreview).toBeVisible();
  await closePreview.click();
  await expect(view).toHaveCount(0);
});

test("DOCX opened from the Files browser scrolls inside the modal (#274)", async ({ page }) => {
  await enterApp(page);
  // Files browser → docx opens in the artifact modal (like the tester's flow).
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('.fb-row[data-workspace-path*="manuscript.docx"]').click();

  const docx = page.locator(".artifact-modal .rp-docx");
  await expect(docx.locator(".docx-wrapper section.docx").first()).toBeVisible();
  // A tall document must scroll inside .rp-docx — the modal figure clips, so the
  // bounded height has to reach .rp-docx (the #274 "can't scroll down" bug).
  await docx.locator(".docx-wrapper").evaluate((el) => {
    (el as HTMLElement).style.minHeight = "4000px";
  });
  await expect.poll(() => docx.evaluate((el) => el.scrollHeight - el.clientHeight)).toBeGreaterThan(100);
  await docx.evaluate((el) => { el.scrollTop = 800; });
  await expect.poll(() => docx.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
});

test("XLSX files render in a virtualized read-only workbook", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('.fb-row[data-workspace-path="office-preview.xlsx"]').click();

  const workbook = page.locator(".artifact-modal .rp-xlsx");
  await expect(workbook).toBeVisible();
  await expect(workbook.locator(".rp-xlsx-tabs button.active")).toHaveText("Results");
  await expect(workbook).toContainText("FOXA2");
  await expect(workbook).toContainText("Merged result");
  const formulaCell = workbook.locator(".rp-xlsx-cell", { hasText: "84" });
  await formulaCell.click();
  await expect(workbook.locator(".rp-xlsx-formula-value")).toHaveText("=B2*2");
  await expect.poll(() => lastInvokeArgs(page, "read_file_bytes"))
    .toMatchObject({ path: "office-preview.xlsx", maxBytes: 32 * 1024 * 1024 });
  // Virtualization keeps the DOM bounded to the viewport, even though the
  // content surface represents the worksheet dimensions.
  await expect.poll(() => workbook.locator(".rp-xlsx-cell").count()).toBeLessThan(100);
});

test("PPTX files render lazily inside the app", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('.fb-row[data-workspace-path="office-preview.pptx"]').click();

  const presentation = page.locator(".artifact-modal .rp-pptx");
  await expect(presentation).toBeVisible();
  await expect(presentation.locator('[data-slide-index="0"]')).toBeVisible();
  await expect(presentation).toContainText("Wisp PPTX preview");
  await expect.poll(() => lastInvokeArgs(page, "read_file_bytes"))
    .toMatchObject({ path: "office-preview.pptx", maxBytes: 32 * 1024 * 1024 });
});

test("center previews are read-only and send selected code or text to the AI conversation", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open report.md");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();

  // Right-click the file tile → "Open in center" opens the real workspace file.
  await page.locator('.rp-tile[data-artifact-name="report.md"]').click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  const preview = page.locator('.center-file-preview[data-file-path="report.md"]');
  await expect(preview.locator("h1")).toHaveText("Draft manuscript");

  await expect(preview.getByRole("button", { name: "Rewind" })).toHaveCount(0);
  await expect(preview.locator(".center-file-editor")).toHaveCount(0);

  // Selecting source material offers the AI handoff. Choosing it opens the
  // existing conversation beside the read-only document and quotes selection.
  await preview.getByText("Original body paragraph.").evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
  });
  await page.locator(".selection-popup")
    .getByRole("button", { name: "Ask AI in the conversation" })
    .click();
  await expect(page.locator(".chat")).toBeVisible();
  await expect(composer(page)).toBeVisible();
  await expect(page.locator(".composer-reference-chips .quote"))
    .toContainText("Original body paragraph.");
  await expect(page.locator(".rightpane")).toHaveCount(0);
});

test("center split keeps the same conversation beside the open document", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open report.md");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="report.md"]').click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  const preview = page.locator('.center-file-preview[data-file-path="report.md"]');
  await expect(preview.locator("h1")).toHaveText("Draft manuscript");

  // Opening a document hides the conversation by default.
  const chat = page.locator(".chat");
  await expect(chat).toBeHidden();

  // Split → the conversation comes back beside the document and the right pane
  // folds away so the two share its width.
  await preview.locator("[data-center-split]").click();
  await expect(chat).toBeVisible();
  await expect(composer(page)).toBeVisible();
  await expect(page.locator(".rightpane")).toHaveCount(0);

  // Really side by side, not stacked: the chat starts past the document's right edge.
  const doc = (await preview.boundingBox())!;
  const box = (await chat.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(doc.x + doc.width - 1);
  expect(box.y).toBeLessThan(doc.y + doc.height);

  // The divider is a real drag target. Moving it right gives the document more
  // room and makes the chat composer switch to its compact controls.
  const divider = page.getByRole("separator", { name: "Resize document and chat" });
  const dividerBox = (await divider.boundingBox())!;
  await page.mouse.move(dividerBox.x + dividerBox.width / 2, dividerBox.y + 100);
  await page.mouse.down();
  await page.mouse.move(dividerBox.x + 100, dividerBox.y + 100);
  await page.mouse.up();
  const resizedDoc = (await preview.boundingBox())!;
  const resizedChat = (await chat.boundingBox())!;
  expect(resizedDoc.width).toBeGreaterThan(doc.width + 60);
  expect(resizedChat.width).toBeLessThan(box.width - 60);
  await expect(page.locator(".model-picker-btn")).toHaveCSS("height", "28px");
  await expect(page.locator("button.send")).toHaveCSS("height", "28px");
  await page.locator(".thread").evaluate((thread) => {
    thread.insertAdjacentHTML(
      "beforeend",
      '<div class="usage-row" data-testid="chat-usage-regression"><div class="usage-line">20.5k in · 555 out tokens · 20.2k cached · 124 reasoning</div></div>',
    );
  });
  const usageLine = page.getByTestId("chat-usage-regression").locator(".usage-line");
  await expect(usageLine).toBeVisible();
  expect((await usageLine.boundingBox())!.height).toBeLessThan(20);

  // Same session, not a new one — the sent message is still in the thread.
  await expect(chat.getByText("open report.md")).toBeVisible();

  // Toggling off restores the document-only view.
  await preview.locator("[data-center-split]").click();
  await expect(chat).toBeHidden();
});

test("artifact modal switches between images with left and right arrows", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make plots first.png second.png third.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await expect(page.locator('.rp-tile[data-artifact-name="second.png"]')).toBeVisible();

  await page.locator('.rp-tile[data-artifact-name="second.png"] .rp-tile-main').click();
  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".am-name")).toHaveText("second.png");
  await expect(page.getByRole("button", { name: "Previous image" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "Next image" })).toBeEnabled();

  await page.keyboard.press("ArrowRight");
  await expect(modal.locator(".am-name")).toHaveText("third.png");
  await expect(page.getByRole("button", { name: "Next image" })).toBeDisabled();

  await page.keyboard.press("ArrowLeft");
  await expect(modal.locator(".am-name")).toHaveText("second.png");
  await page.keyboard.press("ArrowLeft");
  await expect(modal.locator(".am-name")).toHaveText("first.png");
  await expect(page.getByRole("button", { name: "Previous image" })).toBeDisabled();
});

test("center file tabs are restored per conversation", async ({ page }) => {
  await enterApp(page);

  await page.keyboard.press("Control+K");
  const search = commandPalette(page);
  await search.fill("Current analysis");
  await search.press("Enter");

  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  await page.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-tab.active")).toContainText("volcano.png");

  await page.keyboard.press("Control+K");
  await search.fill("Older structure run");
  await search.press("Enter");
  await expect(page.locator(".center-tab-wrap")).toHaveCount(0);
  await expect(page.locator(".center-tabs > .center-tab")).toHaveClass(/active/);

  await page.keyboard.press("Control+K");
  await search.fill("Current analysis");
  await search.press("Enter");
  await expect(page.locator(".center-tab-wrap")).toHaveCount(1);
  await expect(page.locator(".center-tab.active")).toContainText("volcano.png");
});

test("image preview context menu copies the image", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  const image = page.locator(".artifact-modal .rp-img");
  await expect(image).toBeVisible();
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, "write", {
      configurable: true,
      value: async (items: ClipboardItem[]) => { (window as any).__copiedImageTypes = items.flatMap((item) => item.types); },
    });
  });
  await image.click({ button: "right" });
  await page.getByRole("button", { name: "Copy image" }).click();
  await expect(page.locator(".copy-toast")).toHaveText("Copied");
  await expect.poll(() => page.evaluate(() => (window as any).__copiedImageTypes)).toContain("image/png");
});

test("image crop stays highlighted until it is added to chat", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  const image = page.locator(".artifact-modal .rp-img");
  await expect(image).toBeVisible();

  // Toggle crop mode → the capture layer appears.
  await page.getByRole("button", { name: "Select a region to ask about" }).click();
  const layer = page.locator(".file-preview-crop-layer");
  await expect(layer).toBeVisible();

  // Rubber-band a rectangle inside the image.
  const box = (await image.boundingBox())!;
  await page.mouse.move(box.x + 20, box.y + 20);
  await page.mouse.down();
  await page.mouse.move(box.x + 120, box.y + 100, { steps: 4 });
  await expect(page.locator(".file-preview-crop-rect")).toBeVisible();
  await page.mouse.up();

  // Releasing freezes the region and raises the actions; nothing uploads
  // until the user picks an attach action (a comment-only region never will).
  const actions = page.locator(".file-preview-crop-actions");
  await expect(actions.getByRole("button", { name: "Add to chat", exact: true })).toBeVisible();
  await expect(actions.getByRole("button", { name: "Add to chat and jump back to chat" })).toBeVisible();
  await expect(page.locator(".file-preview-crop-rect.selected")).toContainText("Selected region");
  expect(await lastInvokeArgs(page, "upload_file")).toBeNull();
  await expect(page.locator(".composer-attachments .composer-attachment.ready")).toHaveCount(0);

  // Plain Add uploads the region, keeps the preview open, and attaches the PNG.
  await actions.getByRole("button", { name: "Add to chat", exact: true }).click();
  await expect.poll(() => lastInvokeArgs(page, "upload_file"))
    .toMatchObject({ filename: expect.stringMatching(/^region_.*\.png$/) });
  await expect(page.locator(".composer-attachments .composer-attachment.ready")).toContainText("region_");
  await expect(page.locator(".artifact-modal")).toBeVisible();
  await expect(layer).toHaveCount(0);
});

test("image region comments become numbered pins and a revision request", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  const image = page.locator(".artifact-modal .rp-img");
  await expect(image).toBeVisible();

  await page.getByRole("button", { name: "Select a region to ask about" }).click();
  const box = (await image.boundingBox())!;
  await page.mouse.move(box.x + 20, box.y + 20);
  await page.mouse.down();
  await page.mouse.move(box.x + 120, box.y + 100, { steps: 4 });
  await page.mouse.up();

  // The region popup carries a note input; Enter commits it into a pin and
  // keeps crop mode armed. No upload happens for a comment-only region.
  const note = page.locator(".file-preview-crop-annotate input");
  await note.fill("increase the contrast here");
  await note.press("Enter");
  await expect(page.locator(".file-preview-crop-annotate")).toHaveCount(0);
  await expect(page.locator(".file-preview-pin-marker")).toHaveText("1");
  expect(await lastInvokeArgs(page, "upload_file")).toBeNull();

  // Send for AI revision lands one quote in the composer and closes the modal.
  await page.getByRole("button", { name: "Send for AI revision (1)" }).click();
  await expect(page.locator(".artifact-modal")).toHaveCount(0);
  await expect(page.locator(".composer-reference-chips .quote")).toContainText("Revision notes");
  await expect(composer(page)).toBeFocused();

  // Reopening the preview keeps the pin; clicking it in crop mode deletes it.
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  await expect(page.locator(".file-preview-pin-marker")).toHaveText("1");
  await page.getByRole("button", { name: "Select a region to ask about" }).click();
  await page.locator(".file-preview-pin-marker").click();
  await expect(page.locator(".file-preview-pin-marker")).toHaveCount(0);
});

test("image crop can be added and jump back from the preview to chat", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  await page.getByRole("button", { name: "Open in center" }).click();

  const image = page.locator(".center-file-preview .rp-img");
  await expect(image).toBeVisible();
  await page.getByRole("button", { name: "Select a region to ask about" }).click();
  const box = (await image.boundingBox())!;
  await page.mouse.move(box.x + 20, box.y + 20);
  await page.mouse.down();
  await page.mouse.move(box.x + 120, box.y + 100, { steps: 4 });
  await page.mouse.up();

  const jump = page
    .locator(".file-preview-crop-actions")
    .getByRole("button", { name: "Add to chat and jump back to chat" });
  await expect(jump).toBeVisible();
  await expect(page.locator(".composer-attachments .composer-attachment.ready")).toHaveCount(0);
  await jump.click();

  await expect(page.locator(".composer-attachments .composer-attachment.ready")).toContainText("region_");
  await expect(page.locator(".center-file-preview")).toHaveCount(0);
  await expect(composer(page)).toBeFocused();
});

test("artifact panel normalizes png/pdf shorthand to the previewable image", async ({ page }) => {
  await enterApp(page);
  await page
    .locator("#composer-input")
    .fill("show `figures/panel_I_heatmap_4genes_median.png/.pdf`");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();

  const tile = page.locator('.rp-tile[data-artifact-name="panel_I_heatmap_4genes_median.png"]');
  await expect(tile).toBeVisible();
  await expect(tile.locator(".rp-badge")).toHaveText("image");
  await expect(page.locator('.rp-tile[data-artifact-name="panel_I_heatmap_4genes_median.png/.pdf"]')).toHaveCount(0);
});

test("settings page shows the saved protocol", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await expect(providerSelect(page)).toHaveValue("openai");
  await expect(page.locator("label.settings-check", { hasText: "Supports image input" })).toHaveCSS("flex-direction", "row");
  await expect(page.locator("label.settings-check", { hasText: "Use for image analysis" })).toHaveCSS("flex-direction", "row");
  await expect(page.locator("label.settings-check", { hasText: "Use for image generation" })).toHaveCSS("flex-direction", "row");
  await page.locator(".settings-footer").getByRole("button", { name: "Cancel" }).click();
});

test("model settings updates activation and confirms removal", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Models");

  const opus = page.locator(".settings-list-row").filter({ hasText: "opus-4.8" });
  await opus.getByRole("button", { name: "Use" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toMatchObject({ id: "opus" });
  await expect(opus).toHaveClass(/settings-list-row-active/);

  const deepseek = page.locator(".settings-list-row").filter({ hasText: "deepseek-v4-pro" });
  await deepseek.getByTitle("Remove model").click();
  const confirm = page.getByTestId("model-delete-confirm");
  await expect(confirm).toContainText("Remove deepseek-v4-pro? This cannot be undone.");
  await expect.poll(() => lastInvokeArgs(page, "remove_model")).toBeNull();

  await confirm.getByRole("button", { name: "Remove model" }).click();
  await expect.poll(() => lastInvokeArgs(page, "remove_model")).toMatchObject({ id: "default" });
  await expect(deepseek).toHaveCount(0);
});

test("appearance settings persist separate light and dark palettes and font sizes", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Appearance");

  await page.getByTestId("theme-mode-light").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByTestId("appearance-palette-select").selectOption("catppuccin");
  await expect(page.getByTestId("appearance-palette-select")).toHaveValue("catppuccin");
  await expect(page.locator("html")).toHaveAttribute("data-light-palette", "catppuccin");

  await page.getByTestId("theme-mode-dark").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByTestId("appearance-palette-select").selectOption("gruvbox");
  await expect(page.getByTestId("appearance-palette-select")).toHaveValue("gruvbox");
  await expect(page.locator("html")).toHaveAttribute("data-dark-palette", "gruvbox");

  await page.getByRole("slider", { name: "UI font size" }).fill("16");
  await page.getByRole("slider", { name: "Code font size" }).fill("15");
  await expect.poll(() => page.evaluate(() => ({
    theme: localStorage.getItem("wisp-theme"),
    light: localStorage.getItem("wisp-light-palette"),
    dark: localStorage.getItem("wisp-dark-palette"),
    ui: localStorage.getItem("wisp-ui-font-size"),
    code: localStorage.getItem("wisp-code-font-size"),
  }))).toEqual({ theme: "dark", light: "catppuccin", dark: "gruvbox", ui: "16", code: "15" });

  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.locator("html")).toHaveAttribute("data-light-palette", "catppuccin");
  await expect(page.locator("html")).toHaveAttribute("data-dark-palette", "gruvbox");
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.documentElement)
    .getPropertyValue("--ui-font-size").trim())).toBe("16px");
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.documentElement)
    .getPropertyValue("--code-font-size").trim())).toBe("15px");
});

test("appearance settings customize font families and allow 0-30px sizes", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Appearance");

  const uiFont = page.getByTestId("appearance-ui-font");
  const codeFont = page.getByTestId("appearance-code-font");
  await uiFont.fill("Noto Sans SC");
  await codeFont.fill("Fira Code");
  // The user family is injected ahead of the default stack on :root.
  await expect.poll(() => page.evaluate(() =>
    document.documentElement.getAttribute("style") ?? "",
  )).toContain("--font-user-ui:Noto Sans SC");
  await expect.poll(() => page.evaluate(() =>
    getComputedStyle(document.body).fontFamily,
  )).toContain("Noto Sans SC");

  // Font sizes accept the full 0-30 range instead of the old 12-18 clamp.
  await page.getByRole("slider", { name: "UI font size" }).fill("30");
  await page.getByRole("slider", { name: "Code font size" }).fill("0");
  await expect.poll(() => page.evaluate(() => ({
    ui: localStorage.getItem("wisp-ui-font-size"),
    code: localStorage.getItem("wisp-code-font-size"),
    uiFont: localStorage.getItem("wisp-font-ui"),
    codeFont: localStorage.getItem("wisp-font-mono"),
  }))).toEqual({ ui: "30", code: "0", uiFont: "Noto Sans SC", codeFont: "Fira Code" });

  await page.reload();
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.documentElement)
    .getPropertyValue("--font-user-ui").trim())).toBe("Noto Sans SC");

  // Clearing the input removes the override and restores the default stack.
  await page.locator(".proj-card-main").first().click();
  await openSettingsSection(page, "Appearance");
  await expect(page.getByTestId("appearance-ui-font")).toHaveValue("Noto Sans SC");
  await page.getByTestId("appearance-ui-font").fill("");
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.documentElement)
    .getPropertyValue("--font-user-ui").trim())).toBe("");
  await expect.poll(() => page.evaluate(() => localStorage.getItem("wisp-font-ui"))).toBeNull();
});

test("UI font size setting scales chat message body text", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("MDLIST");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("FX细胞")).toBeVisible({ timeout: 10_000 });
  const bodyFontSize = () => page.locator(".msg.assistant .body.md").first()
    .evaluate((el) => getComputedStyle(el).fontSize);
  const composerFontSize = () => composer(page)
    .evaluate((el) => getComputedStyle(el).fontSize);
  expect(await bodyFontSize()).toBe("15px");
  expect(await composerFontSize()).toBe("14px");

  await openSettingsSection(page, "Appearance");
  await page.getByRole("slider", { name: "UI font size" }).fill("18");
  await page.getByRole("button", { name: "Back to app" }).click();

  await expect.poll(bodyFontSize).toBe("19px");
  await expect.poll(composerFontSize).toBe("18px");
});

test("configure presentation applies font size and theme without opening Settings", async ({ page }) => {
  await enterApp(page);
  await emitTauriEvent(page, "agent", {
    kind: "ToolPresentation",
    frame_id: "any-session",
    presentation_kind: "app_prefs",
    payload: { ui_font_size: 18, theme: "dark" },
  });
  await expect.poll(() => page.evaluate(() =>
    document.documentElement.getAttribute("style") ?? ""
  )).toContain("--ui-font-size:18px");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
});

test("markdown table font follows UI font size", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("MDTABLE");
  await page.getByRole("button", { name: "Send" }).click();
  const table = page.locator(".msg.assistant .md table").first();
  await expect(table).toBeVisible();
  const tableFont = () => table.evaluate((el) => getComputedStyle(el).fontSize);
  expect(await tableFont()).toBe("13px");

  await openSettingsSection(page, "Appearance");
  await page.getByRole("slider", { name: "UI font size" }).fill("18");
  await page.getByRole("button", { name: "Back to app" }).click();
  await expect.poll(tableFont).toBe("17px");
});

test("custom CSS hides the bold-at-start lead bar", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("MDLEADBAR");
  await page.getByRole("button", { name: "Send" }).click();

  const leadStrong = page.locator(".msg.assistant .body.md").last()
    .locator("p.md-lead-strong > strong").first();
  await expect(leadStrong).toHaveText("结论");
  const barWidth = () => leadStrong.evaluate((el) => getComputedStyle(el).borderLeftWidth);
  expect(await barWidth()).toBe("3px");

  await openSettingsSection(page, "Appearance");
  await page.getByTestId("appearance-custom-css")
    .fill(":root { --md-lead-bar-width: 0; --md-lead-bar-pad: 0; }");
  await page.getByRole("button", { name: "Back to app" }).click();
  await expect.poll(barWidth).toBe("0px");
  await expect.poll(() => leadStrong.evaluate((el) => getComputedStyle(el).paddingLeft)).toBe("0px");
});

test("appearance custom CSS can be pasted, imported, and cleared", async ({ page }, testInfo) => {
  await enterApp(page);
  await openSettingsSection(page, "Appearance");

  const card = page.getByTestId("appearance-custom-css-card");
  await expect(card).toHaveClass(/appearance-config-card/);
  await expect(card.getByTestId("appearance-custom-css")).toBeVisible();
  await expect(card.getByTestId("appearance-custom-css-clear")).toBeDisabled();

  const css = ":root { --md-lead-bar-width: 0; --md-lead-bar-pad: 0; }";
  await page.getByTestId("appearance-custom-css").fill(css);
  await expect.poll(() => page.evaluate(() =>
    document.getElementById("wisp-custom-theme")?.textContent ?? ""
  )).toContain("--md-lead-bar-width: 0");
  await expect.poll(() => page.evaluate(() => localStorage.getItem("wisp-custom-css")))
    .toContain("--md-lead-bar-width: 0");

  const imported = ":root { --md-table-font-size: 16px; }";
  mkdirSync(testInfo.outputDir, { recursive: true });
  const file = resolve(testInfo.outputDir, "theme.css");
  writeFileSync(file, imported);
  await page.getByTestId("appearance-custom-css-file").setInputFiles(file);
  await expect.poll(() => page.getByTestId("appearance-custom-css").inputValue())
    .toContain("--md-table-font-size: 16px");
  await expect.poll(() => page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--md-table-font-size").trim()
  )).toBe("16px");

  await page.getByTestId("appearance-custom-css-clear").click();
  await expect.poll(() => page.evaluate(() =>
    document.getElementById("wisp-custom-theme")?.textContent ?? ""
  )).toBe("");
});

test("custom CSS sanitizes remote url and import", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Appearance");
  await page.getByTestId("appearance-custom-css").fill([
    ":root { --clay: #111111; }",
    '@import url("https://evil.example/x.css");',
    "body { background: url(https://evil.example/p.png); }",
  ].join("\n"));
  await expect.poll(() => page.evaluate(() =>
    document.getElementById("wisp-custom-theme")?.textContent ?? ""
  )).toContain("--clay: #111111");
  const injected = () => page.evaluate(() =>
    (document.getElementById("wisp-custom-theme")?.textContent ?? "").toLowerCase()
  );
  await expect.poll(injected).not.toContain("@import");
  await expect.poll(injected).not.toContain("url(");
});

test("configure presentation applies custom CSS", async ({ page }) => {
  await enterApp(page);
  await emitTauriEvent(page, "agent", {
    kind: "ToolPresentation",
    frame_id: "any-session",
    presentation_kind: "app_prefs",
    payload: { custom_css: ":root { --md-lead-bar-width: 0; }" },
  });
  await expect.poll(() => page.evaluate(() =>
    document.getElementById("wisp-custom-theme")?.textContent ?? ""
  )).toContain("--md-lead-bar-width: 0");
});

test("UI font size setting scales Chinese chat markdown and composer", async ({ page }) => {
  await page.goto("/?mockLocale=zh");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator(".sidebar").getByRole("button", { name: "新建会话" })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh");
  await composer(page).fill("MDLIST");
  await page.getByRole("button", { name: "发送" }).click();
  await expect(page.getByText("FX细胞")).toBeVisible({ timeout: 10_000 });
  const bodyFontSize = () => page.locator(".msg.assistant .body.md").first()
    .evaluate((el) => getComputedStyle(el).fontSize);
  const composerFontSize = () => composer(page)
    .evaluate((el) => getComputedStyle(el).fontSize);
  expect(await bodyFontSize()).toBe("15.5px");
  expect(await composerFontSize()).toBe("14px");

  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.getByRole("button", { name: "外观", exact: true }).click();
  await page.getByRole("slider", { name: "UI 字号" }).fill("18");
  await page.getByRole("button", { name: "返回应用" }).click();

  await expect.poll(bodyFontSize).toBe("19.5px");
  await expect.poll(composerFontSize).toBe("18px");
});

test("vision assignment keeps model fields and stored key placeholder untouched", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);

  const effort = page.getByLabel("Reasoning effort");
  const key = page.getByLabel("API key (stored in OS keyring)");
  const useForVision = page.getByLabel("Use for image analysis");

  await providerSelect(page).selectOption("openai_responses");
  await page.getByLabel("Base URL").fill("https://api.openai-proxy.org/v1");
  await page.getByRole("textbox", { name: "Model ID", exact: true }).fill("gpt-5.6-luna");
  await effort.selectOption("medium");
  await expect(key).toHaveValue("");
  await expect(key).toHaveAttribute("placeholder", "(stored — leave blank to keep)");

  if (await useForVision.isChecked()) {
    await useForVision.uncheck();
  }
  await useForVision.check();

  await expect(providerSelect(page)).toHaveValue("openai_responses");
  await expect(effort).toHaveValue("medium");
  await expect(key).toHaveValue("");

  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => page.evaluate(() => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      return value;
    };
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "save_model");
    const args = plain(calls.at(-1)?.args ?? null);
    return args ? { ...args, key: args.key ?? null } : null;
  })).toMatchObject({
    key: null,
    useForVision: true,
    profile: {
      provider: "openai_responses",
      // gpt-5.6-luna is in the baked catalog (128K out / 1.05M ctx).
      context_window: 1050000,
      reasoning_effort: "medium",
      use_for_vision: true,
    },
  });

  await page.locator(".settings-list-row").first().click();
  await expect(providerSelect(page)).toHaveValue("openai_responses");
  await expect(effort).toHaveValue("medium");
  await expect(page.getByLabel("Use for image analysis")).toBeChecked();
});

test("Fast profile toggle is available for both OpenAI HTTP protocols", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);

  for (const provider of ["openai", "openai_responses"]) {
    await providerSelect(page).selectOption(provider);
    await page.getByLabel("Base URL").fill("https://api.openai.com/v1");
    await page.getByRole("textbox", { name: "Model ID", exact: true }).fill("gpt-5.6-luna");
    await page.getByLabel("Reasoning effort").selectOption("high");
    const row = page.getByTestId("service-tier-toggle-row");
    const toggle = page.getByTestId("service-tier-toggle");
    await expect(row).toBeVisible();
    if (await toggle.isChecked()) await row.locator(".toggle-track").click();
    await expect(toggle).not.toBeChecked();
    await row.locator(".toggle-track").click();
    await expect(toggle).toBeChecked();
    await expect(row).toHaveClass(/enabled/);
    await expect(row).toContainText("On · priority");
    await page.getByRole("button", { name: "Save" }).click();
    await expect.poll(() => lastInvokeArgs(page, "save_model")).toMatchObject({
      profile: { provider, reasoning_effort: "high", service_tier: "priority" },
    });
    await page.locator(".settings-list-row").first().click();
  }

  await providerSelect(page).selectOption("anthropic");
  await expect(page.getByTestId("service-tier-toggle")).toHaveCount(0);
});

test("composer Fast lightning is a conversation override and has distinct states", async ({ page }) => {
  await enterApp(page);
  const fast = page.getByTestId("composer-fast-toggle");
  await expect(fast).toBeVisible();
  await expect(fast).toHaveAttribute("aria-pressed", "false");
  await expect(fast).not.toHaveClass(/enabled/);
  await fast.click();
  await expect(fast).toHaveAttribute("aria-pressed", "true");
  await expect(fast).toHaveClass(/enabled/);
  await expect(fast).toHaveAttribute("title", /service_tier=priority/);
  // No frame exists yet, so the choice is staged instead of mutating a profile.
  await expect.poll(() => lastInvokeArgs(page, "save_model")).toBeNull();

  await composer(page).fill("FAST FIRST TURN");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect.poll(async () => (await invokeArgsList(page, "set_session_service_tier")).length).toBe(1);
  await expect.poll(() => lastInvokeArgs(page, "set_session_service_tier"))
    .toMatchObject({ serviceTier: "priority" });
  const order = await page.evaluate(() => ((window as any).__skillInvokeLog ?? [])
    .map((call: any) => call.cmd)
    .filter((cmd: string) => ["new_session", "set_session_service_tier", "send_message"].includes(cmd)));
  expect(order.slice(-3)).toEqual(["new_session", "set_session_service_tier", "send_message"]);
});

test("composer Fast override is isolated per conversation and can return to inheritance", async ({ page }) => {
  await enterApp(page, "/?mockSessionModels=1&mockFastDefault=1");
  const fast = page.getByTestId("composer-fast-toggle");

  await page.locator('[data-session-id="s-model-a"]').click();
  await expect(fast).toHaveAttribute("aria-pressed", "true");
  await expect(fast).toHaveAttribute("title", /profile default/);
  await fast.click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_service_tier"))
    .toMatchObject({ sessionId: "s-model-a", serviceTier: "" });
  await expect(fast).toHaveAttribute("aria-pressed", "false");
  await expect(fast).toHaveAttribute("title", /conversation overrides/);

  await page.locator('[data-session-id="s-model-b"]').click();
  await expect(fast).toHaveAttribute("aria-pressed", "true");
  await expect(fast).toHaveAttribute("title", /profile default/);

  await page.locator('[data-session-id="s-model-a"]').click();
  await expect(fast).toHaveAttribute("aria-pressed", "false");
  await fast.click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_service_tier"))
    .toMatchObject({ sessionId: "s-model-a", serviceTier: undefined });
  await expect(fast).toHaveAttribute("aria-pressed", "true");
  await expect(fast).toHaveAttribute("title", /profile default/);
});

test("composer Fast lightning hides for ACP and unsupported HTTP providers", async ({ page }) => {
  await enterApp(page);
  await expect(page.getByTestId("composer-fast-toggle")).toBeVisible();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /opus-4.8/ }).click();
  await expect(page.getByTestId("composer-fast-toggle")).toHaveCount(0);

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await expect(page.getByTestId("composer-fast-toggle")).toHaveCount(0);
});

test("model settings rejects max output tokens above the known ceiling", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);

  // deepseek-v4-pro is in the baked catalog (384K output ceiling).
  const maxTokens = page.getByLabel("Max output tokens");
  await maxTokens.fill("1000000");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.locator(".settings-status"))
    .toContainText("accepts at most 384000 output tokens");
  await expect.poll(() => lastInvokeArgs(page, "save_model")).toBeNull();

  // The documented ceiling itself saves fine.
  await maxTokens.fill("384000");
  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_model"))
    .toMatchObject({ profile: { max_tokens: 384000 } });
});

test("model settings auto-fills catalog limits and save clamps to them", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Models");
  await page.getByRole("button", { name: /Add API access/i }).click();

  // Changing the URL refreshes suggested model ids; type the catalog id after.
  await page.getByLabel("Base URL").fill("https://api.kimi.com/coding/v1");
  const modelId = page.getByTestId("provider-model-row").first().getByLabel("Model ID");
  await modelId.fill("k3-256k");
  await page.getByLabel("API key (stored in OS keyring)").fill("sk-k3");

  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_model"))
    .toMatchObject({ profile: { model: "k3-256k" } });
  // Catalog lookup on save fills documented ceilings; the mocked backend also
  // clamps if the UI sent a larger window.
  const stored: any = await page.evaluate(async () => {
    const models: any[] = await (window as any).__TAURI__.core.invoke("list_models");
    return models.find((m: any) => m.model === "k3-256k") ?? null;
  });
  expect(stored).toMatchObject({ context_window: 262144, max_tokens: 131072 });
});

test("API access creates several models with one key, including an explicit image endpoint", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Models");
  await page.getByRole("button", { name: /Add API access/i }).click();

  await expect(page.getByTestId("provider-byok-hint")).toContainText("shared Base URL and key once");
  await page.getByLabel("Base URL").fill("https://api.openai.com");
  await expect(page.getByTestId("provider-model-row")).toHaveCount(2);
  await expect(page.getByTestId("provider-model-protocol").nth(0)).toHaveValue("openai_responses");
  await expect(page.getByTestId("provider-model-protocol").nth(1)).toHaveValue("openai_responses");
  await expect(page.getByTestId("provider-model-row").nth(0).getByLabel("Model ID")).toHaveValue("gpt-5.5");
  await expect(page.getByTestId("provider-model-row").nth(1).getByLabel("Model ID")).toHaveValue("gpt-image-2");
  await expect(page.getByTestId("provider-model-row").nth(1).getByTestId("provider-use-for-image")).toBeChecked();
  await page.getByTestId("provider-endpoint-suffix").nth(1).fill("/v1/images/generations");

  await page.getByLabel("API key (stored in OS keyring)").fill("sk-openai");
  await page.getByRole("button", { name: "Save" }).click();

  await expect.poll(() => page.evaluate(() => ((window as any).__skillInvokeLog ?? [])
    .filter((c: any) => c.cmd === "save_model")
    .map((c: any) => {
      const args = c.args instanceof Map ? Object.fromEntries(c.args) : c.args;
      const profile = args.profile instanceof Map ? Object.fromEntries(args.profile) : args.profile;
      return {
        model: profile.model,
        provider: profile.provider,
        apiUrl: profile.api_url,
        suffix: profile.endpoint_suffix,
      };
    }))).toEqual([
      {
        model: "gpt-image-2",
        provider: "openai_responses",
        apiUrl: "https://api.openai.com",
        suffix: "/v1/images/generations",
      },
      {
        model: "gpt-5.5",
        provider: "openai_responses",
        apiUrl: "https://api.openai.com",
        suffix: "",
      },
    ]);
});

test("API access reuses a stored key for the same Base URL", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Models");
  await page.getByRole("button", { name: /Add API access/i }).click();

  // Default DeepSeek URL already has a saved key in the mock list.
  await expect(page.getByTestId("provider-api-key")).toHaveAttribute(
    "placeholder",
    /api\.deepseek\.com/,
  );
  await expect(page.getByTestId("provider-separate-key-hint"))
    .toContainText("Paste a different key");
  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => {
    const args = await lastInvokeArgs(page, "save_model");
    return args ? { ...args, key: args.key ?? null } : null;
  }).toMatchObject({
    key: null,
    profile: { model: "deepseek-v4-flash" },
  });
});

test("API access can paste a second key on the same Base URL", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Models");
  await page.getByRole("button", { name: /Add API access/i }).click();

  await expect(page.getByTestId("provider-separate-key-hint")).toBeVisible();
  await page.getByTestId("provider-model-row").nth(0).getByLabel("Display name").fill("flash-work");
  await page.getByTestId("provider-model-row").nth(1).getByLabel("Display name").fill("pro-work");
  await page.getByLabel("API key (stored in OS keyring)").fill("sk-work");
  await page.getByRole("button", { name: "Save" }).click();

  await expect.poll(() => page.evaluate(() => ((window as any).__skillInvokeLog ?? [])
    .filter((c: any) => c.cmd === "save_model")
    .map((c: any) => {
      const args = c.args instanceof Map ? Object.fromEntries(c.args) : c.args;
      const profile = args.profile instanceof Map ? Object.fromEntries(args.profile) : args.profile;
      return {
        model: profile.model,
        label: profile.label,
        key: args.key ?? null,
        apiUrl: profile.api_url,
      };
    }))).toEqual([
      {
        model: "deepseek-v4-pro",
        label: "pro-work",
        key: "sk-work",
        apiUrl: "https://api.deepseek.com",
      },
      {
        model: "deepseek-v4-flash",
        label: "flash-work",
        key: "sk-work",
        apiUrl: "https://api.deepseek.com",
      },
    ]);
});

test("one DeepSeek Base URL can save Responses and Anthropic protocol models", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Models");
  await page.getByRole("button", { name: /Add API access/i }).click();

  await page.getByLabel("Base URL").fill("https://api.deepseek.com");
  const rows = page.getByTestId("provider-model-row");
  await expect(rows).toHaveCount(2);

  await rows.nth(0).getByTestId("provider-model-protocol").selectOption("openai_responses");
  await rows.nth(1).getByTestId("provider-model-protocol").selectOption("anthropic");
  await rows.nth(1).getByTestId("provider-endpoint-suffix").fill("/anthropic");
  await page.getByRole("button", { name: "Save" }).click();

  await expect.poll(() => page.evaluate(() => ((window as any).__skillInvokeLog ?? [])
    .filter((call: any) => call.cmd === "save_model")
    .map((call: any) => {
      const args = call.args instanceof Map ? Object.fromEntries(call.args) : call.args;
      const profile = args.profile instanceof Map ? Object.fromEntries(args.profile) : args.profile;
      return {
        model: profile.model,
        protocol: profile.provider,
        baseUrl: profile.api_url,
        suffix: profile.endpoint_suffix,
      };
    }))).toEqual([
      {
        model: "deepseek-v4-pro",
        protocol: "anthropic",
        baseUrl: "https://api.deepseek.com",
        suffix: "/anthropic",
      },
      {
        model: "deepseek-v4-flash",
        protocol: "openai_responses",
        baseUrl: "https://api.deepseek.com",
        suffix: "",
      },
    ]);
});

test("onboarding key setup lands on flash after adding pro", async ({ page }) => {
  await page.goto("/?mockOnboarding=1");
  await expect(page.locator(".onboard-overlay")).toBeVisible();
  await page.getByLabel("API key (stored in OS keyring)").fill("sk-onboard");
  await page.getByRole("button", { name: "Next" }).click();
  // Order matters: save_model activates each new profile, so flash must land
  // last for the user to start on the cheaper default.
  await expect.poll(() => page.evaluate(() => ((window as any).__skillInvokeLog ?? [])
    .filter((c: any) => c.cmd === "save_model")
    .map((c: any) => {
      const args = c.args instanceof Map ? Object.fromEntries(c.args) : c.args;
      const profile = args.profile instanceof Map ? Object.fromEntries(args.profile) : args.profile;
      return profile.model;
    }))).toEqual(["deepseek-v4-pro", "deepseek-v4-flash"]);
  // The built-in Reader gets bound to the flash profile so reading-heavy
  // work runs on the cheap tier out of the box.
  await expect.poll(() => page.evaluate(() => ((window as any).__skillInvokeLog ?? [])
    .filter((c: any) => c.cmd === "save_specialist_cmd")
    .map((c: any) => {
      const args = c.args instanceof Map ? Object.fromEntries(c.args) : c.args;
      const spec = args.spec instanceof Map ? Object.fromEntries(args.spec) : args.spec;
      return { id: spec.id, model_id: spec.model_id };
    }))).toEqual([{ id: "reader", model_id: "m2" }]);
});

test("API access on xAI suggests grok chat and imagine image", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Models");
  await page.getByRole("button", { name: /Add API access/i }).click();
  await page.getByLabel("Base URL").fill("https://api.x.ai");
  await expect(page.getByTestId("provider-model-row")).toHaveCount(2);
  await expect(page.getByTestId("provider-model-row").nth(0).getByLabel("Model ID")).toHaveValue("grok-4.6");
  await expect(page.getByTestId("provider-model-row").nth(1).getByLabel("Model ID")).toHaveValue("grok-imagine-image-2.0");
  await expect(page.getByTestId("provider-model-row").nth(1).getByTestId("provider-use-for-image")).toBeChecked();
});

test("gpt-image-2 can be assigned for generation but not selected for chat", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Models");
  const opus = page.locator(".settings-list-row", { hasText: "opus-4.8" });
  await opus.click();

  await providerSelect(page).selectOption("openai_responses");
  await page.getByLabel("Base URL").fill("https://api.openai.com/v1");
  await page.getByLabel("Model").fill("gpt-image-2");
  await expect(page.getByLabel("Max output tokens")).toHaveCount(0);
  await expect(page.getByLabel("Supports image input")).toHaveCount(0);
  await expect(page.getByTestId("image-size")).toBeVisible();
  await expect(page.getByTestId("image-quality")).toBeVisible();
  await page.getByTestId("image-size").selectOption("1536x1024");
  await page.getByTestId("image-quality").selectOption("high");
  await expect(page.getByTestId("use-for-image-generation")).toBeChecked();
  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toHaveText(
    "Validated openai_responses with gpt-image-2",
  );
  await page.getByRole("button", { name: "Save" }).click();

  await expect.poll(() => lastInvokeArgs(page, "save_model")).toMatchObject({
    useForImageGeneration: true,
    profile: {
      id: "opus",
      provider: "openai_responses",
      model: "gpt-image-2",
      use_for_image_generation: true,
      image_size: "1536x1024",
      image_quality: "high",
    },
  });

  const imageModel = page.locator(".settings-list-row", { hasText: "opus-4.8" });
  await expect(imageModel).toContainText("gpt-image-2");
  await expect(imageModel).toContainText("image gen");
  await expect(imageModel.getByRole("button", { name: "Use" })).toHaveCount(0);

  await page.locator(".settings-head-close").click();
  await page.locator(".model-picker-btn").click();
  await expect(page.locator(".model-menu")).not.toContainText("gpt-image-2");
  await expect(page.locator(".model-menu")).toContainText("deepseek-v4-pro");
});

test("grok-imagine-image-2.0 can be assigned for generation but not selected for chat", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Models");
  const opus = page.locator(".settings-list-row", { hasText: "opus-4.8" });
  await opus.click();

  await providerSelect(page).selectOption("openai");
  await page.getByLabel("Base URL").fill("https://api.x.ai");
  await page.getByLabel("Model").fill("grok-imagine-image-2.0");
  await expect(page.getByLabel("Max output tokens")).toHaveCount(0);
  await expect(page.getByLabel("Supports image input")).toHaveCount(0);
  await expect(page.getByTestId("image-aspect-ratio")).toBeVisible();
  await expect(page.getByTestId("image-resolution")).toBeVisible();
  await expect(page.getByTestId("image-quality")).toBeVisible();
  await expect(page.getByTestId("image-size")).toHaveCount(0);
  await page.getByTestId("image-aspect-ratio").selectOption("16:9");
  await page.getByTestId("image-resolution").selectOption("2k");
  await page.getByTestId("image-quality").selectOption("low");
  await expect(page.getByTestId("use-for-image-generation")).toBeChecked();
  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toHaveText(
    "Validated openai with grok-imagine-image-2.0",
  );
  await page.getByRole("button", { name: "Save" }).click();

  await expect.poll(() => lastInvokeArgs(page, "save_model")).toMatchObject({
    useForImageGeneration: true,
    profile: {
      id: "opus",
      provider: "openai",
      model: "grok-imagine-image-2.0",
      use_for_image_generation: true,
      image_aspect_ratio: "16:9",
      image_resolution: "2k",
      image_quality: "low",
    },
  });

  const imageModel = page.locator(".settings-list-row", { hasText: "opus-4.8" });
  await expect(imageModel).toContainText("grok-imagine-image-2.0");
  await expect(imageModel).toContainText("image gen");
  await expect(imageModel.getByRole("button", { name: "Use" })).toHaveCount(0);

  await page.locator(".settings-head-close").click();
  await page.locator(".model-picker-btn").click();
  await expect(page.locator(".model-menu")).not.toContainText("grok-imagine-image-2.0");
  await expect(page.locator(".model-menu")).toContainText("deepseek-v4-pro");
});

test("grok-imagine-video can be assigned for generation but not selected for chat", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Models");
  const opus = page.locator(".settings-list-row", { hasText: "opus-4.8" });
  await opus.click();

  await providerSelect(page).selectOption("openai");
  await page.getByLabel("Base URL").fill("https://api.x.ai");
  await page.getByLabel("Model").fill("grok-imagine-video");
  await expect(page.getByLabel("Max output tokens")).toHaveCount(0);
  await expect(page.getByLabel("Supports image input")).toHaveCount(0);
  await expect(page.getByTestId("video-duration")).toBeVisible();
  await expect(page.getByTestId("video-aspect-ratio")).toBeVisible();
  await expect(page.getByTestId("video-resolution")).toBeVisible();
  await expect(page.getByTestId("image-size")).toHaveCount(0);
  await page.getByTestId("video-duration").fill("8");
  await page.getByTestId("video-aspect-ratio").selectOption("9:16");
  await page.getByTestId("video-resolution").selectOption("1080p");
  await expect(page.getByTestId("use-for-video-generation")).toBeChecked();
  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toHaveText(
    "Validated openai with grok-imagine-video",
  );
  await page.getByRole("button", { name: "Save" }).click();

  await expect.poll(() => lastInvokeArgs(page, "save_model")).toMatchObject({
    useForVideoGeneration: true,
    profile: {
      id: "opus",
      provider: "openai",
      model: "grok-imagine-video",
      use_for_video_generation: true,
      video_duration_secs: 8,
      video_aspect_ratio: "9:16",
      video_resolution: "1080p",
    },
  });

  const videoModel = page.locator(".settings-list-row", { hasText: "opus-4.8" });
  await expect(videoModel).toContainText("grok-imagine-video");
  await expect(videoModel).toContainText("video gen");
  await expect(videoModel.getByRole("button", { name: "Use" })).toHaveCount(0);

  await page.locator(".settings-head-close").click();
  await page.locator(".model-picker-btn").click();
  await expect(page.locator(".model-menu")).not.toContainText("grok-imagine-video");
  await expect(page.locator(".model-menu")).toContainText("deepseek-v4-pro");
});

test("settings normalizes a blank stored provider to openai", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await expect(providerSelect(page)).toHaveValue("openai");
  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toContainText("Validated openai with deepseek-v4-pro");
});

test("editing Base URL keeps protocol state and display aligned", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await page.getByLabel("Base URL").fill("https://api.deepseek.com");
  await expect(providerSelect(page)).toHaveValue("openai");
  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toContainText("Validated openai with deepseek-v4-pro");
});

test("model Base URL explains per-model endpoint suffixes", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);

  await expect(page.getByTestId("model-api-url-hint")).toHaveText(
    "Enter the shared API root. Put protocol-specific paths such as /anthropic or an explicit image endpoint in that model's optional suffix; Wisp then completes the protocol request path.",
  );
  await expect(page.getByLabel("Base URL")).toHaveAttribute(
    "aria-describedby",
    "model-api-url-hint",
  );
});

test("settings can validate current API config", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await page.getByRole("button", { name: "Valid" }).click();
  // The mock profile has "supports images" on, so validation probes with a
  // test image and says so.
  await expect(page.locator(".settings-status")).toHaveText(
    "Validated openai with deepseek-v4-pro — the test image was accepted.",
  );

  await page.getByLabel("Supports image input").uncheck();
  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toHaveText("Validated openai with deepseek-v4-pro");
});

test("editing a saved model validates with that model profile id", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  await page.getByRole("button", { name: "Models" }).click();
  await page.locator(".settings-list-row", { hasText: "opus-4.8" }).click();
  await expect(providerSelect(page)).toBeVisible();
  await expect(page.getByLabel("Model ID")).toHaveValue("opus-4.8");

  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toContainText("Validated openai with deepseek-v4-pro");
  await expect.poll(() => lastInvokeArgs(page, "validate_settings")).toMatchObject({
    profileId: "opus",
    key: "",
    settings: {
      model: "opus-4.8",
    },
  });
});

test("check for updates shows an up-to-date modal", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  await setMockUpdateCheck(page, {
    current_version: "0.9.0",
    latest_version: "0.9.0",
    update_available: false,
  });

  await page.getByRole("button", { name: "Check for updates" }).click();
  const modal = page.getByTestId("update-check-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("You're up to date");
  await expect(modal).toContainText("Wisp 0.9.0 is already the latest version.");
  await modal.getByRole("button", { name: "OK" }).click();
  await expect(modal).toHaveCount(0);
});

test("check for updates shows an available-update modal before opening releases", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  await setMockUpdateCheck(page, {
    current_version: "0.9.0",
    latest_version: "1.2.3",
    update_available: true,
    release_url: "https://github.com/xuzhougeng/wisp-science/releases/tag/v1.2.3",
  });

  await page.getByRole("button", { name: "Check for updates" }).click();
  const modal = page.getByTestId("update-check-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("Update available");
  await expect(modal).toContainText("Wisp 1.2.3 is available.");
  await expect(await lastInvokeArgs(page, "open_external_url")).toBeNull();
  await page.getByTestId("update-check-open-releases").click();
  await expect(modal).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "open_external_url")).toMatchObject({
    url: "https://github.com/xuzhougeng/wisp-science/releases/tag/v1.2.3",
  });
});

test("macOS update download is verified before a separate install confirmation", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  await setMockUpdateCheck(page, {
    current_version: "0.27.0",
    latest_version: "0.28.0",
    update_available: true,
    release_url: "https://github.com/xuzhougeng/wisp-science/releases/tag/v0.28.0",
    notes: "Signed macOS update",
    install_supported: true,
  });
  await setMockUpdateDownload(page, { pending: true });

  await page.getByRole("button", { name: "Check for updates" }).click();
  const modal = page.getByTestId("update-check-modal");
  await expect(modal.getByRole("button", { name: "Download update" })).toBeVisible();
  await expect(await lastInvokeArgs(page, "install_update")).toBeNull();

  await modal.getByRole("button", { name: "Download update" }).click();
  await expect(modal).toContainText("Downloading Wisp 0.28.0");
  await expect(modal).toContainText("25 B / 100 B");
  await modal.evaluate((element) => ((window as any).__downloadingModal = element));
  await page.evaluate(() => (window as any).__mockUpdateProgress(10));
  await expect(modal).toContainText("35 B / 100 B");
  expect(await modal.evaluate((element) => element === (window as any).__downloadingModal)).toBe(true);

  // An in-flight update owns the top of the Escape stack and keeps Settings open.
  await page.keyboard.press("Escape");
  await expect(modal).toBeVisible();
  await expect(page.locator(".settings-page")).toBeVisible();

  await resolveMockUpdateDownload(page);
  await expect(modal).toContainText("Ready to install");
  await expect(modal).toContainText("signature verified");
  await expect(await lastInvokeArgs(page, "install_update")).toBeNull();

  await modal.getByRole("button", { name: "Install and restart" }).click();
  await expect(modal).toContainText("Installing Wisp 0.28.0");
  await expect.poll(() => page.evaluate(() => (window as any).__mockUpdateInstalled)).toBe(true);
});

test("update signature failure keeps the current app and offers Releases", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  const releaseUrl =
    "https://github.com/xuzhougeng/wisp-science/releases/tag/v0.28.0";
  await setMockUpdateCheck(page, {
    latest_version: "0.28.0",
    update_available: true,
    release_url: releaseUrl,
    install_supported: true,
  });
  await setMockUpdateDownload(page, {
    error: "Update download or signature verification failed: signature verification failed",
  });

  await page.getByRole("button", { name: "Check for updates" }).click();
  const modal = page.getByTestId("update-check-modal");
  await modal.getByRole("button", { name: "Download update" }).click();
  await expect(modal).toContainText("signature verification failed");
  expect(await page.evaluate(() => (window as any).__mockUpdateInstalled)).toBe(false);

  await modal.getByTestId("update-check-open-releases").click();
  await expect.poll(() => lastInvokeArgs(page, "open_external_url")).toMatchObject({
    url: releaseUrl,
  });
});

test("update check failure offers the Releases fallback", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  await setMockUpdateCheckError(
    page,
    "Failed to check for a signed update: no matching macOS architecture",
  );

  await page.getByRole("button", { name: "Check for updates" }).click();
  const modal = page.getByTestId("update-check-modal");
  await expect(modal).toContainText("no matching macOS architecture");
  await expect(modal.getByTestId("update-check-open-releases")).toBeVisible();
});

test("install is blocked while a task is active", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  await setMockUpdateCheck(page, {
    latest_version: "0.28.0",
    update_available: true,
    release_url: "https://github.com/xuzhougeng/wisp-science/releases/tag/v0.28.0",
    install_supported: true,
  });

  await page.getByRole("button", { name: "Check for updates" }).click();
  const modal = page.getByTestId("update-check-modal");
  await modal.getByRole("button", { name: "Download update" }).click();
  await expect(modal).toContainText("Ready to install");
  await setMockInstallUpdateError(
    page,
    "Wait for every task and run to finish before installing the update.",
  );

  await modal.getByRole("button", { name: "Install and restart" }).click();
  await expect(modal).toContainText(
    "Wait for every task and run to finish before installing the update.",
  );
  expect(await page.evaluate(() => (window as any).__mockUpdateInstalled)).toBe(false);
});

test("Escape closes only the available-update modal above Settings", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  await setMockUpdateCheck(page, {
    latest_version: "0.28.0",
    update_available: true,
    install_supported: true,
  });

  await page.getByRole("button", { name: "Check for updates" }).click();
  await expect(page.getByTestId("update-check-modal")).toBeVisible();
  await page.keyboard.press("Escape");

  await expect(page.getByTestId("update-check-modal")).toHaveCount(0);
  await expect(page.locator(".settings-page")).toBeVisible();
});

test("stale update card refreshes to the latest release before opening it (#521)", async ({ page }) => {
  await enterApp(page);
  await setMockUpdateCheck(page, {
    current_version: "0.23.0",
    latest_version: "0.24.0",
    update_available: true,
    release_url: "https://github.com/xuzhougeng/wisp-science/releases/tag/v0.24.0",
  });

  await page.keyboard.press("Control+p");
  const input = page.locator("#action-palette-input");
  await input.fill("check for updates");
  await input.press("Enter");
  let modal = page.getByTestId("update-check-modal");
  await expect(modal).toContainText("Wisp 0.24.0 is available.");
  await modal.getByRole("button", { name: "Later" }).click();
  await expect(page.getByTestId("update-card")).toContainText("v0.24.0");

  await setMockUpdateCheck(page, {
    latest_version: "0.25.0",
    release_url: "https://github.com/xuzhougeng/wisp-science/releases/tag/v0.25.0",
  });
  await page.getByTestId("update-card").click();

  modal = page.getByTestId("update-check-modal");
  await expect(modal).toContainText("Wisp 0.25.0 is available.");
  await expect(page.getByTestId("update-card")).toContainText("v0.25.0");
  await modal.getByTestId("update-check-open-releases").click();
  await expect.poll(() => lastInvokeArgs(page, "open_external_url")).toMatchObject({
    url: "https://github.com/xuzhougeng/wisp-science/releases/tag/v0.25.0",
  });
});

test("command palette check for updates also shows the result modal", async ({ page }) => {
  await enterApp(page);
  await setMockUpdateCheck(page, {
    current_version: "0.9.0",
    latest_version: "0.9.0",
    update_available: false,
  });

  await page.keyboard.press("Control+p");
  const input = page.locator("#action-palette-input");
  await input.fill("check for updates");
  await input.press("Enter");

  const modal = page.getByTestId("update-check-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("You're up to date");
});

test("command palette click shows checking feedback immediately", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".proj-card-main")).not.toHaveCount(0);
  await setMockUpdateCheckPending(page, true);

  await page.keyboard.press("Control+p");
  await page.getByRole("button", { name: "Check for updates" }).click();

  const modal = page.getByTestId("update-check-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("Checking for updates");
  await expect(modal).toContainText("Contacting GitHub Releases");
  await resolveMockUpdateCheck(page);
  await expect(modal).toContainText("You're up to date", { timeout: 2_000 });
});

test("credential services explain their behavior and open official setup links", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Credentials");

  await expect(page.locator(".cred-help-trigger")).toHaveCount(4);
  const openAlexHelp = page.getByRole("button", { name: "OpenAlex: About this credential" });
  const openAlexTooltip = page.locator("#cred-help-openalex");
  await expect(openAlexTooltip).not.toBeVisible();
  await openAlexHelp.hover();
  await expect(openAlexTooltip).toBeVisible();
  await expect(openAlexTooltip).toContainText("What it is");
  await expect(openAlexTooltip).toContainText("When configured");
  await expect(openAlexTooltip).toContainText("When not configured");
  await expect(openAlexTooltip).toContainText("falls back to anonymous OpenAlex requests");

  const ncbiHelp = page.getByRole("button", { name: "NCBI E-utilities (PubMed): About this credential" });
  await ncbiHelp.focus();
  await expect(page.locator("#cred-help-ncbi")).toBeVisible();
  await expect(page.locator("#cred-help-ncbi")).toContainText("3 requests/s limit");

  const links = [
    ["Get OpenAlex API key", "https://openalex.org/settings/api"],
    ["Open InfiniSynapse console", "https://app.infinisynapse.cn/tasks"],
    ["Visit InfiniSynapse", "https://infinisynapse.cn"],
    ["Get SCIMaster API key", "https://scimaster.bohrium.com/vibe-write/home"],
    ["Open NCBI account", "https://www.ncbi.nlm.nih.gov/account/"],
  ] as const;
  for (const [label, url] of links) {
    await page.getByRole("button", { name: label, exact: true }).click();
    await expect.poll(() => lastInvokeArgs(page, "open_external_url")).toMatchObject({ url });
  }
  await expect(page.locator(".settings-page")).toBeVisible();
});

test("Chinese NCBI credential help gives the complete account navigation path", async ({ page }) => {
  await page.goto("/?mockLocale=zh");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator(".sidebar").getByRole("button", { name: "新建会话" })).toBeVisible();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.getByRole("button", { name: "凭据", exact: true }).click();

  const help = page.getByRole("button", { name: /NCBI E-utilities.*了解该凭据/ });
  await help.focus();
  const tooltip = page.locator("#cred-help-ncbi");
  await expect(tooltip).toBeVisible();
  await expect(tooltip).toContainText("这是什么");
  await expect(tooltip).toContainText("配置后");
  await expect(tooltip).toContainText("未配置时");
  await expect(tooltip).toContainText("每秒 3 次");
  await expect(page.locator(".cred-setup-note", { hasText: "API Key Management" }))
    .toContainText("右上角用户名");
  await expect(page.getByRole("button", { name: "打开 NCBI 账户页面", exact: true }))
    .toBeVisible();
});

test("credentials settings include SCIMaster and save its key", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Credentials");
  const field = page.locator("label", { hasText: "SCIMaster API key" });
  await expect(field).toContainText("Not configured");
  await field.locator("input").fill("sk-sci-123");
  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_credential")).toMatchObject({
    id: "scimaster_api_key",
    value: "sk-sci-123",
  });
  await expect(page.locator(".settings-status")).toHaveText("Saved. Applies to new sessions.");
  await expect(field).toContainText("Configured");
});

test("credentials settings add, replace, clear, and remove a custom credential", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Credentials");

  await page.getByLabel("Service name").fill("MetaSo");
  await page.getByLabel("Environment variable").fill("METASO_API_KEY");
  await page.getByLabel("Credential value").fill("meta-secret");
  await page.getByRole("button", { name: "Add credential", exact: true }).click();

  await expect.poll(() => lastInvokeArgs(page, "add_custom_credential")).toMatchObject({
    name: "MetaSo",
    envVar: "METASO_API_KEY",
    value: "meta-secret",
  });
  const card = page.locator('[data-custom-credential="METASO_API_KEY"]');
  await expect(card).toContainText("MetaSo");
  await expect(card).toContainText("Configured");
  await expect(card).not.toContainText("meta-secret");

  await card.locator('input[type="password"]').fill("replacement-secret");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_credential")).toMatchObject({
    id: "custom-1",
    value: "replacement-secret",
  });

  await card.getByRole("button", { name: "Clear", exact: true }).click();
  await expect(card).toContainText("Not configured");
  await card.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(card).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "remove_custom_credential"))
    .toMatchObject({ id: "custom-1" });
});

test("capability counts open skills, connections, and editable project memory", async ({ page }) => {
  await enterApp(page);

  await page.getByRole("button", { name: "Capabilities" }).click();
  let capabilities = page.getByRole("dialog", { name: "Capabilities" });
  await expect(capabilities.getByRole("button", { name: "2 Bundled Skills" })).toBeVisible();
  await capabilities.getByRole("button", { name: "1 Project Skills" }).click();
  await expect(page.locator(".settings-nav button.active")).toHaveText("Skills");

  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Capabilities" }).click();
  capabilities = page.getByRole("dialog", { name: "Capabilities" });
  await expect(capabilities.getByRole("button", { name: "2 Bundled MCP" })).toBeVisible();
  await capabilities.getByRole("button", { name: "1 Project MCP" }).click();
  await expect(page.locator(".settings-nav button.active")).toHaveText("Connections");

  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Capabilities" }).click();
  capabilities = page.getByRole("dialog", { name: "Capabilities" });
  await capabilities.getByRole("button", { name: "1 Memory files" }).click();
  await expect(page.locator(".settings-nav button.active")).toHaveText("Memory");
  await expect(page.getByText("2026-07-01.md", { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Add note" }).click();
  const editor = page.locator(".memory-editor-text");
  await expect(editor).toHaveValue("");
  await editor.fill("Prefer reproducible local workflows.");
  await page.getByRole("button", { name: "Save note" }).click();
  await expect.poll(() => lastInvokeArgs(page, "write_memory_file")).toMatchObject({
    name: "2026-07-04.md",
    content: "Prefer reproducible local workflows.",
  });

  await page.locator(".settings-head-back").click();
  await page.getByText("2026-07-04.md", { exact: true }).click();
  await expect(editor).toHaveValue("Prefer reproducible local workflows.");
  await editor.fill("Prefer editable, reproducible local workflows.");
  await page.getByRole("button", { name: "Save note" }).click();
  await expect.poll(() => lastInvokeArgs(page, "write_memory_file")).toMatchObject({
    name: "2026-07-04.md",
    content: "Prefer editable, reproducible local workflows.",
  });
});

test("skill manager filters by tag and batch disables visible skills", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Add to message" }).click();
  await page.getByRole("button", { name: "Manage skills" }).click();
  await expect(page.getByRole("button", { name: "Skills", exact: true })).toBeVisible();
  await expect(page.locator(".settings-search")).toHaveAttribute("type", "text");
  await expect(page.locator(".settings-search")).toHaveAttribute("inputmode", "search");
  await expect(page.locator(".settings-search")).toHaveAttribute("autocomplete", "off");
  await expect(page.locator(".settings-filter")).toContainText(/visible.*enabled/);
  await expect(page.locator(".skill-tags-editor").first()).not.toHaveAttribute("open", "");

  await page.getByRole("button", { name: "Disabled", exact: true }).click();
  await expect(page.getByText("No skills match the current filters.")).toBeVisible();
  await expect(page.locator("[data-skill-name]")).toHaveCount(0);

  await page.getByRole("button", { name: "Enabled", exact: true }).click();
  await expect(page.getByText("literature-review")).toBeVisible();
  await expect(page.getByText("remote-compute-ssh")).toBeVisible();

  await page.getByRole("button", { name: "compute", exact: true }).click();
  await expect(page.getByText("remote-compute-ssh")).toBeVisible();
  await expect(page.getByText("literature-review")).not.toBeVisible();

  await page.getByRole("button", { name: "Disable visible" }).click();
  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "set_skills_enabled");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : (args ?? null);
  })).toEqual({ names: ["remote-compute-ssh"], enabled: false });
  await expect(page.locator('[data-skill-name="remote-compute-ssh"] input[type="checkbox"]')).not.toBeChecked();
});

test("skill manager reloads manually copied skills and shows their scope", async ({ page }) => {
  await enterApp(page, "/?mockSkillReload=1");
  await openSettingsSection(page, "Skills");

  await expect(page.locator('[data-skill-name="fresh-project-skill"]')).toHaveCount(0);
  await expect(page.locator('[data-skill-name="paper-narrative"]')).toContainText("Global");
  await page.getByRole("button", { name: "Reload skills" }).click();

  const fresh = page.locator('[data-skill-name="fresh-project-skill"]');
  await expect.poll(() => lastInvokeArgs(page, "reload_skills")).toEqual({});
  await expect(fresh).toContainText("Newly copied project skill");
  await expect(fresh).toContainText("Project");
  await expect(fresh.locator('input[type="checkbox"]')).toBeChecked();
  await expect(fresh.getByRole("button", { name: "Delete skill" })).toHaveCount(0);
  await expect(page.getByText("Skills reloaded. 5 available.")).toBeVisible();
});

test("skill manager updates and deletes user-added skills", async ({ page }) => {
  await enterApp(page, "/?mockSkillImport=1");
  await openSettingsSection(page, "Skills");

  await page.getByText("Add skill", { exact: true }).click();
  const addFile = page.getByRole("button", { name: "Add SKILL.md or ZIP" });
  await page.keyboard.press("Escape");
  await expect(addFile).not.toBeVisible();
  await expect(page.locator(".settings-page")).toBeVisible();

  await page.getByText("Add skill", { exact: true }).click();
  await addFile.click();
  await expect.poll(() => lastInvokeArgs(page, "install_skill")).toMatchObject({
    srcPath: "/downloads/paper-narrative.zip",
  });
  await expect(page.getByText("Skill added or updated.")).toBeVisible();

  const skill = page.locator('[data-skill-name="paper-narrative"]');
  await expect(skill.getByRole("button", { name: "Delete skill" })).toBeVisible();
  await skill.getByRole("button", { name: "Delete skill" }).click();
  const confirm = page.getByTestId("skill-remove-confirm");
  await expect(confirm).toContainText(
    "Delete paper-narrative? Its installed files will be removed. This cannot be undone.",
  );
  await confirm.getByRole("button", { name: "Delete skill" }).click();

  await expect.poll(() => lastInvokeArgs(page, "remove_skill")).toEqual({
    name: "paper-narrative",
  });
  await expect(page.locator('[data-skill-name="paper-narrative"]')).toHaveCount(0);
  await expect(page.getByText("Skill deleted.")).toBeVisible();
});

test("plugin settings diagnose, launch, install, and remove a feature plugin", async ({ page }) => {
  await enterApp(page, "/?mockPluginImport=1");
  await openSettingsSection(page, "Plugins");

  const row = page.locator('[data-plugin-id="motif-for-claude-science"]');
  await expect(row).toContainText("Motif for Claude Science");
  await expect(row).toContainText("Runtime ready");

  // Verification / skill / MCP details live in the expandable "Details" panel.
  await row.getByText("Details").click();
  await expect(row).toContainText("checksum_verified");
  await expect(row).toContainText("MCP servers");
  const [stateBox, detailGridBox] = await Promise.all([
    row.locator(".plugin-state-line").boundingBox(),
    row.locator(".plugin-detail-grid").boundingBox(),
  ]);
  expect(stateBox).not.toBeNull();
  expect(detailGridBox).not.toBeNull();
  expect(detailGridBox!.y).toBeGreaterThanOrEqual(stateBox!.y + stateBox!.height);

  const toggle = row.locator('input[type="checkbox"]');
  await expect(toggle).not.toBeChecked();
  await row.getByRole("button", { name: "Enable & use" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_plugin_enabled")).toMatchObject({
    pluginId: "motif-for-claude-science",
    version: "0.2.1",
    enabled: true,
  });
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    message: expect.stringContaining("Its skill guidance is attached to this message"),
    // The managed Skill fixture lives under the different hypothesis-review slug.
    references: [{ kind: "skill", name: "motif-for-claude-science" }],
  });
  await expect(page.locator(".settings-page")).toHaveCount(0);

  // The guided launch creates the required fresh session. The plugin-owned
  // Skill is visible under Skills as read-only provenance, not as another
  // plugin-management UI.
  await openSettingsSection(page, "Skills");
  await expect(page.locator('[data-plugin-id="motif-for-claude-science"]')).toHaveCount(0);
  const managedSkill = page.locator('[data-skill-name="motif-for-claude-science"]');
  await expect(managedSkill).toContainText("Managed by Motif for Claude Science");
  await expect(managedSkill.locator('input[type="checkbox"]')).toHaveCount(0);

  await page.getByRole("button", { name: "Plugins", exact: true }).click();
  const enabledRow = page.locator('[data-plugin-id="motif-for-claude-science"]');
  const enabledToggle = enabledRow.locator('input[type="checkbox"]');
  await expect(enabledToggle).toBeChecked();

  // The install dialog is above Settings in the Escape stack.
  await page.getByRole("button", { name: "Install plugin", exact: true }).click();
  let section = page.getByTestId("plugin-settings");
  await page.keyboard.press("Escape");
  await expect(section).toHaveCount(0);
  await expect(page.locator(".settings-page")).toBeVisible();

  // A local install first records the selected ZIP, then requires a separate
  // install action.
  await page.getByRole("button", { name: "Install plugin", exact: true }).click();
  section = page.getByTestId("plugin-settings");
  const localInstall = section.getByRole("button", { name: "Install plugin", exact: true });
  await expect(localInstall).toBeDisabled();
  await section.getByRole("button", { name: "Choose ZIP", exact: true }).click();
  await expect(section.getByRole("textbox", { name: "Plugin ZIP" }))
    .toHaveValue("/downloads/motif-update.zip");
  await expect.poll(() => lastInvokeArgs(page, "install_plugin")).toBeNull();
  await expect(localInstall).toBeEnabled();
  await localInstall.click();
  await expect.poll(() => lastInvokeArgs(page, "install_plugin")).toMatchObject({
    srcPath: "/downloads/motif-update.zip",
    expectedSha256: undefined,
  });
  await expect(section).toHaveCount(0);

  // Remote releases still require a full checksum before install is enabled.
  await page.getByRole("button", { name: "Install plugin", exact: true }).click();
  section = page.getByTestId("plugin-settings");
  await section.getByRole("tab", { name: "Release URL" }).click();
  await section.locator('input[type="url"]').fill("https://example.test/motif.zip");
  await section.locator('input[placeholder*="64 hexadecimal"]').fill("b".repeat(64));
  await section.getByRole("button", { name: "Download & install" }).click();
  await expect.poll(() => lastInvokeArgs(page, "install_plugin_url")).toMatchObject({
    sourceUrl: "https://example.test/motif.zip",
    expectedSha256: "b".repeat(64),
  });
  await expect(section).toHaveCount(0);

  await enabledRow.getByTitle("Remove").click();
  const removeConfirm = page.getByTestId("plugin-remove-confirm");
  await expect(removeConfirm).toContainText("Motif for Claude Science");
  await expect.poll(() => lastInvokeArgs(page, "remove_plugin")).toBeNull();
  await removeConfirm.getByRole("button", { name: "Cancel" }).click();
  await expect(enabledRow).toBeVisible();
  await enabledRow.getByTitle("Remove").click();
  await page.getByTestId("plugin-remove-confirm")
    .getByRole("button", { name: "Remove plugin" }).click();
  await expect.poll(() => lastInvokeArgs(page, "remove_plugin")).toMatchObject({
    pluginId: "motif-for-claude-science",
    version: "0.2.1",
  });
  await expect(row).toHaveCount(0);
});

test("custom MCP row opens tools while edit uses a dedicated button", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  await page.getByRole("button", { name: "Connections" }).click();
  await expect(page.getByRole("button", { name: "Connect Notion" })).toHaveCount(0);

  const row = page.locator(".settings-list-row", { hasText: "wolai_cmp" });
  await row.click();
  await expect(page.getByText("wolai_search")).toBeVisible();
  await expect(page.getByText("Search Wolai pages")).toBeVisible();

  await page.locator(".settings-head-back").click();
  await row.getByRole("button", { name: "Edit connection" }).click();
  await expect(page.getByLabel("Name")).toHaveValue("wolai_cmp");
  await expect(page.getByPlaceholder("https://host/mcp")).toHaveValue("https://api.wolai.com/v1/mcp/");
});

test("Notion uses the generic Remote URL OAuth connection flow", async ({ page }) => {
  await enterApp(page);
  await globalSettingsButton(page).click();
  await page.getByRole("button", { name: "Connections" }).click();

  await expect.poll(() => lastInvokeArgs(page, "authorize_http_connection")).toBeNull();
  await expect.poll(() => lastInvokeArgs(page, "test_oauth_mcp_connection")).toBeNull();
  await page.getByRole("button", { name: "Add connection" }).click();
  const type = page.getByLabel("Type");
  await expect(type.locator("option")).toHaveCount(2);
  await expect(type.locator('option[value="notion"]')).toHaveCount(0);

  await page.getByLabel("Name").fill("Notion");
  await type.selectOption("http");
  await page.getByPlaceholder("https://host/mcp").fill("https://mcp.notion.com/mcp");
  await page.getByLabel("Authentication").selectOption("oauth");
  await expect(page.getByText("Testing does not save the connection.")).toBeVisible();

  await page.getByRole("button", { name: "Test" }).click();
  await expect.poll(() => lastInvokeArgs(page, "test_oauth_mcp_connection")).toMatchObject({
    conn: {
      name: "Notion",
      transport: {
        kind: "http",
        url: "https://mcp.notion.com/mcp",
        auth: "oauth",
      },
    },
  });
  await expect(page.locator(".settings-status")).toHaveText("OK — 2 tools");
  await expect.poll(() => lastInvokeArgs(page, "authorize_http_connection")).toBeNull();
  await expect(page.getByLabel("Name")).toHaveValue("Notion");

  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "authorize_http_connection")).toMatchObject({
    conn: {
      name: "Notion",
      enabled: true,
      transport: {
        kind: "http",
        url: "https://mcp.notion.com/mcp",
        auth: "oauth",
      },
    },
  });
  const row = page.locator(".settings-list-row", { hasText: "Notion" });
  await expect(row).toContainText("https://mcp.notion.com/mcp");
  await expect(row).toContainText("OAuth");
  await expect(row).toContainText("Enabled");

  await row.click();
  await expect(page.getByText("Service", { exact: true })).toBeVisible();
  await expect(page.getByText("https://mcp.notion.com/mcp", { exact: true })).toBeVisible();
  await expect(page.getByText("Status", { exact: true })).toBeVisible();
  await expect(page.getByText("Enabled", { exact: true })).toBeVisible();
  await expect(page.getByText("Authentication", { exact: true })).toBeVisible();
  await expect(page.getByText("OAuth", { exact: true })).toBeVisible();
});

test("OAuth authorization keeps Cancel available and clears form status", async ({ page }) => {
  await enterApp(page, "/?mockOAuthPending=1");
  await globalSettingsButton(page).click();
  await page.getByRole("button", { name: "Connections" }).click();
  await page.getByRole("button", { name: "Add connection" }).click();
  await page.getByLabel("Name").fill("Hosted MCP");
  await page.getByLabel("Type").selectOption("http");
  await page.getByPlaceholder("https://host/mcp").fill("https://example.com/mcp");
  await page.getByLabel("Authentication").selectOption("oauth");
  await expect(page.getByPlaceholder("X-Custom-Header")).toBeVisible();

  await page.getByRole("button", { name: "Test" }).click();
  await expect(page.getByText("Complete authorization in your browser…")).toBeVisible();
  const cancel = page.getByRole("button", { name: "Cancel" });
  await expect(cancel).toBeEnabled();
  await cancel.click();
  await expect(page.getByRole("button", { name: "Add connection" })).toBeVisible();
  await expect.poll(async () => (await invokeArgsList(page, "cancel_oauth_authorization")).length).toBe(1);

  await page.evaluate(() => (window as any).__resolveMockOAuth());
  await expect(page.locator(".settings-status")).toHaveCount(0);
});

test("settings validation rejects blank required fields", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await page.getByLabel("Base URL").fill("");
  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toHaveText("Validation failed: API URL is required.");
});

test("protocol switch keeps the current Base URL, endpoint suffix, and model ID", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);

  const baseUrl = page.getByLabel("Base URL");
  const endpointSuffix = page.getByLabel("Endpoint suffix (optional)");
  const model = page.getByLabel("Model ID");
  await endpointSuffix.fill("/anthropic");

  await providerSelect(page).selectOption("openai_responses");
  await expect(baseUrl).toHaveValue("https://api.deepseek.com");
  await expect(endpointSuffix).toHaveValue("/anthropic");
  await expect(model).toHaveValue("deepseek-v4-pro");

  await providerSelect(page).selectOption("anthropic");
  await expect(baseUrl).toHaveValue("https://api.deepseek.com");
  await expect(endpointSuffix).toHaveValue("/anthropic");
  await expect(model).toHaveValue("deepseek-v4-pro");
});

test("model form inputs keep focus while typing (#62)", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  const model = page.getByLabel("Model");
  await model.fill("");
  // Type character-by-character. The bug: the form pane was gated on the whole
  // model_form signal, so each keystroke rebuilt the inputs and dropped focus —
  // only the first character survived. After the fix the field stays mounted.
  await model.pressSequentially("gpt-5.5-x");
  await expect(model).toHaveValue("gpt-5.5-x");
  await expect(model).toBeFocused();

  // The provider-level add form has a separate multi-model editor. Its
  // edit/add view gate must also remain stable as a model row changes.
  await page.getByRole("button", { name: "Cancel" }).click();
  await page.getByRole("button", { name: /Add API access/i }).click();
  const providerModel = page.getByTestId("provider-model-id").first();
  await providerModel.fill("");
  await providerModel.pressSequentially("deepseek-v4-flash-x");
  await expect(providerModel).toHaveValue("deepseek-v4-flash-x");
  await expect(providerModel).toBeFocused();
});

test("inline approval card keeps its buttons reachable with a long preview (#63)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("NEEDCONFIRM");
  await page.getByRole("button", { name: "Send" }).click();
  // A very long preview must not push the allow button off-screen; the card
  // scrolls the code block internally so the actions stay in view.
  const allow = page.getByRole("button", { name: "Allow once" });
  await expect(allow).toBeVisible({ timeout: 10_000 });
  await expect(allow).toBeInViewport();
});

test("native approval remains clickable while the agent turn is blocked", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("BLOCKINGCONFIRM");
  await page.getByRole("button", { name: "Send" }).click();

  const allow = page.getByRole("button", { name: "Allow once" });
  await expect(allow).toBeVisible({ timeout: 10_000 });
  await expect.poll(() => page.evaluate(() =>
    Object.values((window as any).__nativeConfirmPending ?? {}).some(Boolean)
  )).toBe(true);

  await allow.click();

  await expect.poll(() => lastInvokeArgs(page, "confirm_response")).toMatchObject({
    approved: true,
    scope: "once",
  });
  await expect.poll(() => page.evaluate(() =>
    Object.values((window as any).__nativeConfirmPending ?? {}).some(Boolean)
  )).toBe(false);
});

test("background approval targets its session after switching conversations", async ({ page }) => {
  await enterApp(page, "/?mockSessionModels=1&mockBackgroundApproval=1");
  await page.locator('[data-session-id="s-model-a"]').click();
  await expect(page.getByText("Answer in s-model-a")).toBeVisible();

  await page.evaluate(() => (window as any).__tauriEmit("confirm-request", {
    frame_id: "s-model-b",
    message: "Run tool 'transfer_between_contexts'?",
    tool: "transfer_between_contexts",
    preview: "ssh:source:/data/results.tsv -> local:/project/data/results.tsv",
  }));
  await page.locator('[data-session-id="s-model-b"]').click();

  const reject = page.getByRole("button", { name: "Deny" });
  await expect(reject).toBeVisible();
  await reject.click();

  await expect.poll(() => lastInvokeArgs(page, "confirm_response")).toMatchObject({
    sessionId: "s-model-b",
    approved: false,
  });
  await expect(reject).toHaveCount(0);
});

test("resource conflict approval explains the owner and only offers wait or cancel", async ({ page }) => {
  await enterApp(page, "/?mockSessionModels=1");
  await page.locator('[data-session-id="s-model-a"]').click();
  await page.evaluate(() => (window as any).__tauriEmit("confirm-request", {
    frame_id: "s-model-a",
    message: "WISP_RESOURCE_CONFLICT\nPlot analysis · abc123 is using `plot.R`.",
    tool: "resource_conflict",
    preview: "Plot analysis · abc123 is using `plot.R`. Approve to wait for the R call to finish.",
  }));

  await expect(page.getByText("Another conversation is using this resource")).toBeVisible();
  await expect(page.getByText(/Plot analysis .* is using `plot\.R`/)).toBeVisible();
  await expect(page.getByLabel("Approval scope")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Cancel operation" })).toBeVisible();
  await page.getByRole("button", { name: "Wait & continue" }).click();

  await expect.poll(() => lastInvokeArgs(page, "confirm_response")).toMatchObject({
    sessionId: "s-model-a",
    approved: true,
    scope: "once",
  });
});

test("large image approval warns before resizing and cannot be remembered", async ({ page }) => {
  await enterApp(page, "/?mockSessionModels=1");
  await page.locator('[data-session-id="s-model-a"]').click();
  await page.evaluate(() => (window as any).__tauriEmit("confirm-request", {
    frame_id: "s-model-a",
    message: "plot.png exceeds 5 MiB. The original file will not be changed. Fine details may be lost.",
    tool: "image_resize",
    preview: "",
  }));

  await expect(page.getByText("Resize large image for model input?")).toBeVisible();
  await expect(page.getByText(/Fine details may be lost/)).toBeVisible();
  await expect(page.getByLabel("Approval scope")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();
  await page.getByRole("button", { name: "Resize & continue" }).click();

  await expect.poll(() => lastInvokeArgs(page, "confirm_response")).toMatchObject({
    sessionId: "s-model-a",
    approved: true,
    scope: "once",
  });
});

test("update_plan approval renders structured multiline Markdown", async ({ page }) => {
  await enterApp(page, "/?mockSessionModels=1");
  await page.locator('[data-session-id="s-model-a"]').click();
  await page.evaluate(() => (window as any).__tauriEmit("confirm-request", {
    frame_id: "s-model-a",
    message: "Review the proposed plan",
    tool: "update_plan",
    preview: JSON.stringify({
      v: 1,
      steps: [{
        status: "pending",
        content: "**实现 Fast**\n\n- Chat Completions\n  - `service_tier=priority`\n\n> 保持默认兼容",
      }],
    }),
  }));

  const step = page.locator(".plan-step-text");
  await expect(step.locator("strong")).toHaveText("实现 Fast");
  await expect(step.locator("ul li")).toHaveCount(2);
  await expect(step.locator("code")).toHaveText("service_tier=priority");
  await expect(step.locator("blockquote")).toContainText("保持默认兼容");
  await expect(step).not.toContainText("**实现 Fast**");
});

test("Escape closes plan feedback before rejecting the plan", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("NEEDPLAN");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Review plan before starting?")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Other" }).click();
  const feedback = page.getByPlaceholder("Tell wisp what to change in this plan.");
  await expect(feedback).toBeVisible();

  await page.keyboard.press("Escape");

  await expect(feedback).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Approve & start" })).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "confirm_response")).toBeNull();
});

test("inline approval scope is sent with confirmation", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("NEEDCONFIRM");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByRole("button", { name: "Allow once" })).toBeVisible({ timeout: 10_000 });
  await page.getByLabel("Approval scope").selectOption("project");
  await page.getByRole("button", { name: "Allow for this project" }).click();

  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).map((c: any) => ({
      cmd: c.cmd,
      args: c.args instanceof Map ? Object.fromEntries(c.args) : (c.args ?? {}),
    }));
    return calls.find((c: any) => c.cmd === "confirm_response") ?? null;
  })).toMatchObject({
    cmd: "confirm_response",
    args: {
      approved: true,
      scope: "project",
    },
  });
});

test("R execution uses the language-specific approval label", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("NEEDRCONFIRM");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Run R code?")).toBeVisible({ timeout: 10_000 });
  await expect(page.locator(".approval-code code.language-r")).toContainText("summary(dataset)");
});

test("awaiting approval marks the session dot and requests a desktop notification (#327)", async ({ page }) => {
  await page.goto("/?mockLongSession=1");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();
  await composer(page).fill("NEEDCONFIRM");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByRole("button", { name: "Allow once" })).toBeVisible({ timeout: 10_000 });
  // The waiting state shows on the sidebar session row as a circle-alert icon.
  const waiting = page.locator(".side-item.ses.attention");
  await expect(waiting).toHaveCount(1);
  await expect(waiting.locator(".ses-attention")).toBeVisible();
  await expect(waiting.locator(".ses-attention svg")).toBeVisible();
  await expect(waiting.locator(".ses-live")).toBeHidden();
  // The UI asked the backend for a desktop notification carrying the session
  // title (the backend decides visibility from window focus + settings).
  await expect.poll(async () => page.evaluate(() => {
    const call = ((window as any).__skillInvokeLog ?? []).find((c: any) => c.cmd === "notify_user");
    if (!call) return null;
    return call.args instanceof Map ? Object.fromEntries(call.args) : (call.args ?? {});
  })).toMatchObject({
    title: "Waiting for your approval",
    body: "Long transcript · shell",
  });
  // Responding clears the badge.
  await page.getByRole("button", { name: "Allow once" }).click();
  await expect(page.locator(".side-item.ses.attention")).toHaveCount(0);
});

test("settings permissions lists and revokes remembered approvals", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Permissions");

  await expect(page.getByText("Shell commands")).toBeVisible();
  await expect(page.getByText("Global")).toBeVisible();
  await page.getByRole("button", { name: "Revoke all" }).click();
  await expect(page.getByText("No remembered approvals.")).toBeVisible();
});

test("browser URL filters persist block and prefer hosts", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Browser");
  await expect(page.getByTestId("browser-url-filters")).toBeVisible();
  const autoLaunch = page.getByTestId("browser-auto-launch");
  await expect(autoLaunch).toBeChecked();
  await autoLaunch.locator("..").click();
  await expect(autoLaunch).not.toBeChecked();
  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toHaveCount(0);

  await openSettingsSection(page, "Browser");
  await expect(page.getByTestId("browser-auto-launch")).not.toBeChecked();
  await page.getByTestId("browser-block-host").fill("hijacked.example");
  await page.getByTestId("browser-block-reason").fill("domain taken over");
  await page.getByTestId("browser-block-add").click();
  await expect(page.getByTestId("browser-block-list")).toContainText("hijacked.example");
  await expect(page.getByTestId("browser-block-list")).toContainText("domain taken over");

  await page.getByTestId("browser-prefer-host").fill("pubmed.ncbi.nlm.nih.gov");
  await page.getByTestId("browser-prefer-add").click();
  await expect(page.getByTestId("browser-prefer-list")).toContainText("pubmed.ncbi.nlm.nih.gov");

  await page.getByTestId("browser-block-remove").click();
  await expect(page.getByTestId("browser-block-list")).toContainText("No blocked hosts.");
});

test("browser auto-close tabs setting persists", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Browser");
  const autoClose = page.getByTestId("browser-auto-close-tabs");
  await expect(autoClose).not.toBeChecked();
  await autoClose.locator("..").click();
  await expect(autoClose).toBeChecked();
  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toHaveCount(0);

  await openSettingsSection(page, "Browser");
  await expect(page.getByTestId("browser-auto-close-tabs")).toBeChecked();
});

test("browser tab cleanup confirms selected tabs and Escape keeps them", async ({ page }) => {
  await enterApp(page);
  const prompt = {
    turn_id: "turn-ui",
    frame_id: "s1",
    tabs: [
      { session: "shared", tab_id: 11, url: "https://keep.example/paper", title: "Keep me", initial_url: "https://keep.example" },
      { session: "shared", tab_id: 12, url: "https://close.example/paper", title: "Close me", initial_url: "https://close.example" },
    ],
  };
  await emitTauriEvent(page, "browser-tab-cleanup", prompt);
  const dialog = page.getByTestId("browser-tab-cleanup");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("This turn opened 2 tabs");
  await expect(page.getByTestId("browser-tab-cleanup-check-11")).toBeChecked();
  await expect(page.getByTestId("browser-tab-cleanup-check-12")).toBeChecked();
  await page.getByTestId("browser-tab-cleanup-check-11").uncheck();
  await page.getByTestId("browser-tab-cleanup-close").click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(async () => page.evaluate(() => {
    const call = ((window as any).__skillInvokeLog ?? []).find((c: any) => c.cmd === "confirm_browser_tab_cleanup");
    if (!call) return null;
    const args = call.args instanceof Map ? Object.fromEntries(call.args) : (call.args ?? {});
    const tabs = Array.isArray(args.tabs) ? args.tabs.map((tab: any) => tab instanceof Map ? Object.fromEntries(tab) : tab) : [];
    return { turnId: args.turnId, tabIds: tabs.map((tab: any) => tab.tab_id) };
  })).toEqual({ turnId: "turn-ui", tabIds: [12] });

  await emitTauriEvent(page, "browser-tab-cleanup", {
    ...prompt,
    turn_id: "turn-escape",
  });
  await expect(page.getByTestId("browser-tab-cleanup")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("browser-tab-cleanup")).toHaveCount(0);
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((c: any) => c.cmd === "dismiss_browser_tab_cleanup")
  )).toBe(true);
});

test("browser tab cleanup Escape closes only the overlay while settings stay open", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Browser");
  await expect(page.getByTestId("browser-url-filters")).toBeVisible();
  await emitTauriEvent(page, "browser-tab-cleanup", {
    turn_id: "turn-settings",
    frame_id: "s1",
    tabs: [
      { session: "shared", tab_id: 7, url: "https://example.com", title: "Example", initial_url: "https://example.com" },
    ],
  });
  await expect(page.getByTestId("browser-tab-cleanup")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("browser-tab-cleanup")).toHaveCount(0);
  await expect(page.getByTestId("browser-url-filters")).toBeVisible();
});

test("chat stays pinned to the bottom while streaming a long reply (#61)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("SCROLLTEST");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("line 79")).toBeVisible({ timeout: 15_000 });
  // The per-delta re-render used to clamp scrollTop toward the top and unfollow,
  // stranding the view at the top mid-stream. The scroller must end at the bottom.
  await expect
    .poll(
      async () =>
        page.evaluate(() => {
          const el = document.getElementById("chat-scroller");
          if (!el) return 9999;
          return el.scrollHeight - el.clientHeight - el.scrollTop;
        }),
      { timeout: 5000 },
    )
    .toBeLessThan(8);
});

test("chat stays at the latest message after tool results rebuild the thread (#927)", async ({ page }) => {
  await page.goto("/?mockLongPages=8");
  await page.locator(".proj-card-main").first().click();
  const scroller = page.locator("#chat-scroller");
  await expect(page.getByText(/Window page 0 row 19/)).toBeVisible();
  await expect.poll(() => scroller.evaluate((element) =>
    element.scrollHeight - element.clientHeight - element.scrollTop,
  )).toBeLessThan(8);

  await composer(page).fill("TOOLSCROLLTEST");
  await page.getByRole("button", { name: "Send" }).click();
  // A click on the live transcript used to mark a user-scroll gesture, so the
  // next tool-result collapse parked the view at the top.
  await scroller.evaluate((element) => {
    element.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
  });
  await expect(page.getByText("Tools finished at the tail.")).toBeVisible({ timeout: 15_000 });
  await expect.poll(() => scroller.evaluate((element) =>
    element.scrollHeight - element.clientHeight - element.scrollTop,
  )).toBeLessThan(8);
});

test("streaming assistant keeps formatted Markdown with a lightweight live tail", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("MARKDOWNSTREAM");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText("stream line 4", { exact: false })).toBeVisible({ timeout: 10_000 });
  const live = page.locator(".msg.assistant .streaming-markdown");
  await expect(live).toBeVisible();
  await expect(live.locator(".streaming-markdown-prefix strong").first()).toBeVisible();
  // Deltas and the Markdown commit interval are both ~50 ms. A Playwright poll
  // started after line 18 can miss every pending-tail window under CI load, so
  // watch the attribute continuously from the page instead. The live tail is
  // removed when the turn finishes, which a slow runner can reach before the
  // assertions below, so both the pending-byte peak and the node identity are
  // recorded on `window` rather than re-resolved through the locator.
  await live.evaluate((element) => {
    const seen = { max: 0 };
    const record = (pending: string | null) => {
      seen.max = Math.max(seen.max, Number(pending ?? 0));
      (window as any).__maxPendingBytes = seen.max;
      const current = document.querySelector(".msg.assistant .streaming-markdown");
      if (current && current !== element) (window as any).__liveMarkdownReplaced = true;
    };
    (window as any).__maxPendingBytes = 0;
    (window as any).__liveMarkdownReplaced = false;
    record(element.getAttribute("data-pending-bytes"));
    const observer = new MutationObserver((mutations) => {
      for (const mutation of mutations) record(mutation.oldValue);
      record(element.getAttribute("data-pending-bytes"));
    });
    observer.observe(element, {
      attributes: true,
      attributeFilter: ["data-pending-bytes"],
      attributeOldValue: true,
    });
    const tick = () => {
      record(element.getAttribute("data-pending-bytes"));
      if (element.isConnected) requestAnimationFrame(tick);
      else observer.disconnect();
    };
    requestAnimationFrame(tick);
  });

  await expect(page.getByText("stream line 18", { exact: false })).toBeVisible({ timeout: 10_000 });
  await expect.poll(() => page.evaluate(() => Number((window as any).__maxPendingBytes ?? 0)))
    .toBeGreaterThan(0);
  expect(await page.evaluate(() => (window as any).__liveMarkdownReplaced === false)).toBe(true);

  await expect(page.getByText("stream line 23", { exact: false })).toBeVisible({ timeout: 10_000 });
  await expect(page.locator(".msg.assistant .body.md")).toBeVisible();
  await expect(page.locator(".msg.assistant .body.md strong")).toHaveCount(24);
  await expect(page.locator(".msg.assistant .body.streaming")).toHaveCount(0);
});

test("step commentary renders full Markdown before the overall turn finishes", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPMARKDOWN");
  await page.getByRole("button", { name: "Send" }).click();

  const live = page.locator(".msg.assistant .streaming-markdown");
  await expect(live.getByRole("heading", { name: "Live analysis" })).toBeVisible();
  await expect(live.locator("strong")).toHaveText("Significant result");
  await expect(live.locator("li")).toHaveCount(2);
  await expect(live.locator("table")).toContainText("ESR1");
  await expect(live.locator("code")).toHaveText("normalized_counts.csv");

  // The following tool seals the commentary but the turn deliberately stays
  // pending. Its progress row must retain rendered Markdown rather than
  // falling back to source text until Done.
  const settledStep = page.locator(".msg.assistant.commentary .compact-markdown");
  await expect(settledStep.getByRole("heading", { name: "Live analysis" })).toBeVisible();
  await expect(settledStep.locator("strong")).toHaveText("Significant result");
  await expect(settledStep.locator("li")).toHaveCount(2);
  await expect(settledStep.locator("table")).toContainText("ESR1");
  await expect(settledStep.locator("code")).toHaveText("normalized_counts.csv");
});

test("chat keeps the user's reading position when streaming finishes (#670)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("SCROLLTEST");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("line 39", { exact: false })).toBeVisible({ timeout: 10_000 });

  const scroller = page.locator("#chat-scroller");
  await scroller.evaluate((element) => {
    element.scrollTop = Math.max(80, element.scrollHeight / 3);
    element.dispatchEvent(new WheelEvent("wheel", { deltaY: -80, bubbles: true }));
  });
  const readingTop = await scroller.evaluate((element) => element.scrollTop);

  await expect(page.getByText("line 79", { exact: false })).toBeVisible({ timeout: 10_000 });
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBeGreaterThan(readingTop - 40);
});

test("closing a center-file tab restores the conversation reading position", async ({ page }) => {
  await page.goto("/?mockLongPages=8");
  await page.locator(".proj-card-main").first().click();
  const scroller = page.locator("#chat-scroller");
  await expect(page.getByText(/Window page 0 row 19/)).toBeVisible();
  await scroller.evaluate((element) => {
    element.scrollTop = Math.max(120, element.scrollHeight / 3);
    element.dispatchEvent(new WheelEvent("wheel", { deltaY: -80, bubbles: true }));
  });
  const readingTop = await scroller.evaluate((element) => element.scrollTop);

  await page.locator(".sidebar").getByRole("button", { name: "Search sessions" }).click();
  const search = commandPalette(page);
  await search.fill("nif3.treefile");
  await expect(page.locator(".project-search-row", { hasText: "nif3.treefile" })).toBeVisible();
  await search.press("Enter");
  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await modal.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-file-preview")).toBeVisible();

  await page.locator(".center-tab-wrap", { hasText: "nif3.treefile" })
    .getByRole("button", { name: "Close tab" }).click();
  await expect(scroller).toBeVisible();
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(readingTop - 40);
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBeLessThan(readingTop + 40);
});

test("agent options stay mounted while chat content streams (#678)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("SCROLLTEST");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("line 10", { exact: false })).toBeVisible({ timeout: 10_000 });

  await page.getByRole("button", { name: "Agent options" }).click();
  const menu = page.getByRole("menu", { name: "Agent options" });
  await expect(menu).toBeVisible();
  await menu.evaluate((element) => ((window as any).__streamingAgentMenu = element));

  await expect(page.getByText("line 79", { exact: false })).toBeVisible({ timeout: 10_000 });
  expect(await menu.evaluate((element) => element === (window as any).__streamingAgentMenu)).toBe(true);
});

test("recent sessions show only title and status badge", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 700 });
  await page.goto("/");
  const cards = page.locator('[data-testid="recent-session-card"]');
  await expect(cards).toHaveCount(2);

  const first = cards.first();
  await expect(first.locator(".pc-name")).toHaveText("帮我找一篇单细胞的文章");
  await expect(first.locator(".sess-status-needs-you")).toBeVisible();
  await expect(first.locator(".pc-hint")).toHaveCount(0);
  await expect(first.locator(".pc-when")).toHaveCount(0);
  await expect(first.locator(".pc-meta-row")).toHaveCount(0);

  const second = cards.nth(1);
  await expect(second.locator(".pc-name")).toHaveText("Enumerate MCP bio-tools databases");
  await expect(second.locator(".sess-status-complete")).toBeVisible();

  for (const card of [first, second]) {
    const badge = card.locator(".sess-status");
    await expect.poll(() => badge.evaluate((node) => {
      const style = getComputedStyle(node);
      return { flexShrink: style.flexShrink, whiteSpace: style.whiteSpace };
    })).toEqual({ flexShrink: "0", whiteSpace: "nowrap" });
    await expect.poll(() => card.locator(".pc-name").evaluate((node) => {
      const style = getComputedStyle(node);
      return { overflow: style.overflow, textOverflow: style.textOverflow, whiteSpace: style.whiteSpace };
    })).toEqual({ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" });
  }
});

test("right panel keeps actions and the active tab visible when tabs overflow", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 700 });
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();

  const panel = page.locator(".rightpane");
  const addPanel = panel.getByRole("button", { name: "Add panel" });
  for (const name of [/^Notebook/, /^Highlights/, /^Provenance/, /^Side chat$/]) {
    await addPanel.click();
    await panel.locator(".rp-tab-add-menu").getByRole("button", { name }).click();
  }

  await expect(panel.locator(".rp-tab-scroll")).toBeVisible();
  await expect(addPanel).toBeVisible();
  await expect(panel.locator(".rp-tab.active")).toHaveText("Side chat");
  await expect.poll(() => panel.evaluate((node) => {
    const scroller = node.querySelector<HTMLElement>(".rp-tab-scroll")!;
    const active = node.querySelector<HTMLElement>(".rp-tab.active")!;
    const add = node.querySelector<HTMLElement>(".rp-tab-add")!;
    const panelBox = node.getBoundingClientRect();
    const scrollBox = scroller.getBoundingClientRect();
    const activeBox = active.getBoundingClientRect();
    const addBox = add.getBoundingClientRect();
    return {
      overflowed: scroller.scrollWidth > scroller.clientWidth,
      activeVisible: activeBox.left >= scrollBox.left - 1 && activeBox.right <= scrollBox.right + 1,
      actionsInsidePanel: addBox.left >= panelBox.left && addBox.right <= panelBox.right,
      actionsOutsideScroller: addBox.left >= scrollBox.right - 1,
    };
  })).toEqual({
    overflowed: true,
    activeVisible: true,
    actionsInsidePanel: true,
    actionsOutsideScroller: true,
  });
});

test("session history loads older pages with a stable cursor", async ({ page }) => {
  await page.goto("/?mockManySessions=1");
  await page.locator(".proj-card-main").first().click();

  await expect(page.locator(".sidebar").getByRole("button", { name: "Paged session 1", exact: true })).toBeVisible();
  expect(await page.locator(".sidebar").getByRole("button", { name: "Paged session 101", exact: true }).count()).toBe(0);
  await page.getByRole("button", { name: "Load earlier sessions" }).click();
  await expect(page.locator(".sidebar").getByRole("button", { name: "Paged session 101", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Load earlier sessions" })).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "list_sessions_page")).toMatchObject({
    cursor: { id: "session-100", ts: 1901 },
  });
});

test("sidebar search opens the Ctrl+K palette and finds sessions beyond loaded history", async ({ page }) => {
  await pinNonMacPlatform(page);
  await page.goto("/?mockManySessions=1");
  await page.locator(".proj-card-main").first().click();

  const sidebar = page.locator(".sidebar");
  const navButtons = sidebar.locator(".nav .side-btn");
  await expect(navButtons.nth(0).locator(".side-btn-label")).toHaveText("New session");
  await expect(navButtons.nth(0).locator(".side-shortcut")).toHaveText("Ctrl+N");
  await expect(navButtons.nth(1).locator(".side-btn-label")).toHaveText("Search");
  await expect(navButtons.nth(1).locator(".side-shortcut")).toHaveText("Ctrl+K");

  await sidebar.getByRole("button", { name: "Search sessions" }).click();
  const search = commandPalette(page);
  await expect(search).toBeFocused();

  await search.fill("101");
  await expect(page.locator(".project-search-row", { hasText: "Paged session 101" })).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "search_sessions")).toMatchObject({
    query: "101",
    limit: 12,
    preferredProjectId: "default",
  });

  await page.keyboard.press("Escape");
  await expect(search).toHaveCount(0);
  await expect(sidebar.getByRole("button", { name: "Paged session 1", exact: true })).toBeVisible();
  await expect(sidebar.getByRole("button", { name: "Paged session 101", exact: true })).toHaveCount(0);
});

test("home search opens artifacts, sessions, and settings", async ({ page }) => {
  await page.goto("/");

  await globalSettingsButton(page).click();
  const settingsPage = page.locator(".settings-page");
  await expect(settingsPage).toBeVisible();
  await expect(page.locator(".overlay", { has: settingsPage })).toHaveCount(0);
  const expectedSettingsTop = await page.locator(".window-titlebar").count() === 1 ? 38 : 0;
  await expect.poll(() => settingsPage.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      top: Math.round(rect.top),
      left: Math.round(rect.left),
      right: Math.round(rect.right),
      bottom: Math.round(rect.bottom),
    };
  })).toEqual({ top: expectedSettingsTop, left: 0, right: 1280, bottom: 720 });
  await expect(page.getByRole("button", { name: "Back to app" })).toBeVisible();
  await page.locator(".settings-head-close").click();

  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await expect(search).toBeVisible();
  await expect(page.locator(".project-search-row", { hasText: "nif3.treefile" })).toBeVisible();
  await search.fill("update");
  await expect(page.locator(".project-search-row", { hasText: "Check for updates" })).toBeVisible();
  await search.fill("star");
  await expect(page.locator(".project-search-row", { hasText: "Star us on GitHub" })).toBeVisible();
  await search.fill("file");
  await expect(page.locator(".project-search-row", { hasText: "nif3.treefile" })).toBeVisible();
  await search.press("Enter");
  await expect(page.locator(".artifact-modal")).toBeVisible();
  await expect(page.locator(".am-name")).toHaveText("nif3.treefile");
  await page.locator(".artifact-modal").getByRole("button", { name: "Close panel" }).click();

  await page.getByRole("button", { name: "Search" }).click();
  await search.fill("Enumerate");
  await expect(page.locator(".project-search-row", { hasText: "Enumerate MCP bio-tools databases" })).toBeVisible();
  await search.press("Enter");
  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).map((c: any) => ({
      cmd: c.cmd,
      args: c.args instanceof Map ? Object.fromEntries(c.args) : (c.args ?? {}),
    }));
    return calls.find((c: any) => c.cmd === "load_session") ?? null;
  })).toMatchObject({ cmd: "load_session", args: { id: "s-complete" } });
});

test("long transcripts load earlier turns without jumping to the new top", async ({ page }) => {
  await page.goto("/?mockLongSession=1");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  await expect(page.getByText("Newest page first question", { exact: true })).toBeVisible();
  const scroller = page.locator("#chat-scroller");
  const loadEarlier = page.getByRole("button", { name: "Load earlier messages" });
  await expect(loadEarlier).toBeVisible();
  await loadEarlier.click();

  await expect(page.getByText("Oldest loaded question", { exact: true })).toBeAttached();
  await expect(loadEarlier).toHaveCount(0);
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await expect.poll(() => page.evaluate(() => (window as any).__transcriptPageCalls)).toEqual([
    null,
    41,
  ]);
});

test("opening a long conversation lands at the latest message and stays stable on scroll (#663)", async ({ page }) => {
  await page.goto("/?mockLongPages=8");
  await page.locator(".proj-card-main").first().click();

  const scroller = page.locator("#chat-scroller");
  await expect(page.getByText(/Window page 0 row 19/)).toBeVisible();
  await expect.poll(() => scroller.evaluate((element) =>
    element.scrollHeight - element.clientHeight - element.scrollTop,
  )).toBeLessThan(8);

  await scroller.hover();
  const before = await scroller.evaluate((element) => element.scrollTop);
  await page.mouse.wheel(0, -80);
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBeLessThan(before);
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBeGreaterThan(before - 240);

  const readingPosition = await scroller.evaluate((element) => element.scrollTop);
  await expect(scroller).toHaveCSS("overflow-anchor", "auto");
  // Unfollow flips overflow-anchor back to auto, but Chromium only picks a
  // scroll anchor at layout time. Waiting frames alone is not enough: if no
  // layout ran in between, the prepend below is never compensated (#663).
  // Dirty layout with a zero-height node and force a reflow so the anchor is
  // deterministically armed before the real prepend.
  await page.evaluate(
    () => new Promise((resolve) => {
      const thread = document.getElementById("chat-thread");
      const probe = document.createElement("div");
      probe.style.height = "0px";
      probe.style.flex = "0 0 0px";
      thread?.prepend(probe);
      void (thread as HTMLElement | null)?.offsetHeight;
      requestAnimationFrame(() => requestAnimationFrame(resolve));
    }),
  );
  await page.evaluate(() => {
    const thread = document.getElementById("chat-thread");
    const spacer = document.createElement("div");
    spacer.style.height = "480px";
    spacer.style.flex = "0 0 480px";
    thread?.prepend(spacer);
  });
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(readingPosition + 400);
  // ResizeObserver runs after layout. Verify the scroll helper keeps the
  // browser's prepend compensation instead of briefly accepting it and then
  // restoring the stale pre-prepend bookmark.
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
  );
  expect(await scroller.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(readingPosition + 400);
});

test("switching conversations restores each reading position (#849)", async ({ page }) => {
  await page.goto("/?mockLongPages=8");
  await page.locator(".proj-card-main").first().click();

  const scroller = page.locator("#chat-scroller");
  await expect(page.getByText(/Window page 0 row 19/)).toBeVisible();
  await scroller.evaluate((element) => {
    element.scrollTop = Math.max(120, element.scrollHeight / 3);
    element.dispatchEvent(new WheelEvent("wheel", { deltaY: -80, bubbles: true }));
  });
  const readingTop = await scroller.evaluate((element) => element.scrollTop);

  await newSessionButton(page).click();
  await expect(page.locator(".empty")).toBeVisible();
  await page.locator(".side-item.ses", { hasText: "Long transcript" }).click();
  await expect(page.getByText(/Window page 0 row 19/)).toBeVisible();
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(readingTop - 40);
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBeLessThan(readingTop + 40);
});

test("a thread rebuild clamp does not park a followed view at the top (#927)", async ({ page }) => {
  await page.goto("/?mockLongPages=8");
  await page.locator(".proj-card-main").first().click();
  const scroller = page.locator("#chat-scroller");
  await expect(page.getByText(/Window page 0 row 19/)).toBeVisible();
  await expect.poll(() => scroller.evaluate((element) =>
    element.scrollHeight - element.clientHeight - element.scrollTop,
  )).toBeLessThan(8);

  await scroller.evaluate((element) => {
    element.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    const thread = document.getElementById("chat-thread");
    if (!thread) return;
    const children = Array.from(thread.children) as HTMLElement[];
    for (const child of children) child.style.display = "none";
    thread.getBoundingClientRect();
    for (const child of children) child.style.display = "";
    element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
  });
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
  );
  await expect.poll(() => scroller.evaluate((element) =>
    element.scrollHeight - element.clientHeight - element.scrollTop,
  )).toBeLessThan(8);
});

test("jump pill returns a scrolled-up view to the latest message", async ({ page }) => {
  await page.goto("/?mockLongPages=8");
  await page.locator(".proj-card-main").first().click();
  const scroller = page.locator("#chat-scroller");
  await expect(page.getByText(/Window page 0 row 19/)).toBeVisible();
  await expect.poll(() => scroller.evaluate((element) =>
    element.scrollHeight - element.clientHeight - element.scrollTop,
  )).toBeLessThan(8);

  await scroller.evaluate((element) => {
    element.dispatchEvent(new WheelEvent("wheel", { deltaY: -80, bubbles: true }));
    element.scrollTop = 0;
  });
  const pill = page.locator("#chat-jump-pill");
  await expect(pill).toBeVisible();
  await expect(pill).toContainText("Back to latest");
  await pill.click();
  await expect.poll(() => scroller.evaluate((element) =>
    element.scrollHeight - element.clientHeight - element.scrollTop,
  )).toBeLessThan(8);
  await expect(pill).not.toHaveClass(/visible/);
});

test("a thread rebuild keeps a scrolled-up reading position (#927)", async ({ page }) => {
  await page.goto("/?mockLongPages=8");
  await page.locator(".proj-card-main").first().click();
  const scroller = page.locator("#chat-scroller");
  await expect(page.getByText(/Window page 0 row 19/)).toBeVisible();
  await scroller.evaluate((element) => {
    element.dispatchEvent(new WheelEvent("wheel", { deltaY: -80, bubbles: true }));
    element.scrollTop = Math.max(120, element.scrollHeight / 3);
  });
  const readingTop = await scroller.evaluate((element) => element.scrollTop);
  expect(readingTop).toBeGreaterThan(40);
  await scroller.evaluate(() => new Promise((resolve) => setTimeout(resolve, 600)));

  await scroller.evaluate((element) => {
    const thread = document.getElementById("chat-thread");
    if (!thread) return;
    const children = Array.from(thread.children) as HTMLElement[];
    for (const child of children) child.style.display = "none";
    thread.getBoundingClientRect();
    for (const child of children) child.style.display = "";
  });
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
  );
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(readingTop - 40);
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBeLessThan(readingTop + 80);
});

test("conversation outline loads and jumps to an older user question", async ({ page }) => {
  await page.goto("/?mockLongSession=1");
  await page.locator(".proj-card-main").first().click();

  const toggle = page.getByRole("button", { name: "Show conversation outline" });
  await expect(toggle).toBeVisible();
  await expect(page.getByTestId("conversation-outline")).toHaveCount(0);
  await toggle.click();
  const outline = page.getByTestId("conversation-outline");
  await expect(outline).toBeVisible();
  await expect(outline).toHaveClass(/is-open/);
  await page.keyboard.press("Escape");
  await expect(outline).toBeHidden();
  await expect(page.getByTestId("conversation-outline")).toHaveCount(0);
  await expect(toggle).toBeVisible();

  await toggle.click();
  await expect(outline).toBeVisible();
  const oldestOutline = outline.getByRole("button", { name: "Oldest loaded question" });
  await expect(oldestOutline.locator(".conversation-outline-time")).toHaveAttribute(
    "data-timestamp",
    "1783478400",
  );
  await expect(oldestOutline.locator(".conversation-outline-time")).not.toBeEmpty();
  await oldestOutline.click();

  await expect.poll(() => lastInvokeArgs(page, "load_session")).toMatchObject({
    id: "long-session",
    beforeSeq: 5,
  });
  const target = page.locator('[data-user-index="0"]');
  await expect(target).toContainText("Oldest loaded question");
  await expect(target.locator(".user-message-time")).toHaveAttribute(
    "data-timestamp",
    "1783478400",
  );
  await expect(page.locator('[data-ui-index="1"] .assistant-message-time')).toHaveAttribute(
    "data-timestamp",
    "1783478430",
  );
  await expect.poll(() => target.evaluate((element) => {
    const scroller = document.querySelector("#chat-scroller");
    if (!scroller) return false;
    const row = element.getBoundingClientRect();
    const viewport = scroller.getBoundingClientRect();
    return row.top >= viewport.top && row.bottom <= viewport.bottom;
  })).toBe(true);

  await page.getByRole("button", { name: "Hide conversation outline" }).click();
  await expect(outline).toBeHidden();
  await expect(toggle).toBeVisible();
  await expect(page.getByTestId("conversation-outline")).toHaveCount(0);
});

test("long transcript rendering keeps a bounded turn window", async ({ page }) => {
  const pageCount = Number(process.env.TRANSCRIPT_SOAK_PAGES ?? 8);
  test.setTimeout(Math.max(30_000, pageCount * 2_000));
  await page.goto(`/?mockLongPages=${pageCount}`);
  await page.locator(".proj-card-main").first().click();

  for (let loaded = 1; loaded < pageCount; loaded += 1) {
    await page.getByRole("button", { name: "Load earlier messages" }).click();
    await expect.poll(() => page.evaluate(() =>
      ((window as any).__transcriptPageCalls ?? []).length,
    )).toBe(loaded + 1);
  }

  await expect(page.locator(".msg.user")).toHaveCount(20);
  const oldestRow = new RegExp(`Window page ${pageCount - 1} row 0`);
  await expect(page.getByText(oldestRow)).toBeVisible();
  const newerSteps = Math.ceil(Math.max(0, pageCount * 10 - 20) / 20);
  for (let step = 0; step < newerSteps; step += 1) {
    await page.getByRole("button", { name: "Show newer messages" }).click();
  }
  await expect(page.locator(".msg.user")).toHaveCount(20);
  await expect(page.getByText(/Window page 0 row 0/)).toBeVisible();
  await expect(page.getByText(oldestRow)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Show earlier loaded messages" })).toBeVisible();
});

test("a continuously open conversation unloads old live rows after a completed turn", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("seed turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.locator(".msg.assistant")).toContainText("Hello from mock wisp-science.");
  const sent = await lastInvokeArgs(page, "send_message");
  const frameId = String(sent?.sessionId ?? "");
  expect(frameId).not.toBe("");

  await page.evaluate(({ frameId }) => {
    for (let turn = 0; turn < 41; turn += 1) {
      (window as any).__tauriEmit("agent", {
        kind: "User",
        frame_id: frameId,
        text: `Live turn ${turn}`,
      });
      (window as any).__tauriEmit("agent", {
        kind: "MessageBoundary",
        frame_id: frameId,
        seq: 100 + turn * 2,
      });
      (window as any).__tauriEmit("agent", {
        kind: "Text",
        frame_id: frameId,
        delta: `Live answer ${turn}`,
      });
    }
  }, { frameId });

  const thread = page.locator("#chat-thread");
  const scroller = page.locator("#chat-scroller");
  await expect(thread.getByText("Live turn 40", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Show earlier loaded messages" })).toBeVisible();
  await expect.poll(() => scroller.evaluate((element) =>
    element.scrollHeight - element.clientHeight - element.scrollTop,
  )).toBeLessThan(8);

  await emitTauriEvent(page, "agent", { kind: "Done", frame_id: frameId });

  await expect(page.locator(".msg.user")).toHaveCount(20);
  await expect(thread.getByText("Live turn 21", { exact: true })).toBeVisible();
  await expect(thread.getByText("Live turn 40", { exact: true })).toBeVisible();
  await expect(thread.getByText("Live turn 0", { exact: true })).toHaveCount(0);
  const loadEarlier = page.getByRole("button", { name: "Load earlier messages" });
  await expect(loadEarlier).toBeVisible();
  await expect.poll(() => scroller.evaluate((element) =>
    element.scrollHeight - element.clientHeight - element.scrollTop,
  )).toBeLessThan(8);

  await loadEarlier.click();
  await expect.poll(() => lastInvokeArgs(page, "load_session")).toMatchObject({
    id: frameId,
    beforeSeq: 142,
  });
});

test("a multi-megabyte transcript stays interactive while an answer streams", async ({ page }) => {
  test.setTimeout(45_000);
  await page.goto(`/?mockLongPages=1&mockLongRows=160&mockLongRowBytes=${32 * 1024}`);
  await page.locator(".proj-card-main").first().click();
  await expect(page.getByText(/Window page 0 row 159/)).toBeVisible({ timeout: 15_000 });
  const historicAssistant = page.locator(".msg.assistant", {
    hasText: /Window page 0 row 159/,
  });
  await historicAssistant.evaluate((element) => ((element as any).__historicRowProbe = true));

  await composer(page).fill("MARKDOWNSTREAM");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("stream line 4", { exact: false })).toBeVisible({ timeout: 10_000 });
  expect(await historicAssistant.evaluate((element) => (element as any).__historicRowProbe === true)).toBe(true);
  await expect(historicAssistant.locator(".body.md")).toBeVisible();
  await page.getByRole("button", { name: "Agent options" }).click({ timeout: 2_000 });
  const menu = page.getByRole("menu", { name: "Agent options" });
  await expect(menu).toBeVisible();
  await menu.evaluate((element) => ((element as any).__largeTranscriptProbe = true));

  await expect(page.getByText("stream line 23", { exact: false })).toBeVisible({ timeout: 10_000 });
  expect(await historicAssistant.evaluate((element) => (element as any).__historicRowProbe === true)).toBe(true);
  expect(await menu.evaluate((element) => (element as any).__largeTranscriptProbe === true)).toBe(true);
});

test("branching from a paged transcript uses the global user-turn index", async ({ page }) => {
  await page.goto("/?mockLongSession=1");
  await page.locator(".proj-card-main").first().click();
  const firstLoadedUser = page.locator(".msg.user", { hasText: "Newest page first question" });
  await firstLoadedUser.getByRole("button", { name: "Branch" }).click();

  await expect.poll(() => lastInvokeArgs(page, "branch_session")).toMatchObject({
    sessionId: "long-session",
    userIndex: 10,
  });
});

test("HTML artifact modal uses a desktop preview viewport", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("dashboard");
  await search.press("Enter");

  const modal = page.locator(".artifact-modal.html-preview");
  await expect(modal).toBeVisible();
  const frame = modal.locator("iframe.rp-html");
  await expect(frame).toBeVisible();
  await expect.poll(() => frame.evaluate((el) => el.clientWidth)).toBeGreaterThanOrEqual(1190);
  await expect.poll(() => frame.evaluate((el: HTMLIFrameElement) => {
    const mode = el.contentDocument?.querySelector("#mode");
    return mode ? getComputedStyle(mode, "::after").content : "";
  })).toBe('"Desktop"');
});

test("MCP App opens as a persistent center tab and delivers tool data", async ({ page }) => {
  await enterApp(page);
  const html = `<!doctype html><html><body><div id="state">waiting</div><div id="sequence">TTTACGTACGTGGG</div><button id="map-mbp" class="motif-pm-feature" data-feature-id="mbp" aria-pressed="false">MBP map feature</button><div id="sequence-pane" class="motif-cs-sequence-column" style="height:80px;overflow:auto"><div style="height:500px"></div><button id="sequence-mbp" class="motif-cs-feature-block" aria-pressed="false">MBP sequence feature</button><div style="height:500px"></div></div><div class="motif-cs-selection-bar"><span class="motif-cs-selection-name">4-11 (8)</span></div><script>
    const state = document.getElementById("state");
    let initialized = false;
    let contextCapability = false;
    let contextSent = false;
    let contextAcknowledged = false;
    let input = false;
    let result = false;
    // Production Motif bundles contain HTML closing-tag text inside minified
    // JavaScript. The host bridge must never splice into this literal.
    window.__motifEmbeddedClosingBody = "</body>";
    window.motifGetActiveRecord = () => ({ id: "pet-28a", name: "pET-28a", molecule: "dna", seq: "TTTACGTACGTGGG", annotations: [{ id: "mbp", name: "MBP", start: 4, end: 12, strand: -1 }] });
    window.motifAddRecords = (records) => { window.__addedMotifRecords = records; };
    document.getElementById("map-mbp").addEventListener("click", () => {
      document.getElementById("map-mbp").setAttribute("aria-pressed", "true");
      document.getElementById("sequence-mbp").setAttribute("aria-pressed", "true");
      document.querySelector(".motif-cs-selection-name").textContent = "MBP 5-12";
    });
    const render = () => {
      state.textContent = [initialized, contextCapability, input, result, contextAcknowledged].join(":");
    };
    const updateContext = () => {
      if (!initialized || !input || !result || contextSent) return;
      contextSent = true;
      parent.postMessage({
        jsonrpc: "2.0",
        id: 2,
        method: "ui/update-model-context",
        params: {
          content: [{ type: "text", text: "Active record: pET-28a(+)" }],
          structuredContent: { recordId: "pet-28a", length: 5369 },
        },
      }, "*");
    };
    addEventListener("message", (event) => {
      const message = event.data || {};
      if (message.id === 1 && message.result?.hostInfo?.name === "wisp-science") {
        initialized = true;
        contextCapability = !!message.result?.hostCapabilities?.updateModelContext?.text;
        parent.postMessage({ jsonrpc: "2.0", method: "ui/notifications/initialized", params: {} }, "*");
      } else if (message.id === 2 && message.result) {
        contextAcknowledged = true;
      } else if (message.method === "ui/notifications/tool-input") {
        input = message.params?.arguments?.sequence === "ACGT";
      } else if (message.method === "ui/notifications/tool-result") {
        result = message.params?.structuredContent?.accepted === true;
      }
      updateContext();
      render();
    });
    parent.postMessage({
      jsonrpc: "2.0",
      id: 1,
      method: "ui/initialize",
      params: { protocolVersion: "2026-01-26" },
    }, "*");
  <\/script></body></html>`;
  await composer(page).fill("open the test app");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).not.toBeNull();
  const frameId = String((await lastInvokeArgs(page, "send_message")).sessionId);
  await page.evaluate(({ frameId, html }) => {
    (window as any).__tauriEmit("agent", {
      kind: "ToolPresentation",
      frame_id: frameId,
      presentation_kind: "mcp_app",
      payload: {
        tool: { name: "motif_open_workbench", title: "Motif test app" },
        arguments: { sequence: "ACGT" },
        result: { content: [], structuredContent: { accepted: true } },
        resource: { uri: "ui://motif/workbench.html", text: html, _meta: {} },
      },
    });
  }, { frameId, html });

  const app = page.frameLocator('iframe[title="Motif test app"]');
  await expect(app.locator("#state")).toHaveText("true:true:true:true:true");
  await expect(app.locator("body")).not.toContainText("const reply =");
  await expect.poll(() => app.locator("body").evaluate(() => (window as any).__motifEmbeddedClosingBody)).toBe("</body>");
  await expect(app.locator("[data-wisp-motif-selection-length]")).toHaveText("8 bp");
  await app.locator(".motif-cs-selection-name").evaluate((element) => {
    element.textContent = "12-3 wrap (6)";
  });
  await expect(app.locator("[data-wisp-motif-selection-length]")).toHaveText("6 bp");
  await app.locator(".motif-cs-selection-name").evaluate((element) => {
    element.textContent = "4-11 (8)";
  });
  await expect(app.locator("[data-wisp-motif-selection-length]")).toHaveText("8 bp");
  await expect.poll(() => lastInvokeArgs(page, "update_mcp_app_context")).toMatchObject({
    instanceId: expect.stringContaining(`mcp-app:${frameId}:`),
    appName: "Motif test app",
    context: {
      content: [{ type: "text", text: "Active record: pET-28a(+)" }],
      structuredContent: { recordId: "pet-28a", length: 5369 },
    },
  });
  const frame = page.locator('iframe[title="Motif test app"]');
  await expect(frame).toHaveAttribute("sandbox", "allow-scripts");
  const appTab = page.locator('.center-tab[data-center-path^="mcp-app:"]');
  await expect(appTab).toContainText("Motif test app");
  await expect(page.locator("main.center")).toHaveClass(/split/);
  await expect(page.locator(".center-mcp-app-preview")).toBeVisible();

  // Motif gets a host-owned picker that can read an explicitly selected file
  // from outside the project and reload the live workbench through its MCP
  // tool. The local path itself is never sent to the plugin.
  await page.evaluate(() => {
    (window as any).__mcpAppLiveBridges = true;
    (window as any).__mcpAppToolResults = {
      motif_open_workbench: {
        content: [],
        structuredContent: {
          schema: "motif.mcp.workbench.v1",
          mode: "artifact",
          recordCount: 1,
          residueCount: 8,
          payload: { records: [{ name: "local", type: "dna", sequence: "ACGTACGT" }] },
        },
        isError: false,
      },
    };
  });
  const chooserPromise = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Load DNA file" }).click();
  const chooser = await chooserPromise;
  await chooser.setFiles({
    name: "existing-local.fasta",
    mimeType: "text/x-fasta",
    buffer: Buffer.from(">local\nACGTACGT\n"),
  });
  await expect.poll(() => lastInvokeArgs(page, "call_mcp_app_tool")).toMatchObject({
    name: "motif_open_workbench",
    arguments: { filename: "existing-local.fasta", content: ">local\nACGTACGT\n" },
  });
  await expect(page.locator(".center-mcp-import-status")).toContainText("existing-local.fasta");

  const cookie = Buffer.alloc(5 + 14);
  cookie[0] = 0x09;
  cookie.writeUInt32BE(14, 1);
  cookie.write("SnapGene", 5, "ascii");
  const snapSequence = Buffer.from("ACGTRYSWKMBDHVN", "ascii");
  const dnaPacket = Buffer.alloc(5 + 1 + snapSequence.length);
  dnaPacket[0] = 0x00;
  dnaPacket.writeUInt32BE(1 + snapSequence.length, 1);
  dnaPacket[5] = 0x01; // SnapGene flags bit 0 denotes circular topology
  snapSequence.copy(dnaPacket, 6);
  const snapChooserPromise = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Load DNA file" }).click();
  const snapChooser = await snapChooserPromise;
  await snapChooser.setFiles({
    name: "pET-28a.dna",
    mimeType: "application/octet-stream",
    buffer: Buffer.concat([cookie, dnaPacket]),
  });
  await expect.poll(() => lastInvokeArgs(page, "call_mcp_app_tool")).toMatchObject({
    name: "motif_open_workbench",
    arguments: {
      payload: {
        records: [{
          name: "pET-28a",
          type: "dna",
          topology: "circular",
          sequence: "ACGTRYSWKMBDHVN",
          annotations: [],
        }],
      },
    },
  });
  await expect(page.locator(".center-mcp-import-status")).toContainText("pET-28a.dna");

  await app.locator("#sequence").evaluate((element) => {
    const node = element.firstChild!;
    const range = document.createRange();
    range.setStart(node, 3);
    range.setEnd(node, 11);
    const selection = getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
  });
  // Clicking a control outside an opaque iframe can clear WebView's native
  // DOM selection. Motif's own rendered range remains authoritative.
  await app.locator("#sequence").evaluate(() => getSelection()?.removeAllRanges());
  await page.getByRole("button", { name: "Add selection to chat" }).click();
  await expect(page.getByTestId("motif-selection-reference")).toContainText("pET-28a");
  await expect(page.getByTestId("motif-selection-reference")).toContainText("8 bp");
  await expect(composer(page)).toHaveValue(/Coordinates: 4-11 \(forward\)/);
  await expect(composer(page)).toHaveValue(/Length: 8 bp/);
  await expect(composer(page)).toHaveValue(/Sequence: ACGTACGT/);

  await app.locator("#map-mbp").click();
  await expect.poll(() => app.locator("#sequence-pane").evaluate((element) => element.scrollTop))
    .toBeGreaterThan(0);
  await page.getByRole("button", { name: "Add selection to chat" }).click();
  await expect(page.getByTestId("motif-selection-reference")).toContainText("MBP");
  await expect(page.getByTestId("motif-selection-reference")).toContainText("8 bp");
  await expect(app.locator("[data-wisp-motif-selection-length]")).toHaveText("8 bp");
  await expect(composer(page)).toHaveValue(/Feature: MBP/);
  await expect(composer(page)).toHaveValue(/Coordinates: 5-12 \(reverse\)/);
  await expect(composer(page)).toHaveValue(/Length: 8 bp/);
  await expect(composer(page)).toHaveValue(/Sequence: CGTACGTG/);

  // Switching back to the conversation parks the iframe, and returning to
  // the app preserves its live state instead of reloading it.
  await page.locator(".center-tab").first().click();
  await expect(page.locator(".center-mcp-app-preview")).toHaveCount(0);
  await appTab.click();
  await expect(app.locator("#state")).toHaveText("true:true:true:true:true");

  const tabWrap = page.locator(".center-tab-wrap", { has: appTab });
  await tabWrap.getByRole("button", { name: "Close tab" }).click();
  await expect(page.locator(".center-mcp-app-preview")).toHaveCount(0);
  await expect(frame).toHaveCount(0, { timeout: 2_000 });
  await expect.poll(async () => {
    const calls = await invokeArgsList(page, "update_mcp_app_context");
    return calls.at(-1)?.context ?? null;
  }).toEqual({});
});

test("live MCP App tools/call reaches the host without a new agent turn", async ({ page }) => {
  await enterApp(page);
  const html = `<!doctype html><html><body><div id="state">waiting</div><script>
    const state = document.getElementById("state");
    addEventListener("message", (event) => {
      const message = event.data || {};
      if (message.id === 1 && message.result?.hostCapabilities?.serverTools) {
        parent.postMessage({ jsonrpc: "2.0", method: "ui/notifications/initialized", params: {} }, "*");
        parent.postMessage({
          jsonrpc: "2.0",
          id: 2,
          method: "tools/call",
          params: { name: "figure_preview_exact", arguments: { id: "fig-1" } },
        }, "*");
      } else if (message.id === 2 && message.result?.structuredContent?.preview === true) {
        state.textContent = "preview:" + message.result.structuredContent.tool;
      } else if (message.id === 2 && message.error) {
        state.textContent = "error:" + message.error.message;
      }
    });
    parent.postMessage({
      jsonrpc: "2.0",
      id: 1,
      method: "ui/initialize",
      params: { protocolVersion: "2026-01-26" },
    }, "*");
  <\/script></body></html>`;
  await composer(page).fill("open the figure library");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).not.toBeNull();
  const sendsBefore = (await invokeArgsList(page, "send_message")).length;
  const frameId = String((await lastInvokeArgs(page, "send_message")).sessionId);
  await page.evaluate(() => { (window as any).__mcpAppLiveBridges = true; });
  await page.evaluate(({ frameId, html }) => {
    (window as any).__tauriEmit("agent", {
      kind: "ToolPresentation",
      frame_id: frameId,
      presentation_id: "live-figure-library",
      presentation_kind: "mcp_app",
      payload: {
        tool: { name: "figure_search", title: "Figure Library" },
        arguments: {},
        result: { content: [], structuredContent: { hits: 1 } },
        resource: { uri: "ui://figure/library.html", text: html, _meta: {} },
      },
    });
  }, { frameId, html });

  const app = page.frameLocator('iframe[title="Figure Library"]');
  await expect(app.locator("#state")).toHaveText("preview:figure_preview_exact");
  await expect.poll(() => lastInvokeArgs(page, "call_mcp_app_tool")).toMatchObject({
    name: "figure_preview_exact",
    arguments: { id: "fig-1" },
  });
  expect((await invokeArgsList(page, "send_message")).length).toBe(sendsBefore);
});

test("repeated MCP App presentations reuse one center tab", async ({ page }) => {
  await enterApp(page);
  const appHtml = (state: string) =>
    `<!doctype html><html><body><div id="state">${state}</div></body></html>`;
  await composer(page).fill("open the figure library");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).not.toBeNull();
  const frameId = String((await lastInvokeArgs(page, "send_message")).sessionId);

  await emitTauriEvent(page, "agent", {
    kind: "ToolPresentation",
    frame_id: frameId,
    presentation_id: "open-1",
    presentation_kind: "mcp_app",
    payload: {
      tool: { name: "figure_open", title: "Open Scientific Figure Library" },
      arguments: {},
      result: { content: [] },
      resource: { uri: "ui://figure/library.html", text: appHtml("catalog"), _meta: {} },
    },
  });

  const figureTab = page.locator(
    `.center-tab[data-center-path="mcp-app:${frameId}:ui://figure/library.html"]`,
  );
  await expect(figureTab).toHaveCount(1);
  await expect(figureTab).toContainText("Open Scientific Figure Library");
  await expect(figureTab).toHaveAttribute("title", "Open Scientific Figure Library");
  await expect(page.frameLocator('iframe[title="Open Scientific Figure Library"]').locator("#state"))
    .toHaveText("catalog");

  await emitTauriEvent(page, "agent", {
    kind: "ToolPresentation",
    frame_id: frameId,
    presentation_id: "search-2",
    presentation_kind: "mcp_app",
    payload: {
      tool: { name: "figure_search", title: "Search scientific figure templates" },
      arguments: { q: "survival" },
      result: { content: [] },
      resource: {
        uri: "ui://figure/library.html?q=survival#hits",
        text: appHtml("survival"),
        _meta: {},
      },
    },
  });

  await expect(page.locator('.center-tab[data-center-path^="mcp-app:"]')).toHaveCount(1);
  await expect(figureTab).toContainText("Open Scientific Figure Library");
  await expect(page.frameLocator('iframe[title="Search scientific figure templates"]').locator("#state"))
    .toHaveText("survival");

  await emitTauriEvent(page, "agent", {
    kind: "ToolPresentation",
    frame_id: frameId,
    presentation_id: "motif-3",
    presentation_kind: "mcp_app",
    payload: {
      tool: { name: "motif_open_workbench", title: "Motif workbench" },
      arguments: {},
      result: { content: [] },
      resource: { uri: "ui://motif/workbench.html", text: appHtml("motif"), _meta: {} },
    },
  });

  await expect(page.locator('.center-tab[data-center-path^="mcp-app:"]')).toHaveCount(2);
  await expect(page.locator('.center-tab[data-center-path$="ui://motif/workbench.html"]'))
    .toContainText("Motif workbench");
});

test("reopening a saved session restores its MCP App workbench", async ({ page }) => {
  const openSavedSession = async () => {
    await page.locator(".proj-card-main").first().click();
    const app = page.frameLocator('iframe[title="Restored Motif workbench"]');
    await expect(app.locator("#state")).toHaveText("restored");
    await expect(page.locator('.center-tab[data-center-path^="mcp-app:"]'))
      .toContainText("Restored Motif workbench");
    await expect(page.locator("main.center")).toHaveClass(/split/);
  };

  await page.goto("/?mockMcpAppSession=1");
  await openSavedSession();
  await page.reload();
  await openSavedSession();
});

test("real Motif MCP App sends its internal sequence range through the Wisp host", async ({ page }) => {
  test.skip(!motifAppHtmlPath, "set WISP_MOTIF_APP_HTML for release acceptance");
  const html = readFileSync(motifAppHtmlPath!, "utf8");
  await enterApp(page);
  await composer(page).fill("open Motif acceptance");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).not.toBeNull();
  const frameId = String((await lastInvokeArgs(page, "send_message")).sessionId);
  await page.evaluate(({ frameId, html }) => {
    (window as any).__tauriEmit("agent", {
      kind: "ToolPresentation",
      frame_id: frameId,
      presentation_kind: "mcp_app",
      payload: {
        tool: {
          name: "motif_open_workbench",
          title: "Motif for Claude Science",
          description: "Open the Motif workbench",
          inputSchema: { type: "object", properties: {} },
        },
        arguments: { content: ">wisp-acceptance\nACGTACGT", filename: "wisp-acceptance.fasta" },
        result: {
          content: [{ type: "text", text: "Motif acceptance payload" }],
          structuredContent: {
            schema: "motif.mcp.workbench.v1",
            mode: "payload",
            recordCount: 1,
            residueCount: 8,
            payload: {
              schema: "motif.claude-science.inventory.v2",
              inventory: { title: "Wisp acceptance" },
              records: [{ id: "wisp-acceptance", name: "Wisp acceptance", sequence: "ACGTACGT", molecule: "dna" }],
            },
          },
          isError: false,
        },
        resource: {
          uri: "ui://motif/workbench.html",
          mimeType: "text/html;profile=mcp-app",
          text: html,
          _meta: { ui: { csp: { connectDomains: [], resourceDomains: [] } } },
        },
      },
    });
  }, { frameId, html });

  const motif = page.frameLocator('iframe[title="Motif for Claude Science"]');
  const sequence = motif.locator(".motif-cs-sequence");
  await expect(sequence).toBeVisible({ timeout: 20_000 });
  for (let index = 0; index < 8; index += 1) {
    await sequence.press("Shift+ArrowRight");
  }
  await expect(motif.locator(".motif-cs-selection-name")).toHaveText("1-8 (8)");
  await expect(motif.locator("[data-wisp-motif-selection-length]")).toHaveText("8 bp");

  const active = await sequence.evaluate(() => {
    const record = (window as any).motifGetActiveRecord();
    getSelection()?.removeAllRanges();
    return { name: record.name, sequence: record.seq.slice(0, 8) };
  });
  await page.getByRole("button", { name: "Add selection to chat" }).click();
  await expect(page.getByTestId("motif-selection-reference")).toContainText(active.name);
  await expect(page.getByTestId("motif-selection-reference")).toContainText("8 bp");
  await expect(composer(page)).toHaveValue(/Coordinates: 1-8 \(forward\)/);
  await expect(composer(page)).toHaveValue(/Length: 8 bp/);
  await expect(composer(page)).toHaveValue(new RegExp(`Sequence: ${active.sequence}`));
});

test("real SnapGene annotations render in the real Motif MCP App", async ({ page }) => {
  test.skip(!motifAppHtmlPath || !snapGeneFixturePath,
    "set WISP_MOTIF_APP_HTML and WISP_SNAPGENE_FIXTURE for release acceptance");
  const html = readFileSync(motifAppHtmlPath!, "utf8");
  await enterApp(page);
  await composer(page).fill("open annotated Motif acceptance");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).not.toBeNull();
  const frameId = String((await lastInvokeArgs(page, "send_message")).sessionId);
  await page.evaluate(({ frameId, html }) => {
    (window as any).__tauriEmit("agent", {
      kind: "ToolPresentation",
      frame_id: frameId,
      presentation_kind: "mcp_app",
      payload: {
        tool: { name: "motif_open_workbench", title: "Motif annotated acceptance" },
        arguments: { content: ">seed\nACGT", filename: "seed.fasta" },
        result: {
          content: [],
          structuredContent: {
            schema: "motif.mcp.workbench.v1",
            payload: { records: [{ name: "seed", type: "dna", sequence: "ACGT" }] },
          },
          isError: false,
        },
        resource: { uri: "ui://motif/workbench.html", text: html, _meta: {} },
      },
    });
    (window as any).__mcpAppLiveBridges = true;
    (window as any).__mcpAppToolResults = {
      motif_open_workbench: { content: [], structuredContent: { payload: { records: [] } }, isError: false },
    };
  }, { frameId, html });

  const motif = page.frameLocator('iframe[title="Motif annotated acceptance"]');
  await expect(motif.locator(".motif-cs-sequence")).toBeVisible({ timeout: 20_000 });
  const chooserPromise = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Load DNA file" }).click();
  const chooser = await chooserPromise;
  await chooser.setFiles(snapGeneFixturePath!);
  await expect.poll(() => lastInvokeArgs(page, "call_mcp_app_tool")).not.toBeNull();
  const call = await lastInvokeArgs(page, "call_mcp_app_tool");
  const record = call.arguments.payload.records[0];
  expect(record.topology).toBe("circular");
  expect(record.sequence.length).toBe(11_891);
  expect(record.annotations.length).toBeGreaterThanOrEqual(30);
  expect(record.annotations.map((annotation: any) => annotation.name)).toEqual(
    expect.arrayContaining(["T7 promoter", "RBS", "KanR", "ori", "lacI"]),
  );
  expect(record.annotations.find((annotation: any) => annotation.name === "KanR")).toMatchObject({
    type: "cds",
    strand: -1,
    color: expect.stringMatching(/^#/),
  });

  const rendered = await motif.locator("html").evaluate(async (_element, importedRecord) => {
    await (window as any).motifAddRecords([importedRecord]);
    const active = (window as any).motifGetActiveRecord();
    return {
      name: active.name,
      length: active.length,
      annotations: active.annotations.map((annotation: any) => annotation.name),
    };
  }, record);
  expect(rendered.name).toBe("pET-28a-250kd");
  expect(rendered.length).toBe(11_891);
  expect(rendered.annotations.length).toBeGreaterThanOrEqual(30);
  expect(rendered.annotations).toEqual(expect.arrayContaining(["T7 promoter", "KanR", "lacI"]));
  await expect(motif.locator("body")).toContainText("KanR");

  const mbp = record.annotations.find((annotation: any) => annotation.name === "MBP");
  expect(mbp).toBeTruthy();
  await motif.locator(`.motif-pm-feature[data-feature-id="${mbp.id}"]`).press("Enter");
  await expect(motif.locator(".motif-cs-selection-name")).toContainText("MBP");
  const mbpLength = mbp.end - mbp.start;
  await expect(motif.locator("[data-wisp-motif-selection-length]")).toHaveText(`${mbpLength} bp`);
  await expect.poll(() => motif.locator(".motif-cs-sequence-column").evaluate((pane) => {
    const block = pane.querySelector(".motif-cs-feature-block[aria-pressed=\'true\']");
    if (!block) return false;
    const blockRect = block.getBoundingClientRect();
    const paneRect = pane.getBoundingClientRect();
    return blockRect.top >= paneRect.top && blockRect.bottom <= paneRect.bottom;
  })).toBe(true);
  await page.getByRole("button", { name: "Add selection to chat" }).click();
  await expect(page.getByTestId("motif-selection-reference")).toContainText("MBP");
  await expect(page.getByTestId("motif-selection-reference")).toContainText(`${mbpLength} bp`);
  await expect(composer(page)).toHaveValue(/Feature: MBP/);
  await expect(composer(page)).toHaveValue(new RegExp(`Length: ${mbpLength} bp`));
  await expect(composer(page)).toHaveValue(
    new RegExp(`Coordinates: ${mbp.start + 1}-${mbp.end} \\(${mbp.strand === -1 ? "reverse" : "forward"}\\)`),
  );
  await expect(composer(page)).toHaveValue(
    new RegExp(`Sequence: ${record.sequence.slice(mbp.start, mbp.end)}`),
  );
});

test("Markdown artifact modal opens its rendered preview in center", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("analysis-report");
  await search.press("Enter");

  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".am-name")).toHaveText("analysis-report.md");
  await expect(modal.locator(".am-figure h1")).toHaveText("Differential expression report");
  await modal.getByRole("button", { name: "Open in center" }).click();

  await expect(modal).toHaveCount(0);
  await expect(page.locator('.center-tab[data-center-path="artifact:art-markdown"]')).toContainText("analysis-report.md");
  await expect(page.locator(".center-file-preview h1")).toHaveText("Differential expression report");
  await expect(page.locator(".center-file-preview")).toContainText("Rendered Markdown body.");
});

test("reverse preview selections anchor the action popup above the first selected line (#779)", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("analysis-report");
  await search.press("Enter");
  await page.locator(".artifact-modal").getByRole("button", { name: "Open in center" }).click();

  const preview = page.locator('.center-file-preview[data-file-path="artifact:art-markdown"]');
  await expect(preview.locator("h1")).toHaveText("Differential expression report");
  const selection = await preview.evaluate((host) => {
    const start = host.querySelector("h1")?.firstChild;
    const end = host.querySelector("p")?.firstChild;
    if (!(start instanceof Text) || !(end instanceof Text)) {
      throw new Error("Markdown preview did not render the expected text nodes");
    }

    // Anchor at the end and focus at the beginning to reproduce an upward drag.
    const selected = window.getSelection()!;
    selected.removeAllRanges();
    selected.setBaseAndExtent(end, end.data.length, start, 0);
    const range = selected.getRangeAt(0);
    const rects = Array.from(range.getClientRects())
      .filter((rect) => rect.width > 0 && rect.height > 0);
    const top = Math.min(...rects.map((rect) => rect.top));
    const bottom = Math.max(...rects.map((rect) => rect.bottom));
    const backward = selected.anchorNode === end && selected.focusNode === start;
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
    return { top, bottom, backward };
  });

  expect(selection.backward).toBe(true);
  expect(selection.bottom - selection.top).toBeGreaterThan(20);
  const popup = page.locator(".selection-popup");
  await expect(popup).toBeVisible();
  const anchorY = await popup.evaluate((element) => Number.parseFloat((element as HTMLElement).style.top));
  expect(anchorY).toBeCloseTo(selection.top, 0);
  await expect.poll(() => popup.evaluate((element) => element.getBoundingClientRect().bottom))
    .toBeLessThan(selection.top);
});

test("bound Markdown resources use immutable versions and a scrollable center preview", async ({ page }) => {
  await page.goto("/?mockResourceSession=1");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("Enumerate");
  await search.press("Enter");

  await page.getByRole("link", { name: "Open bound report" }).click();
  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".am-name")).toHaveText("report.md");
  // Downloading a pinned version must go through the version command — the
  // workspace-path download would fail on branch/exploration views where the
  // file only exists as a stored artifact version.
  await modal.getByRole("button", { name: "Download" }).click();
  await expect.poll(() => lastInvokeArgs(page, "download_artifact_version"))
    .toMatchObject({ versionId: "resource-version-markdown" });
  expect(await lastInvokeArgs(page, "download_file")).toBeNull();
  await modal.getByRole("button", { name: "Open in center" }).click();
  const tab = page.locator('.center-tab[data-center-path="artifact-version:resource-version-markdown"]');
  await expect(tab).toContainText("report.md");
  const preview = page.locator(".center-file-preview");
  await expect(preview.locator("h1")).toHaveText("Bound report");
  await expect(preview).toContainText("Scrollable row 120");
  await expect.poll(() => preview.evaluate((element) => ({
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
  }))).toMatchObject({ clientHeight: expect.any(Number), scrollHeight: expect.any(Number) });
  const dimensions = await preview.evaluate((element) => ({
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
  }));
  expect(dimensions.scrollHeight).toBeGreaterThan(dimensions.clientHeight);
  await preview.evaluate((element) => { element.scrollTop = element.scrollHeight; });
  await expect.poll(() => preview.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await expect.poll(() => lastInvokeArgs(page, "read_artifact_version"))
    .toMatchObject({ versionId: "resource-version-markdown" });
});

test("bound DOCX resources open their immutable preview", async ({ page }) => {
  await page.goto("/?mockResourceSession=1");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("Enumerate");
  await search.press("Enter");

  await page.getByRole("link", { name: "Open bound manuscript" }).click();
  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".am-name")).toHaveText("manuscript.docx");
  await modal.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator('.center-tab[data-center-path="artifact-version:resource-version-docx"]'))
    .toContainText("manuscript.docx");
  await expect(page.locator(".center-file-preview .rp-docx"))
    .toContainText("Differential expression of FX-cell markers");
  await expect.poll(() => lastInvokeArgs(page, "read_artifact_version_bytes"))
    .toMatchObject({ versionId: "resource-version-docx" });
});

test("bound Python resources open their immutable code preview", async ({ page }) => {
  await page.goto("/?mockResourceSession=1");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("Enumerate");
  await search.press("Enter");

  const pythonLink = page.getByRole("link", { name: "Open bound Python script" });
  await expect(pythonLink).toHaveAttribute("title", "random_walk_demo.py");
  await pythonLink.click();
  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".am-name")).toHaveText("random_walk_demo.py");
  const figure = modal.locator(".am-figure .rp-code-body code");
  await expect(figure).toHaveClass(/language-python/);
  await expect(figure).toContainText("SEED = 42");
  await modal.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator('.center-tab[data-center-path="artifact-version:resource-version-python"]'))
    .toContainText("random_walk_demo.py");
  const preview = page.locator(".center-file-preview");
  await expect(preview.locator(".center-file-head > span").first())
    .toHaveText("analysis/scripts/random_walk_demo.py");
  await expect(preview.locator(".center-file-head")).not.toContainText("artifact-version:");
  await expect(preview.locator(".center-file-snapshot-badge")).toHaveText("Snapshot");
  const pythonCode = preview.locator(".rp-code-body code");
  await expect(pythonCode).toHaveClass(/language-python/);
  await expect(pythonCode.locator(".hljs-string")).toHaveText("'random walk'");
  await expect(preview).toContainText("SEED = 42");
  await preview.getByRole("button", { name: "Open in editor" }).click();
  await expect(page.locator('.center-tab[data-center-path="analysis/scripts/random_walk_demo.py"]'))
    .toHaveClass(/active/);
  await expect(page.locator(".center-file-preview")).toHaveClass(/center-file-runtime-preview/);
  await expect.poll(() => lastInvokeArgs(page, "read_artifact_version"))
    .toMatchObject({ versionId: "resource-version-python" });
});

test("bound R resources open their immutable code preview", async ({ page }) => {
  await page.goto("/?mockResourceSession=1");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("Enumerate");
  await search.press("Enter");

  const rLink = page.getByRole("link", { name: "Open bound R script" });
  await expect(rLink).toHaveAttribute("title", "plot.R");
  await rLink.click();
  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".am-name")).toHaveText("plot.R");
  const figure = modal.locator(".am-figure .rp-code-body code");
  await expect(figure).toHaveClass(/language-r/);
  await expect(figure).toContainText("plot(1:3)");
  await modal.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator('.center-tab[data-center-path="artifact-version:resource-version-r"]'))
    .toContainText("plot.R");
  const preview = page.locator(".center-file-preview");
  await expect(preview.locator(".center-file-head > span").first()).toHaveText("analysis/plot.R");
  await expect(preview.locator(".center-file-head")).not.toContainText("artifact-version:");
  await expect(preview.locator(".center-file-snapshot-badge")).toHaveText("Snapshot");
  const rCode = preview.locator(".rp-code-body code");
  await expect(rCode).toHaveClass(/language-r/);
  await expect(rCode.locator(".hljs-string")).toHaveText('"data"');
  await expect(preview).toContainText("plot(1:3)");
  await expect.poll(() => preview.evaluate((element) => {
    const code = element.querySelector<HTMLElement>(".rp-code");
    if (!code) return 0;
    return code.getBoundingClientRect().height / element.getBoundingClientRect().height;
  })).toBeGreaterThan(0.75);
  await preview.getByRole("button", { name: "Open in editor" }).click();
  await expect(page.locator('.center-tab[data-center-path="analysis/plot.R"]')).toHaveClass(/active/);
  await expect(page.locator(".center-file-preview")).toHaveClass(/center-file-runtime-preview/);
  await expect(page.locator(".center-file-preview .rp-code-editor")).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "read_artifact_version"))
    .toMatchObject({ versionId: "resource-version-r" });
});

test("bound BibTeX resources open their immutable text preview", async ({ page }) => {
  await page.goto("/?mockResourceSession=1");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("Enumerate");
  await search.press("Enter");

  await page.getByRole("link", { name: "Open bound references" }).click();
  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".am-name")).toHaveText("references.bib");
  await modal.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator('.center-tab[data-center-path="artifact-version:resource-version-bib"]'))
    .toContainText("references.bib");
  await expect(page.locator(".center-file-preview"))
    .toContainText("@article{wisp");
  await expect(page.locator(".center-file-preview [data-open-editor]")).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "read_artifact_version"))
    .toMatchObject({ versionId: "resource-version-bib" });
});

// Programmatically select the rendered body of the center file preview and
// raise the quote popup (Playwright has no direct "select text" gesture).
async function selectCenterPreviewText(page: Page) {
  await page.evaluate(() => {
    const host = document.querySelector(".center-file-preview .md")
      ?? document.querySelector(".center-file-preview");
    if (!host) throw new Error("no center preview to select");
    const range = document.createRange();
    range.selectNodeContents(host);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
  });
}

test("selecting preview text quotes it into chat and saves a review annotation", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("analysis-report");
  await search.press("Enter");

  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await modal.getByRole("button", { name: "Open in center" }).click();
  const preview = page.locator(".center-file-preview");
  await expect(preview.locator("h1")).toHaveText("Differential expression report");
  await expect(preview).toHaveAttribute("data-file-path", "artifact:art-markdown");

  // Selecting inside the preview offers both conversation destinations plus
  // the preview-specific review action.
  await selectCenterPreviewText(page);
  const popup = page.locator(".selection-popup");
  await expect(popup).toBeVisible();
  await expect(popup.getByRole("button", { name: "Ask AI in the conversation" })).toBeVisible();
  await expect(popup.getByRole("button", { name: "Quote in side chat" })).toBeVisible();
  await expect(popup.getByRole("button", { name: "Add to review" })).toBeVisible();

  // The AI handoff opens the conversation and attaches the selection as a
  // composer quote chip (#274).
  await popup.getByRole("button", { name: "Ask AI in the conversation" }).click();
  await expect(page.locator(".chat")).toBeVisible();
  await expect(page.locator(".composer-reference-chips .quote")).toContainText("Differential expression report");

  // "Add to review" appends the passage to the reviews/ sidecar the agent reads.
  await selectCenterPreviewText(page);
  await page.locator(".selection-popup").getByRole("button", { name: "Add to review" }).click();
  await expect.poll(() => lastInvokeArgs(page, "append_review_note"))
    .toMatchObject({ sourcePath: "artifact:art-markdown" });
  await expect(page.locator(".topbar .hint")).toContainText("reviews/");
});

test("scratch chat opens from landing and closes on Escape", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".projects-screen")).toBeVisible();
  await page.getByRole("button", { name: "Scratch chat" }).click();
  await expect(page.locator(".app.scratch-mode")).toBeVisible();
  await expect(page.locator(".scratch-title")).toHaveText("Scratch chat");
  // Scratch chrome is title + close only — inbox/terminal/panel stay project-scoped.
  await expect(page.locator(".topbar-actions")).toBeHidden();
  await expect(page.locator(".scratch-close")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".app.scratch-mode")).toHaveCount(0);
  await expect(page.locator(".projects-screen")).toBeVisible();
});

test("projects landing stays centered on wide windows", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto("/");
  await expect(page.locator(".projects-head")).toBeVisible();
  await expect(page.locator(".projects-brand-mark")).toBeVisible();
  await expect(page.locator(".projects-title")).toHaveText("Wisp Science");
  await expect.poll(async () => page.locator(".projects-head").evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return Math.round(rect.width);
  })).toBeLessThanOrEqual(1200);
});

test("empty session shows the branded chat empty state", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  const empty = page.locator(".empty");
  await expect(empty).toBeVisible();
  await expect(empty.locator(".empty-logo")).toBeVisible();
  await expect.poll(() => empty.locator(".empty-logo").evaluate((el) =>
    Math.round(el.getBoundingClientRect().width)
  )).toBe(32);
  await expect(empty.locator("h1")).not.toBeEmpty();
  await expect(empty.locator("h1")).toHaveCSS("font-family", /Source Serif/);
});

test("Windows uses the integrated title bar without covering the project landing", async ({ browser }) => {
  const context = await browser.newContext({
    userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/136 Safari/537.36",
  });
  const page = await context.newPage();
  await page.addInitScript(tauriMock);
  await page.goto("/");

  await expect(page.locator(".window-titlebar")).toBeVisible();
  await expect(page.getByTestId("window-brand-title")).toHaveText("wisp science");
  await expect(page).toHaveTitle("wisp science");
  await expect(page.getByRole("button", { name: "Minimize" })).toBeVisible();
  await expect(page.getByTestId("window-maximize")).toBeVisible();
  await expect(page.locator("#titlebar-maximize")).toHaveAttribute("aria-label", "Maximize");
  await expect(page.locator(".window-titlebar [data-tauri-drag-region]")).toHaveCount(0);
  await expect(page.getByTestId("window-snap-drag")).toHaveCount(2);
  await expect.poll(async () => page.locator(".projects-screen").evaluate((el) =>
    Math.round(el.getBoundingClientRect().top)
  )).toBe(38);

  await globalSettingsButton(page).click();
  await expect.poll(async () => page.locator(".settings-page").evaluate((el) =>
    Math.round(el.getBoundingClientRect().top)
  )).toBe(38);
  await page.getByRole("button", { name: "Back to app" }).click();

  // Home menus only list actions that work without an open project.
  await page.getByRole("button", { name: "File", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "New project" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Import project" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Scratch chat" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Open settings" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "New session" })).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Open projects" })).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Export current project" })).toHaveCount(0);
  await page.getByRole("menuitem", { name: "New project" }).click();
  await expect(page.locator(".overlay .proj-settings-modal")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".proj-settings-modal")).toHaveCount(0);

  // Ctrl+N on the landing means a new project too, but Chromium never
  // delivers Ctrl+N to web content, so that path is only exercisable in the
  // real webview — the menu item above covers the same "new-project" action.

  await page.getByRole("button", { name: "File", exact: true }).click();
  await page.getByRole("menuitem", { name: "Import project" }).click();
  await expect(page.getByTestId("project-import-options")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("project-import-options")).toHaveCount(0);

  await page.getByRole("button", { name: "Edit", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "All commands" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Import Codex conversations" })).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "View", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Light theme" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Toggle sidebar" })).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();
  await page.getByRole("button", { name: "File", exact: true }).click();
  // Inside a workspace the menus flip back to the session-scoped set.
  await expect(page.getByRole("menuitem", { name: "New session" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "New project" })).toHaveCount(0);
  const exportCurrentProject = page.getByRole("menuitem", { name: "Export current project" });
  await expect(exportCurrentProject).toBeEnabled();
  await exportCurrentProject.click();
  const exportOptions = page.getByTestId("project-export-options");
  await expect(exportOptions).toBeVisible();
  await expect(exportOptions).toContainText("Copy this folder directly");
  await page.keyboard.press("Escape");
  await expect(exportOptions).toBeHidden();
  await expect(page.locator(".app")).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "export_project")).toBeNull();

  await page.getByRole("button", { name: "File", exact: true }).click();
  await page.getByRole("menuitem", { name: "Export current project" }).click();
  await page.getByTestId("project-export-options").getByRole("button", { name: "Export ZIP" }).click();
  await expect.poll(() => lastInvokeArgs(page, "export_project")).toMatchObject({ id: "default" });
  const transferProgress = page.getByTestId("project-transfer-progress");
  await expect(transferProgress).toContainText("Project export complete");
  await transferProgress.getByRole("button", { name: "Done" }).click();
  await expect(transferProgress).toBeHidden();

  await page.getByRole("button", { name: "Edit", exact: true }).click();
  await page.getByRole("menuitem", { name: "Import Codex conversations" }).click();
  await expect(page.locator('.codex-import-modal[data-provider="codex"]')).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".codex-import-modal")).toHaveCount(0);
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  await page.getByRole("menuitem", { name: "Import Claude Code conversations" }).click();
  await expect(page.locator('.codex-import-modal[data-provider="claude"]')).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".codex-import-modal")).toHaveCount(0);

  await page.getByRole("button", { name: "Edit", exact: true }).click();
  await page.getByRole("menuitem", { name: "Import session archive" }).click();
  await expect.poll(() => lastInvokeArgs(page, "import_session_archive")).not.toBeNull();
  await expect(page.locator(".copy-toast")).toContainText("Session imported (3 messages)");

  await page.getByRole("button", { name: "Help" }).click();
  await page.getByRole("menuitem", { name: "Documentation" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? [])
      .filter((c: any) => c.cmd === "open_external_url")
      .map((c: any) => (c.args instanceof Map ? c.args.get("url") : c.args?.url))
  )).toContain("https://github.com/xuzhougeng/wisp-science#readme");

  await context.close();
});

test("window title includes the open project name (#1017)", async ({ browser }) => {
  const context = await browser.newContext({
    userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/136 Safari/537.36",
  });
  const page = await context.newPage();
  await page.addInitScript(tauriMock);
  await page.goto("/");

  await expect(page.getByTestId("window-brand-title")).toHaveText("wisp science");
  await expect(page).toHaveTitle("wisp science");

  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();
  await expect(page.getByTestId("window-brand-title")).toHaveText("wisp science \u2014 wisp-science");
  await expect(page).toHaveTitle("wisp science \u2014 wisp-science");

  await page.locator(".proj-switch").click();
  await page.locator(".proj-menu").getByRole("button", { name: "Other project" }).click();
  await expect(page.locator(".proj-name")).toHaveText("Other project");
  await expect(page.getByTestId("window-brand-title")).toHaveText("wisp science \u2014 Other project");
  await expect(page).toHaveTitle("wisp science \u2014 Other project");

  await page.getByRole("button", { name: "Back to projects" }).click();
  await expect(page.locator(".projects-screen")).toBeVisible();
  await expect(page.getByTestId("window-brand-title")).toHaveText("wisp science");
  await expect(page).toHaveTitle("wisp science");

  await context.close();
});

test("Windows titlebar double-click maximizes and drag waits for movement", async ({ browser }) => {
  const context = await browser.newContext({
    userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/136 Safari/537.36",
  });
  const page = await context.newPage();
  await page.addInitScript(tauriMock);
  await page.goto("/");

  const drag = page.getByTestId("window-snap-drag").nth(1);
  const box = await drag.boundingBox();
  expect(box).not.toBeNull();
  const x = box!.x + box!.width / 2;
  const y = box!.y + box!.height / 2;

  await page.mouse.move(x, y);
  await page.mouse.down();
  expect(await page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "start_window_move")
  )).toBe(false);
  await page.mouse.move(x + 8, y);
  await expect.poll(() => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "start_window_move")
  )).toBeTruthy();
  await page.mouse.up();

  await page.evaluate(() => { (window as any).__skillInvokeLog = []; });
  await drag.dblclick();
  await expect.poll(() => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "toggle-maximize")
  )).toBeTruthy();
  expect(await page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "start_window_move")
  )).toBe(false);

  await context.close();
});

test("macOS uses the native title bar without the integrated header", async ({ browser }) => {
  const context = await browser.newContext({
    userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Safari/605.1.15",
  });
  const page = await context.newPage();
  await page.addInitScript(tauriMock);
  await page.goto("/");

  await expect(page.locator(".window-titlebar")).toHaveCount(0);
  await expect(page.locator(".window-controls")).toHaveCount(0);
  await expect(page.locator(".projects-screen")).toBeVisible();

  await globalSettingsButton(page).click();
  await expect.poll(async () => page.locator(".settings-page").evaluate((el) =>
    Math.round(el.getBoundingClientRect().top)
  )).toBe(0);
  await page.getByRole("button", { name: "Back to app" }).click();

  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();
  await openComputeMenu(page);
  await expect(page.locator('.compute-resource-row[data-context-id="ssh:gpu-server"]')).toBeVisible();

  await context.close();
});

test("project cards use semantic buttons for keyboard access", async ({ page }) => {
  await page.goto("/");
  const project = page.locator(".proj-card-main").first();
  await expect(project).toBeVisible();
  await expect(project.evaluate((el) => el.tagName)).resolves.toBe("BUTTON");
});

test("Escape closes settings and unwinds the composer picker before the right pane", async ({ page }) => {
  await page.goto("/");
  await globalSettingsButton(page).click();
  await expect(page.locator(".settings-page")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toHaveCount(0);

  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await expect(page.locator(".rightpane")).toBeVisible();
  await composer(page).press("@");
  await expect(page.locator(".mention-menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".mention-menu")).toHaveCount(0);
  await expect(page.locator(".rightpane")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".rightpane")).toHaveCount(0);
});

test("Windows titlebar menus close on Escape", async ({ browser }) => {
  const context = await browser.newContext({
    userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/136 Safari/537.36",
  });
  const page = await context.newPage();
  await page.addInitScript(tauriMock);
  await page.goto("/");

  await page.getByRole("button", { name: "File" }).click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);

  await context.close();
});

test("compact workspace keeps the conversation usable and opens Inspector as a drawer", async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 720 });
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();

  await expect(page.locator(".rightpane-backdrop")).toBeVisible();
  await expect(page.locator(".rightpane")).toHaveCSS("position", "fixed");
  await expect.poll(async () => page.locator(".center").evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThanOrEqual(700);

  await page.locator(".rightpane-backdrop").click({ position: { x: 16, y: 16 } });
  await expect(page.locator(".rightpane")).toHaveCount(0);
});

test("default Tauri workspace opens Inspector as a split pane", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 760 });
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();

  await expect(page.locator(".rightpane-backdrop")).toBeHidden();
  await expect(page.locator(".rightpane")).not.toHaveCSS("position", "fixed");
  await expect(page.locator(".resizer")).toBeVisible();
  await expect.poll(async () => page.locator(".center").evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThanOrEqual(400);
});

test("project switcher does not show a stale fallback name while opening", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => (window as any).__delayNextProjectOpen("default", 250));
  await page.locator(".proj-card-main").first().click();

  await expect(page.locator(".proj-name")).toHaveText("Opening project…");
  await expect(page.locator(".proj-name")).toHaveText("wisp-science");
});

test("project switcher has no caret and switches workspace in the current window", async ({ page }) => {
  await enterApp(page);
  const switcher = page.locator(".proj-switch");
  await expect(switcher.locator(".caret")).toHaveCount(0);

  await switcher.click();
  await page.locator(".proj-menu").getByRole("button", { name: "Other project" }).click();

  await expect.poll(() => lastInvokeArgs(page, "open_project")).toMatchObject({ id: "other" });
  await expect.poll(() => lastInvokeArgs(page, "open_project_window")).toBeNull();
  await expect(page.locator(".proj-name")).toHaveText("Other project");
});

test("opening a workspace resumes its most recent conversation by default", async ({ page }) => {
  await page.goto("/?mockLongSession=1");
  await page.locator(".proj-card-main").first().click();
  await expect(page.getByRole("tablist").getByRole("button", { name: "Long transcript" })).toBeVisible();
  await expect(page.getByText("Newest page first question")).toBeVisible();
});

test("opening a workspace skips a newer named unused draft when resuming", async ({ page }) => {
  await page.goto("/?mockLongSession=1&mockNamedUnusedDraft=1");
  await page.locator(".proj-card-main").first().click();
  await expect(page.getByRole("tablist").getByRole("button", { name: "Long transcript" })).toBeVisible();
  await expect(page.getByText("Newest page first question")).toBeVisible();
  await expect(page.locator(".side-item.ses", { hasText: "Named unused draft" })).toBeVisible();
  await expect(page.locator(".side-item.ses.active", { hasText: "Long transcript" })).toBeVisible();
});

test("default workspace keeps history labels and compact navigation keeps hover labels", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await enterApp(page);

  const sidebar = page.locator(".sidebar");
  const resizer = page.locator(".sidebar-resizer");
  await expect(resizer).toBeVisible();
  const before = await sidebar.evaluate((el) => Math.round(el.getBoundingClientRect().width));
  const box = await resizer.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + 80);
  await page.mouse.down();
  await page.mouse.move(box!.x + 160, box!.y + 80);
  await page.mouse.up();
  await expect.poll(async () => sidebar.evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThanOrEqual(before + 140);

  // 1100px is the default Tauri window width. It must keep the history area
  // readable rather than hiding all session text behind an icon-only rail.
  await page.setViewportSize({ width: 1100, height: 760 });
  await expect.poll(async () => sidebar.evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThan(200);
  await expect(page.locator(".side-hint")).toBeVisible();

  await page.setViewportSize({ width: 800, height: 720 });
  await expect.poll(async () => sidebar.evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeLessThanOrEqual(64);
  await expect(newSessionButton(page)).toHaveAttribute("title", "New session");
  await expect(page.locator(".proj-switch")).toHaveAttribute("title", /.+/);

  await page.locator(".proj-switch").click();
  const menu = page.locator(".proj-menu");
  await expect(menu).toBeVisible();
  await expect.poll(async () => menu.evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThanOrEqual(220);
  await expect(page.getByRole("button", { name: /Project settings|项目设置/ })).toBeVisible();
});

test("new project form enables Create after name and folder are set", async ({ page }) => {
  // Stay on the Projects landing screen (don't enter a project).
  await page.goto("/");
  await page.getByRole("button", { name: "New project" }).click();
  const overlay = page.locator(".overlay", { has: page.locator("#new-project-name") });
  await expect.poll(() => overlay.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      viewportWidth: innerWidth,
      viewportHeight: innerHeight,
    };
  })).toMatchObject({ x: 0, y: 0, width: 1280, height: 720, viewportWidth: 1280, viewportHeight: 720 });
  await expect.poll(() => overlay.locator(".modal").evaluate((el) => el.getBoundingClientRect().top)).toBeGreaterThanOrEqual(20);
  const create = page.getByRole("button", { name: "Create" });
  await expect(create).toBeDisabled();
  // Typing the name must register in the signal — a wrong event-target cast
  // used to panic in the input handler, leaving the name empty and Create
  // permanently disabled.
  await page.getByPlaceholder("Project name").pressSequentially("My Project");
  await page.locator(".pn-dir .btn-ghost").click(); // Choose folder → mock path
  await expect(page.locator(".pn-dir .path")).toHaveText("/mock/root/new-project");
  await expect(create).toBeEnabled();
});

test("import can open an existing folder in place without copying it", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Import project" }).click();
  const options = page.getByTestId("project-import-options");
  await expect(options).toBeVisible();
  await expect(options).toContainText("without copying it");

  // The choice dialog is the top layer: Escape closes only it immediately,
  // without moving focus first or leaving the Projects screen.
  await page.keyboard.press("Escape");
  await expect(options).toBeHidden();
  await expect(page.locator(".projects-screen")).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "import_project")).toBeNull();

  await page.getByRole("button", { name: "Import project" }).click();
  await options.getByRole("button", { name: "Open a folder in place" }).click();
  const form = page.locator(".overlay", { has: page.locator("#new-project-name") });
  await expect(form.getByRole("heading", { name: "Open project folder" })).toBeVisible();
  await expect(page.locator("#new-project-name")).toHaveValue("new-project");
  await expect(form.locator(".pn-dir .path")).toHaveText("/mock/root/new-project");
  await expect(form.locator(".pn-layout")).toHaveCount(0);
  await form.getByRole("button", { name: "Open project" }).click();

  await expect.poll(() => lastInvokeArgs(page, "create_project")).toMatchObject({
    name: "new-project",
    workspaceDir: "/mock/root/new-project",
    standardLayout: false,
  });
  await expect.poll(() => lastInvokeArgs(page, "import_project")).toBeNull();
});

test("project transfers stay in a lower-right progress card without blocking other projects", async ({ page }) => {
  await page.goto("/");
  await expect.poll(async () => page.evaluate(() =>
    (window as any).__tauriListenerReady?.("project-transfer-progress"),
  )).toBe(true);
  const projectCards = page.locator(".proj-card:not(.proj-example)");
  const projectCard = projectCards.first();
  const otherProjectCard = projectCards.nth(1);
  const exportProject = projectCard.getByRole("button", { name: "Export project" });
  await expect.poll(() => exportProject.evaluate((el) => Number.parseFloat(getComputedStyle(el).opacity))).toBeGreaterThan(0);
  await page.evaluate(() => (window as any).__delayNextProjectTransfer("export", 800));
  await exportProject.click();
  const exportOptions = page.getByTestId("project-export-options");
  await expect(exportOptions).toContainText("A ZIP is the complete portable copy");
  await expect(exportOptions).toContainText("/mock/root");
  await exportOptions.getByRole("button", { name: "Export ZIP" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "export_project"),
  )).toBe(true);
  const transferProgress = page.getByTestId("project-transfer-progress");
  await expect(transferProgress).toContainText("Exporting project");
  await expect(transferProgress).toContainText("Compressing workspace files");
  await expect(transferProgress).toContainText("data/example.tsv");
  await expect(page.locator(".project-transfer-overlay")).toHaveCount(0);
  await expect.poll(() => transferProgress.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      position: getComputedStyle(el).position,
      right: Math.round(innerWidth - rect.right),
      bottom: Math.round(innerHeight - rect.bottom),
    };
  })).toMatchObject({ position: "fixed", right: 20, bottom: 20 });
  await expect(projectCard.locator(".proj-card-main")).toBeDisabled();
  await expect(projectCard.locator(".pc-transfer-lock")).toContainText("read-only");
  await expect(otherProjectCard.locator(".proj-card-main")).toBeEnabled();
  await otherProjectCard.locator(".proj-card-main").click();
  await expect(page.locator(".proj-name")).toHaveText("Other project");
  await expect(transferProgress).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(transferProgress).toBeVisible();
  await expect(transferProgress).toContainText("Project export complete");
  await page.keyboard.press("Escape");
  await expect(transferProgress).toBeHidden();

  await page.goto("/");
  await expect.poll(async () => page.evaluate(() =>
    (window as any).__tauriListenerReady?.("project-transfer-progress"),
  )).toBe(true);
  await page.evaluate(() => (window as any).__delayNextProjectTransfer("import", 800));
  await page.getByRole("button", { name: "Import project" }).click();
  await page.getByTestId("project-import-options")
    .getByRole("button", { name: "Import a ZIP archive" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "import_project"),
  )).toBe(true);
  const importProgress = page.getByTestId("project-transfer-progress");
  await expect(importProgress).toContainText("Importing project");
  await expect(importProgress).toContainText("Extracting workspace files");
  await expect(importProgress).toContainText("workspace/data/example.tsv");
  await page.locator(".proj-card:not(.proj-example)").nth(1).locator(".proj-card-main").click();
  await expect(page.locator(".proj-name")).toHaveText("Other project");
  await expect(importProgress).toContainText("Project import complete");
  await expect(page.locator(".proj-name")).toHaveText("Other project");
  await importProgress.getByRole("button", { name: "Done" }).click();
  await expect(importProgress).toBeHidden();
});

test("project cards show the workspace path and aligned action icons", async ({ page }) => {
  await page.goto("/");
  const cards = page.locator(".proj-card:not(.proj-example)");
  // Issue #772: each card shows its workspace path so identically named
  // projects stay distinguishable before destructive actions.
  await expect(cards.first().locator(".pc-path")).toHaveText("/mock/root");
  await expect(cards.nth(1).locator(".pc-path")).toHaveText("/mock/other");
  // The relative timestamp sits on the name row, level with the project name,
  // instead of floating between the meta line and the action icons.
  const when = cards.first().locator(".pc-name-row .pc-when");
  await expect(when).toBeVisible();
  const nameBox = await cards.first().locator(".pc-name").boundingBox();
  const whenBox = await when.boundingBox();
  expect(Math.abs((nameBox!.y + nameBox!.height / 2) - (whenBox!.y + whenBox!.height / 2))).toBeLessThanOrEqual(4);
  // Action glyphs share one uniform box and one vertical center line.
  const boxes = await cards.first().locator(".pc-actions button").evaluateAll((els) =>
    els.map((el) => {
      const r = el.getBoundingClientRect();
      return { w: r.width, h: r.height, cy: r.y + r.height / 2 };
    }));
  expect(boxes.length).toBeGreaterThan(0);
  for (const box of boxes) {
    expect(box.w).toBeCloseTo(boxes[0].w, 1);
    expect(box.h).toBeCloseTo(boxes[0].h, 1);
    expect(box.cy).toBeCloseTo(boxes[0].cy, 1);
  }
});

test("landing Settings stays distinct from Project settings", async ({ page }) => {
  await page.goto("/");

  const appSettings = globalSettingsButton(page);
  const projectSettings = page.getByTestId("project-card-settings");
  await expect(appSettings).toHaveCount(1);
  await expect(projectSettings.first()).toBeVisible();
  await expect(appSettings).not.toHaveAttribute("data-testid", "project-card-settings");

  await appSettings.click();
  await expect(page.locator(".settings-page")).toBeVisible();
  await expect(page.getByTestId("project-home-settings")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toHaveCount(0);

  await projectSettings.first().click();
  await expect(page.getByTestId("project-home-settings")).toBeVisible();
  await expect(page.locator(".settings-page")).toHaveCount(0);
});

test("project cards open settings without entering the project (#905)", async ({ page }) => {
  await page.goto("/");
  const otherCard = page.locator(".proj-card:not(.proj-example)", { hasText: "Other project" });
  await otherCard.getByTestId("project-card-settings").click();
  const settings = page.getByTestId("project-home-settings");
  await expect(settings).toBeVisible();
  await expect(page.locator(".projects-screen")).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "get_project_settings")).toMatchObject({
    id: "other",
  });
  await expect.poll(() => lastInvokeArgs(page, "open_project")).toBeNull();

  await page.keyboard.press("Escape");
  await expect(settings).toHaveCount(0);
  await expect(page.locator(".projects-screen")).toBeVisible();

  await otherCard.getByTestId("project-card-settings").click();
  await expect(settings).toBeVisible();
  await settings.getByTestId("project-home-settings-name").fill("Formal name");
  await settings.getByTestId("save-project-home-settings").click();
  await expect.poll(() => lastInvokeArgs(page, "update_project")).toMatchObject({
    id: "other",
    name: "Formal name",
  });
  await expect.poll(() => lastInvokeArgs(page, "open_project")).toBeNull();
  await expect(settings).toHaveCount(0);
  await expect(page.locator(".proj-card:not(.proj-example)", { hasText: "Formal name" })).toBeVisible();
  await expect(page.locator(".projects-screen")).toBeVisible();
});

test("project home settings confirm Escape leaves the settings dialog open", async ({ page }) => {
  await page.goto("/");
  await page.locator(".proj-card:not(.proj-example)").first().getByTestId("project-card-settings").click();
  const settings = page.getByTestId("project-home-settings");
  await expect(settings).toBeVisible();
  await settings.locator("textarea.ps-ctx").fill("Prefer the home card setting.");
  await settings.getByTestId("save-project-home-settings").click();
  const confirm = page.getByTestId("project-home-settings-confirm");
  await expect(confirm).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(confirm).toHaveCount(0);
  await expect(settings).toBeVisible();
});

test("projects home suppresses the native context menu except in text fields", async ({ page }) => {
  await page.goto("/");
  const card = page.locator(".proj-card:not(.proj-example)").first();
  const cardPrevented = await card.evaluate((el) => {
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    el.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(cardPrevented).toBe(true);

  await card.getByTestId("project-card-settings").click();
  const namePrevented = await page.getByTestId("project-home-settings-name").evaluate((el) => {
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    el.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(namePrevented).toBe(false);
});

test("projects sync manually, copy a device code, and join on another device", async ({ page }) => {
  await page.goto("/");
  const projectCard = page.locator(".proj-card:not(.proj-example)").first();
  await projectCard.hover();
  await projectCard.getByRole("button", { name: "Sync now" }).click();
  await expect(page.locator(".projects-sync-notice")).toContainText("Uploaded 1 changed workspace file");
  await expect(projectCard.locator(".pc-sync-state")).toContainText("Synced");
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "sync_project"),
  )).toBe(true);

  await projectCard.hover();
  await projectCard.getByRole("button", { name: "Copy device code" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "project_sync_code"),
  )).toBe(true);

  await expect(page.getByRole("button", { name: "Join synced project" })).toHaveCount(0);
  await globalSettingsButton(page).click();
  await page.getByRole("button", { name: "Remote Access", exact: true }).click();
  await page.getByRole("button", { name: "Join synced project" }).click();
  const joinDialog = page.getByRole("dialog", { name: "Join a synced project" });
  const deviceCode = page.getByTestId("sync-device-code");
  await expect(joinDialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(joinDialog).toHaveCount(0);
  await expect(page.locator(".settings-page")).toBeVisible();

  await page.getByRole("button", { name: "Join synced project" }).click();
  await expect(joinDialog).toBeVisible();
  await expect(page.getByText("Secret device code", { exact: true })).toBeVisible();
  await expect.poll(async () => joinDialog.evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThanOrEqual(520);
  await expect.poll(async () => joinDialog.getByRole("button", { name: "Cancel" }).first().evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return [Math.round(rect.width), Math.round(rect.height)];
  })).toEqual([34, 34]);

  await joinDialog.getByRole("button", { name: "Read sync guide" }).click();
  await expect.poll(() => lastInvokeArgs(page, "open_external_url")).toMatchObject({
    url: expect.stringContaining("docs/project-sync.md"),
  });

  await deviceCode.fill("wisp-sync:mock-secret-code");
  await page.getByRole("button", { name: "Choose destination and join" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "join_synced_project"),
  )).toBe(true);
});

test("project sync actions appear only after a sync backend is configured", async ({ page }) => {
  await page.goto("/?mockSyncUnconfigured=1");
  const projectCard = page.locator(".proj-card:not(.proj-example)").first();
  await expect(projectCard.getByRole("button", { name: "Sync now" })).toHaveCount(0);
  await expect(projectCard.getByRole("button", { name: "Copy device code" })).toHaveCount(0);

  await openSettingsSection(page, "Remote Access");
  await page.getByTestId("sync-relay-url").fill("https://relay.example.test");
  await page.getByTestId("sync-relay-token").fill("secret-token");
  await page.locator(".settings-footer").getByRole("button", { name: "Save" }).click();

  await expect(projectCard.getByRole("button", { name: "Sync now" })).toBeVisible();
  await expect(projectCard.getByRole("button", { name: "Copy device code" })).toBeVisible();
});

test("remote access settings configure a cloud-drive sync folder", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "Remote Access");
  await page.getByTestId("sync-backend").selectOption("folder");
  await page.locator(".settings-path-row").getByRole("button", { name: "Choose folder" }).click();
  await expect(page.getByTestId("sync-folder")).toHaveValue("/mock/root/new-project");
  await page.locator(".settings-footer").getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_settings")).toMatchObject({
    settings: { sync_backend: "folder", sync_folder: "/mock/root/new-project" },
  });
});

test("session settings save the maximum agent iterations", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "Session");
  await expect(page.getByTestId("session-settings-pane")).toBeVisible();
  await page.getByTestId("max-iter").fill("0");
  await page.locator(".settings-footer").getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_settings")).toMatchObject({
    settings: { max_iter: 0 },
  });
});

test("session settings enable automatic context compaction by default", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "Session");
  const toggle = page.getByTestId("auto-compact-enabled");
  await expect(toggle).toBeChecked();
  await toggle.locator("..").click();
  await expect(toggle).not.toBeChecked();
  await page.locator(".settings-footer").getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_settings")).toMatchObject({
    settings: { auto_compact: false },
  });
});

test("session settings configure truncated-output auto-continue", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "Session");
  const toggle = page.getByTestId("auto-continue-enabled");
  await expect(toggle).not.toBeChecked();
  await toggle.locator("..").click();
  await page.getByTestId("auto-continue-limit").fill("4");
  await page.locator(".settings-footer").getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_settings")).toMatchObject({
    settings: { auto_continue: true, auto_continue_limit: 4 },
  });
});

test("session settings enable follow-up question suggestions by default", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "Session");
  const toggle = page.getByTestId("follow-up-questions-enabled");
  await expect(toggle).toBeChecked();
  await toggle.locator("..").click();
  await expect(toggle).not.toBeChecked();
  await page.locator(".settings-footer").getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_settings")).toMatchObject({
    settings: { follow_up_questions: false },
  });
});

test("general settings keep workspace prefs without agent loop or proxy controls", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "General");
  await expect(page.getByTestId("settings-language")).toBeVisible();
  await expect(page.getByTestId("resume-last-session-enabled")).toBeAttached();
  await expect(page.getByTestId("max-iter")).toHaveCount(0);
  await expect(page.getByTestId("proxy-url")).toHaveCount(0);
});

test("model settings save the model API proxy", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "Models");
  await expect(page.getByTestId("proxy-url")).toBeVisible();
  await page.getByTestId("proxy-url").fill("none");
  await page.locator(".model-settings-pane .settings-footer").getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_settings")).toMatchObject({
    settings: { proxy_url: "none" },
  });
});

test("general settings resume the last workspace conversation by default", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "General");
  const toggle = page.getByTestId("resume-last-session-enabled");
  await expect(toggle).toBeChecked();
  await toggle.locator("..").click();
  await expect(toggle).not.toBeChecked();
  await page.locator(".settings-footer").getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_settings")).toMatchObject({
    settings: { resume_last_session: false },
  });
});

test("context compaction leaves a visible timeline flag", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("start a context-heavy analysis");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  const frameId = String((await lastInvokeArgs(page, "send_message")).sessionId);

  await emitTauriEvent(page, "agent", {
    kind: "CompactionStarted",
    frame_id: frameId,
    strategy: "auto",
  });
  await expect(page.getByTestId("context-compaction-live")).toContainText("Centrifuging context");

  await emitTauriEvent(page, "agent", {
    kind: "Compaction",
    frame_id: frameId,
    before: 812_000,
    after: 236_000,
    strategy: "auto",
  });

  const flag = page.getByTestId("context-compaction-flag");
  await expect(page.getByTestId("context-compaction-live")).toBeHidden();
  await expect(flag).toContainText("Context automatically compacted");
  await expect(flag).toContainText("812.0k → 236.0k");
});

test("context-limit recovery offers three actions and owns the first Escape", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("continue a long analysis");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  const frameId = String((await lastInvokeArgs(page, "send_message")).sessionId);
  const overflow = {
    kind: "Error",
    frame_id: frameId,
    message: 'api: 400 {"error":{"message":"maximum context length exceeded"}}',
  };

  await emitTauriEvent(page, "agent", overflow);
  const modal = page.getByTestId("context-recovery-modal");
  await expect(modal).toBeVisible();
  await expect(page.getByTestId("context-recovery-compact")).toBeVisible();
  await expect(page.getByTestId("context-recovery-new-session")).toBeVisible();
  await expect(page.getByTestId("context-recovery-pause")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  await expect(page.getByText(/maximum context length exceeded/)).toBeVisible();

  await emitTauriEvent(page, "agent", overflow);
  await page.getByTestId("context-recovery-pause").click();
  await expect(modal).toHaveCount(0);

  await emitTauriEvent(page, "agent", overflow);
  await page.getByTestId("context-recovery-compact").click();
  await expect.poll(async () => {
    const calls = await invokeArgsList(page, "send_message");
    return calls.some((args) => args.message === "/compact")
      && calls.some((args) => args.resume === true);
  }).toBe(true);
});

test("context-limit recovery can continue in a new session with the old one attached", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("finish the long analysis");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  const frameId = String((await lastInvokeArgs(page, "send_message")).sessionId);

  await emitTauriEvent(page, "agent", {
    kind: "Error",
    frame_id: frameId,
    message: 'api: 400 {"error":{"message":"context window exceeded"}}',
  });
  await page.getByTestId("context-recovery-new-session").click();

  await expect.poll(async () => {
    const calls = await invokeArgsList(page, "send_message");
    return calls.find((args) =>
      Array.isArray(args.references)
      && args.references.some((reference: any) =>
        reference.kind === "session" && reference.id === frameId,
      ),
    ) ?? null;
  }).toMatchObject({
    references: [{ kind: "session", id: frameId }],
  });
});

test("a leftover proxy connect error points at Model API proxy", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("hello");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  const frameId = String((await lastInvokeArgs(page, "send_message")).sessionId);

  await emitTauriEvent(page, "agent", {
    kind: "Error",
    frame_id: frameId,
    message: "http: error sending request: tcp connect error: Connection refused (os error 111) (via leftover HTTPS_PROXY=http://127.0.0.1:7890)",
  });

  const card = page.locator(".finding.err");
  await expect(card).toBeVisible();
  await expect(card.locator(".finding-body")).toContainText("Model API proxy");
  await expect(card.locator(".finding-body")).toContainText("none");
});

test("pet stays off until the user explicitly configures its directory", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "Pet");

  await expect(page.getByTestId("pet-enabled")).not.toBeChecked();
  await expect(page.getByTestId("pet-directory")).toHaveValue("");
  await page.getByTestId("pet-directory").fill("C:\\Users\\tester\\.codex\\pets\\wispy");
  await page.locator(".pet-settings-pane .toggle").click();
  await page.locator(".pet-settings-pane .settings-footer").getByRole("button", { name: "Save" }).click();

  await expect.poll(() => lastInvokeArgs(page, "set_settings")).toMatchObject({
    settings: {
      pet_enabled: true,
      pet_directory: "C:\\Users\\tester\\.codex\\pets\\wispy",
    },
  });

  await page.goto("/?pet=desktop&mockPet=1");
  const pet = page.getByTestId("wisp-pet");
  await expect(pet).toBeVisible();
  await expect.poll(() => pet.getAttribute("data-state")).toMatch(/^(idle|looking)$/);
  await pet.click();
  await expect(pet).toHaveAttribute("data-state", "waving");
});

test("disabled desktop pet skips runtime run polling", async ({ page }) => {
  await page.goto("/?pet=desktop");

  // Wait for both the initial settings read and one interval tick. Neither
  // should query the run snapshot while the pet is disabled.
  await expect.poll(() => invokeCount(page, "get_pet")).toBeGreaterThanOrEqual(2);
  expect(await invokeCount(page, "get_pet_runtime_status")).toBe(0);
  await expect.poll(() => page.evaluate(() => (window as any).__petWindowVisible)).toBe(false);
});

test("desktop pet remains independent and reflects global agent state", async ({ page }) => {
  await page.setViewportSize({ width: 128, height: 176 });
  await page.goto("/?pet=desktop&mockPet=1");

  const pet = page.getByTestId("wisp-pet");
  await expect(page.getByTestId("pet-window-root")).toBeVisible();
  await expect(pet).toBeVisible();
  await expect(pet).toHaveAttribute("data-tauri-drag-region", "deep");
  await expect.poll(() => page.evaluate(() => (window as any).__petWindowVisible)).toBe(true);

  await page.evaluate(() => {
    (window as any).__tauriEmit("agent", { kind: "User", frame_id: "pet-frame", text: "run" });
  });
  await expect(pet).toHaveAttribute("data-state", "running");
  const workingLabel = pet.getByText("Working");
  await expect(workingLabel).toBeVisible();
  await expectInsideViewport(workingLabel, 128, 176);

  await page.evaluate(() => {
    (window as any).__tauriEmit("confirm-request", { frame_id: "pet-frame", message: "Approve?" });
  });
  await expect(pet).toHaveAttribute("data-state", "waiting");
  const waitingLabel = pet.getByText("Needs you");
  await expect(waitingLabel).toBeVisible();
  await expectInsideViewport(waitingLabel, 128, 176);
  await pet.click();
  await expect.poll(() => lastInvokeArgs(page, "open_pet_session")).toMatchObject({
    sessionId: "pet-frame",
  });

  await page.evaluate(() => {
    (window as any).__tauriEmit("agent", { kind: "Text", frame_id: "pet-frame", delta: "continuing" });
    (window as any).__tauriEmit("agent", { kind: "ReviewStarted", frame_id: "pet-frame" });
  });
  await expect(pet).toHaveAttribute("data-state", "review");
  await expect(pet.getByText("Reviewing")).toBeVisible();

  await page.evaluate(() => {
    (window as any).__tauriEmit("agent", { kind: "Error", frame_id: "pet-frame", message: "failed" });
  });
  await expect(pet).toHaveAttribute("data-state", "failed");
  await expect(pet.getByText("Failed")).toBeVisible();

  await page.evaluate(() => {
    (window as any).__tauriEmit("agent", { kind: "Done", frame_id: "pet-frame" });
  });
  await expect(pet).toHaveAttribute("data-state", "jumping");
});

test("desktop pet shows active Run titles and celebrates completion (#693)", async ({ page }) => {
  await page.setViewportSize({ width: 128, height: 176 });
  await page.addInitScript(() => {
    (window as any).__mockPetActiveRuns = [
      { id: "run-data", title: "siibra atlas query example (with assignment)" },
    ];
  });
  await page.goto("/?pet=desktop&mockPet=1");

  const pet = page.getByTestId("wisp-pet");
  const sprite = pet.locator(".wisp-pet-sprite");
  const label = pet.locator(".wisp-pet-state-label");
  await expect(pet).toHaveAttribute("data-tauri-drag-region", "deep");
  await expect(label).toHaveText("Running: siibra atlas query example (with assignment)");
  await expect(label).toBeVisible();
  await expect(pet).toHaveAttribute("data-state", "running");
  const [spriteBox, labelBox] = await Promise.all([sprite.boundingBox(), label.boundingBox()]);
  await expectInsideViewport(label, 128, 176);
  expect(labelBox!.y + labelBox!.height).toBeLessThanOrEqual(spriteBox!.y);

  await page.evaluate(() => {
    (window as any).__mockPetActiveRuns = [];
  });
  await expect(pet).toHaveAttribute("data-state", "jumping", { timeout: 5_000 });
  await expect(pet.getByText("Done")).toBeVisible();
});

test("notification navigation opens the project and session that need the user (#499)", async ({ page }) => {
  await page.goto("/");
  await expect.poll(() => page.evaluate(() =>
    (window as any).__tauriListenerReady("open-session"),
  )).toBe(true);
  // `open-session` is emitted to one native window. Registering through the
  // app-wide event bus makes every open project window consume that target and
  // collapse onto the same project/session after task completion.
  await expect.poll(() => page.evaluate(() =>
    (window as any).__tauriListenerScope("open-session"),
  )).toBe("window");
  await page.evaluate(() => {
    (window as any).__tauriEmit("open-session", {
      projectId: "other",
      sessionId: "pet-frame",
    });
  });

  await expect.poll(() => lastInvokeArgs(page, "open_project")).toMatchObject({ id: "other" });
  await expect.poll(() => lastInvokeArgs(page, "load_session")).toMatchObject({ id: "pet-frame" });
});

test("a sync conflict requires an explicit authoritative device choice", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => { (window as any).__failSyncConflict = true; });
  const projectCard = page.locator(".proj-card:not(.proj-example)").first();
  await projectCard.hover();
  await projectCard.getByRole("button", { name: "Sync now" }).click();
  await expect(page.getByRole("dialog", { name: "Both devices changed this project" })).toBeVisible();
  await page.getByRole("button", { name: "Use remote version" }).click();
  await expect.poll(() => lastInvokeArgs(page, "resolve_project_sync")).toMatchObject({
    id: "default", strategy: "remote",
  });
});

test("a second conversation can run in parallel without interleaving transcripts", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  // Start conversation A. The mock streams A's reply at once but delays Done,
  // so A stays "running".
  await composer(page).fill("alpha");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(userTurn(page, "alpha")).toBeVisible({ timeout: 10_000 });
  await expect(assistantReplyQuoting(page, "alpha")).toBeVisible({ timeout: 10_000 });

  // While A is still running, open a fresh session. The composer must be usable
  // (per-session busy: A running does NOT block B).
  await newSessionButton(page).click();
  await expect(composer(page)).toBeEmpty();
  await composer(page).fill("beta");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(userTurn(page, "beta")).toBeVisible({ timeout: 10_000 });
  await expect(assistantReplyQuoting(page, "beta")).toBeVisible({ timeout: 10_000 });

  // A's transcript must not leak into B's view.
  await expect(userTurn(page, "alpha")).toHaveCount(0);
  await expect(assistantReplyQuoting(page, "alpha")).toHaveCount(0);

  // A is still running → its sidebar entry shows the running indicator.
  await expect(page.locator(".side-item.ses.running", { hasText: "alpha" })).toBeVisible();

  // Switch back to A: the cached (live) transcript renders, B's does not.
  await page.locator(".side-item.ses", { hasText: "alpha" }).click();
  await expect(userTurn(page, "alpha")).toBeVisible({ timeout: 10_000 });
  await expect(assistantReplyQuoting(page, "alpha")).toBeVisible({ timeout: 10_000 });
  await expect(userTurn(page, "beta")).toHaveCount(0);
  await expect(assistantReplyQuoting(page, "beta")).toHaveCount(0);
});

test("delayed session loads cannot expose or overwrite another live transcript (#595)", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  // Keep A running, then create a short completed B so switching back to B
  // takes the asynchronous idle-session load path.
  await composer(page).fill("alpha");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(assistantReplyQuoting(page, "alpha")).toBeVisible({ timeout: 10_000 });
  await newSessionButton(page).click();
  await composer(page).fill("actions-beta");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(assistantReplyQuoting(page, "actions-beta")).toBeVisible();
  await expect(page.locator(".side-item.ses", { hasText: "actions-beta" })).toBeVisible();

  await page.locator(".side-item.ses", { hasText: "alpha" }).click();
  await expect(assistantReplyQuoting(page, "alpha")).toBeVisible();
  await page.evaluate(() => { (window as any).__parallelLoadDelayMs = 800; });

  // The target cache must be installed before active_session changes. Under the
  // old ordering, A remained visible here until B's delayed load completed.
  const loadsBeforeSwitch = await page.evaluate(() =>
    Number((window as any).__parallelLoadsResolved ?? 0));
  await page.locator(".side-item.ses", { hasText: "actions-beta" }).click();
  await expect(assistantReplyQuoting(page, "alpha")).toHaveCount(0);
  await expect(assistantReplyQuoting(page, "actions-beta")).toBeVisible();

  // Start a live B turn while its old DB page is still loading. The late empty
  // snapshot must not erase the streamed result.
  await composer(page).fill("gamma");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(assistantReplyQuoting(page, "gamma")).toBeVisible();
  // Wait for the delayed snapshot to actually arrive before asserting it did
  // not erase the live turn (not a fixed sleep hoping it has landed).
  await expect
    .poll(() => page.evaluate(() => Number((window as any).__parallelLoadsResolved ?? 0)))
    .toBeGreaterThan(loadsBeforeSwitch);
  await expect(assistantReplyQuoting(page, "gamma")).toBeVisible();
  await expect(assistantReplyQuoting(page, "alpha")).toHaveCount(0);

  await page.locator(".side-item.ses", { hasText: "alpha" }).click();
  await expect(assistantReplyQuoting(page, "alpha")).toBeVisible();
  await expect(assistantReplyQuoting(page, "gamma")).toHaveCount(0);
});

test("a running conversation accepts another message for queueing", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  await composer(page).fill("alpha");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(assistantReplyQuoting(page, "alpha")).toBeVisible({ timeout: 10_000 });

  await composer(page).fill("queued");
  // Queue (#433): sending into a busy session queues directly — no dialog. The
  // busy send button reads "Queue…" and the message parks in the composer queue.
  const send = page.getByRole("button", { name: "Queue…" });
  await expect(send).toBeEnabled({ timeout: 500 });
  await send.click();
  const queued = page.locator(".msg.user.queued .body", { hasText: /^queued$/ });
  await expect(queued).toBeVisible({ timeout: 500 });
  await expect(page.getByTestId("composer-queue")).toBeVisible();
  await expect(page.locator(".thread .msg.user.queued")).toHaveCount(0);
  await expect(page.getByTestId("composer-queue")).toContainText("1 queued");
  await expect(page.locator(".msg.user.queued").getByTitle("Move up")).toHaveCount(0);

  // The parked turn goes through enqueue_turn, not send_message.
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__sendInvokeLog ?? [])
      .filter((c: any) => c.cmd === "enqueue_turn")
      .map((c: any) => c.args?.message),
  )).toEqual(["queued"]);

  // The first turn keeps streaming after the second is queued. Its tail must
  // stay attached to the first assistant row instead of leaking into a hidden
  // placeholder after the queued user message (#143). "queued" must NOT run yet.
  await expect(page.getByText(parallelReplyTailText("alpha"), { exact: true }))
    .toBeVisible({ timeout: 3_000 });
  await expect(queued).toBeVisible();
  await expect(assistantReplyQuoting(page, "queued")).toHaveCount(0);

  // Only "alpha" ran as a live turn; the queue waits behind it.
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__sendInvokeLog ?? [])
      .filter((c: any) => c.cmd === "send_message")
      .map((c: any) => c.args?.message),
  )).toEqual(["alpha"]);

  // Once alpha finishes, the driver drains the queue: "queued" now runs and its
  // optimistic bubble promotes to a live turn (#433).
  await expect(assistantReplyQuoting(page, "queued")).toBeVisible({ timeout: 10_000 });
});

test("queued follow-ups can be reordered up and down (#433)", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  // alpha stays running ~5s, keeping the session busy while we queue two more.
  await composer(page).fill("alpha");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(assistantReplyQuoting(page, "alpha")).toBeVisible({ timeout: 10_000 });

  for (const msg of ["bravo", "charlie"]) {
    await composer(page).fill(msg);
    await page.getByRole("button", { name: "Queue…" }).click();
    await expect(
      page.locator(".msg.user.queued .body", { hasText: new RegExp(`^${msg}$`) }),
    ).toBeVisible({ timeout: 1_000 });
  }

  const queuedTexts = () => page.locator(".msg.user.queued .body").allInnerTexts();
  expect(await queuedTexts()).toEqual(["bravo", "charlie"]);
  await expect(page.getByTestId("composer-queue")).toContainText("2 queued");

  // Move charlie up → [charlie, bravo]; then back down → [bravo, charlie].
  const charlieRow = page.locator(".msg.user.queued", { hasText: "charlie" });
  await charlieRow.getByTitle("Move up").click();
  await expect.poll(queuedTexts).toEqual(["charlie", "bravo"]);
  await charlieRow.getByTitle("Move down").click();
  await expect.poll(queuedTexts).toEqual(["bravo", "charlie"]);

  // Both reorders were mirrored to the backend queue.
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__sendInvokeLog ?? [])
      .filter((c: any) => c.cmd === "queued_turn_action")
      .map((c: any) => c.args?.action),
  )).toEqual(["move_up", "move_down"]);
});

test("editing a queued follow-up restores it to the composer", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  await composer(page).fill("alpha");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(assistantReplyQuoting(page, "alpha")).toBeVisible({ timeout: 10_000 });

  await composer(page).fill("what is QC?");
  await page.getByRole("button", { name: "Queue…" }).click();
  const queued = page.locator(".msg.user.queued", { hasText: "what is QC?" });
  await expect(queued).toBeVisible({ timeout: 500 });
  await expect(queued.getByRole("button", { name: "Guide now" })).toBeVisible();

  await queued.getByRole("button", { name: "Edit" }).click();
  await expect(queued).toHaveCount(0);
  await expect(composer(page)).toHaveValue("what is QC?");
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__sendInvokeLog ?? [])
      .filter((c: any) => c.cmd === "queued_turn_action")
      .map((c: any) => c.args?.action),
  )).toEqual(["cancel"]);
});

test("project removal offers a files-preserving action in the in-app dialog (#96)", async ({ page }) => {
  // Native window.confirm() is a no-op in this webview (wry's WKUIDelegate has
  // no JS confirm panel), so the ✕ silently did nothing. Deletion now goes
  // through an in-app modal.
  await page.goto("/");
  await page.locator(".proj-card:not(.proj-example) .pc-del").first().click();
  const modal = page.locator(".project-delete-choice-modal");
  await expect(modal).toBeVisible();
  await expect(modal.getByRole("button", { name: "Cancel", exact: true })).toBeVisible();
  await expect(modal.getByRole("button", { name: "Remove from Wisp only", exact: true })).toBeVisible();
  await expect(modal.getByRole("button", { name: "Delete project and local data", exact: true })).toBeVisible();

  // Escape works immediately after opening; it does not depend on modal focus.
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  await page.locator(".proj-card:not(.proj-example) .pc-del").first().click();
  await page.locator(".project-delete-choice-modal")
    .getByRole("button", { name: "Remove from Wisp only", exact: true })
    .click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? [])
      .filter((c: any) => c.cmd === "delete_project")
      .map((c: any) => c.args instanceof Map ? c.args.get("deleteData") : c.args?.deleteData),
  )).toContain(false);
});

test("deleting project data requires a second confirmation and five-second countdown", async ({ page }) => {
  await page.clock.install();
  await page.goto("/");
  const deleteButton = page.locator(".proj-card:not(.proj-example) .pc-del").first();
  await deleteButton.click();
  await page.getByRole("button", { name: "Delete project and local data", exact: true }).click();

  const destructiveModal = page.locator(".project-delete-data-modal");
  await expect(destructiveModal).toBeVisible();
  await expect(destructiveModal).toContainText("This cannot be undone");
  const permanentDelete = destructiveModal.getByRole("button", { name: /Permanently delete/ });
  expect(await permanentDelete.textContent()).toBe("Permanently delete (5s)");
  await expect(permanentDelete).toBeDisabled();

  // One Escape closes only the topmost confirmation and returns to the choice
  // dialog. A second press closes that parent dialog.
  await page.keyboard.press("Escape");
  await expect(destructiveModal).toHaveCount(0);
  await expect(page.locator(".project-delete-choice-modal")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".project-delete-choice-modal")).toHaveCount(0);

  await deleteButton.click();
  await page.getByRole("button", { name: "Delete project and local data", exact: true }).click();
  const confirmedDelete = page.locator(".project-delete-data-modal")
    .getByRole("button", { name: /Permanently delete/ });
  await page.clock.fastForward(4_900);
  await expect(confirmedDelete).toBeDisabled();
  await page.clock.fastForward(200);
  await expect(confirmedDelete).toBeEnabled();
  await expect(confirmedDelete).toHaveText("Permanently delete");
  await confirmedDelete.click();

  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? [])
      .filter((c: any) => c.cmd === "delete_project")
      .map((c: any) => c.args instanceof Map ? c.args.get("deleteData") : c.args?.deleteData),
  )).toContain(true);
});

test("external links open in the system browser, not the app webview (#97)", async ({ page }) => {
  // A reference link in rendered markdown used to navigate the whole webview
  // away from the UI with no way back. Any external <a> must now be intercepted
  // and handed to the system browser instead.
  await enterApp(page);
  await page.evaluate(() => {
    const a = document.createElement("a");
    a.id = "ext-link-probe";
    a.href = "https://example.com/paper.pdf";
    a.textContent = "open paper";
    document.body.appendChild(a);
  });
  await page.click("#ext-link-probe");
  await page.getByTestId("external-link-open").click();
  // serde_wasm_bindgen passes args as a JS Map, not a plain object.
  await expect.poll(() => openedExternalUrls(page)).toContain("https://example.com/paper.pdf");
  // The app itself must still be on screen — the click was intercepted, not
  // followed as a top-level navigation.
  await expect(newSessionButton(page)).toBeVisible();
});

test("relative paths and hash anchors never navigate the app", async ({ page }) => {
  await enterApp(page);
  const appUrl = page.url();
  await page.evaluate(() => {
    const rel = document.createElement("a");
    rel.id = "rel-link-probe";
    rel.href = "notes/FIGURE_LEGEND.md";
    rel.textContent = "relative";
    const hash = document.createElement("a");
    hash.id = "hash-link-probe";
    hash.href = "#section";
    hash.textContent = "anchor";
    document.body.appendChild(rel);
    document.body.appendChild(hash);
  });
  await page.click("#rel-link-probe");
  expect(page.url()).toBe(appUrl);
  await expect(newSessionButton(page)).toBeVisible();
  await page.click("#hash-link-probe");
  expect(page.url()).toBe(appUrl);
  await expect(newSessionButton(page)).toBeVisible();
  expect(await openedExternalUrls(page)).toEqual([]);
});

async function openedExternalUrls(page: Page) {
  return page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? [])
      .filter((c: any) => c.cmd === "open_external_url")
      .map((c: any) => (c.args instanceof Map ? c.args.get("url") : c.args?.url)),
  );
}

test("a bare URL in a reply is clickable and opens only after confirmation", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("MDURL");
  await page.getByRole("button", { name: "Send" }).click();

  // The model typed the URL as prose, with no markdown link syntax and CJK
  // punctuation right after it — it still has to be a link, and only the URL.
  const link = page.locator(".msg.assistant .body.md a", { hasText: "https://www.baidu.com" });
  await expect(link).toBeVisible({ timeout: 10_000 });
  await expect(link).toHaveText("https://www.baidu.com");
  await expect(link).toHaveAttribute("href", "https://www.baidu.com");

  await link.click();
  const confirm = page.getByTestId("external-link-confirm");
  await expect(confirm).toBeVisible();
  await expect(page.getByTestId("external-link-url")).toHaveText("https://www.baidu.com");
  expect(await openedExternalUrls(page)).toEqual([]);

  await page.getByTestId("external-link-open").click();
  await expect(confirm).toHaveCount(0);
  await expect.poll(() => openedExternalUrls(page)).toContain("https://www.baidu.com");
  await expect(newSessionButton(page)).toBeVisible();
});

test("Escape dismisses the link confirmation without opening the browser", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("MDURL");
  await page.getByRole("button", { name: "Send" }).click();
  const link = page.locator(".msg.assistant .body.md a", { hasText: "https://www.baidu.com" });
  await expect(link).toBeVisible({ timeout: 10_000 });

  await link.click();
  const confirm = page.getByTestId("external-link-confirm");
  await expect(confirm).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(confirm).toHaveCount(0);
  expect(await openedExternalUrls(page)).toEqual([]);
  // Only the confirmation closed: the transcript is untouched.
  await expect(link).toBeVisible();
});

test("assistant markdown uses normal whitespace (no phantom blank lines)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("MDLIST");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("FX细胞")).toBeVisible({ timeout: 10_000 });
  const whiteSpace = await page.locator(".msg.assistant .body.md").first().evaluate(
    (el) => getComputedStyle(el).whiteSpace,
  );
  expect(whiteSpace).toBe("normal");
});

test("completed commentary, reasoning, and tools fold into one activity summary", async ({ page }) => {
  const browserErrors: string[] = [];
  page.on("pageerror", (error) => browserErrors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") browserErrors.push(message.text());
  });
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });
  await expect(page.locator(".msg.assistant")).toHaveCount(1);
  await expect(page.locator(".msg.assistant.commentary")).toHaveCount(0);

  const activity = page.locator(".steps.activity-summary");
  await expect(activity).toHaveCount(1);
  expect(browserErrors).toEqual([]);
  await expect(activity).not.toHaveClass(/open/);
  // A collapsed summary owns no hidden transcript subtree; rows and their
  // potentially large tool bodies mount only after the corresponding toggle.
  await expect(activity.locator(".steps-body")).toHaveCount(0);
  await expect(activity.locator(".step")).toHaveCount(0);
  await expect(page.locator(".step-body:visible")).toHaveCount(0);
  const activityHead = activity.getByRole("button", { name: /Processed/ });
  await expect(activityHead).toHaveAttribute("aria-expanded", "false");
  await activityHead.focus();
  await page.keyboard.press("Enter");
  await expect(activityHead).toHaveAttribute("aria-expanded", "true");
  await expect(activity.locator(".step-progress")).toHaveCount(3);
  await expect(activity.locator(".step-think")).toHaveCount(2);
  await expect(activity.locator(".step-name")).toContainText([
    "progress", "thinking", "shell", "progress", "thinking", "python", "progress", "write",
  ]);
  await expect(activity.locator(".step-body")).toHaveCount(0);
  const shell = activity.locator(".step", { hasText: "shell" });
  await shell.locator(".step-head").click();
  await expect(shell.locator(".step-body")).toHaveCount(1);
  await expect(shell.locator(".tool-output")).toContainText("gene_0");
  await activityHead.focus();
  await page.keyboard.press("Space");
  await expect(activityHead).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator("details.rz")).toHaveCount(0);
  expect(browserErrors).toEqual([]);
});

test("live step disclosure choices survive tool updates and completion (#172)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSLIVE");
  await page.getByRole("button", { name: "Send" }).click();

  const steps = page.locator(".steps");
  const shell = page.locator(".steps .step", { hasText: "shell" }).first();
  await expect(steps).toHaveClass(/open/, { timeout: 2_000 });
  await expect(shell).toHaveClass(/open/);

  // Record explicit user choices rather than relying on the automatic live
  // defaults. Each following event changes the row fingerprint and remounts
  // its rendered content.
  await page.locator(".steps-head").click();
  await expect(steps).not.toHaveClass(/open/);
  await page.locator(".steps-head").click();
  await expect(steps).toHaveClass(/open/);
  await shell.locator(".step-head").click();
  await expect(shell).not.toHaveClass(/open/);
  await shell.locator(".step-head").click();
  await expect(shell).toHaveClass(/open/);

  await expect(shell.locator(".tool-output")).toContainText("shell output line", { timeout: 4_000 });
  await expect(steps).toHaveClass(/open/);
  await expect(shell).toHaveClass(/open/);

  await expect(page.getByText("Live steps finished.")).toBeVisible({ timeout: 4_000 });
  // Completion replaces the live disclosure with a fresh, collapsed summary.
  await expect(steps).toHaveClass(/activity-summary/);
  await expect(steps).not.toHaveClass(/open/);
  await expect(shell).toHaveCount(0);
});

test("provenance rows and collapsed bodies stay isolated while assistant text streams", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });

  await page.getByRole("button", { name: "Toggle panel" }).click();
  const panel = page.locator(".rightpane");
  await panel.getByRole("button", { name: "Add panel" }).click();
  await panel.locator(".rp-tab-add-menu").getByRole("button", { name: /^Provenance/ }).click();

  const first = panel.locator(".prov-item").first();
  await expect(first).toBeVisible();
  await expect(first.locator(".prov-body")).toHaveCount(0);
  await first.locator(".prov-head").click();
  await expect(first.locator(".prov-body").last()).toContainText("gene_0");
  await first.evaluate((element) => ((element as any).__streamStableProbe = true));

  await composer(page).fill("SCROLLTEST");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("line 40", { exact: false })).toBeVisible({ timeout: 10_000 });
  expect(await first.evaluate((element) => (element as any).__streamStableProbe === true)).toBe(true);
  await expect(first).toHaveAttribute("open", "open");
});

test("completed ACP commentary, reasoning, and tools share one summary", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACPTHINK");
  await page.getByRole("button", { name: "Send" }).click();

  const activity = page.locator(".steps.activity-summary");
  await expect(activity).toHaveCount(1, { timeout: 4_000 });
  await expect(activity).not.toHaveClass(/open/);
  await activity.locator(".steps-head").click();
  await expect(activity.getByText("web_search")).toBeVisible();
  await expect(activity.locator(".step-progress")).toHaveCount(1);
  await expect(activity.locator(".step-think")).toHaveCount(1);

  // Preserve the wire order inside the completed activity disclosure.
  const progressY = await activity.locator(".step-progress").evaluate((el) => el.getBoundingClientRect().top);
  const reasoningY = await activity.locator(".step-think").evaluate((el) => el.getBoundingClientRect().top);
  const toolY = await activity.locator(".acp-tool").evaluate((el) => el.getBoundingClientRect().top);
  expect(progressY).toBeLessThan(reasoningY);
  expect(reasoningY).toBeLessThan(toolY);
});

test("Agents panel is activity-only and opens the standalone Workflow Studio", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator(".rightpane").getByRole("button", { name: "Agents", exact: true }).click();
  const panel = page.getByTestId("agent-workflows");

  await expect(panel).toContainText("Agent workflow activity");
  await expect(panel).toContainText("Create and edit reusable workflows in Workflow Studio");
  await expect(panel).toContainText("Delegation is off for this conversation");
  await expect(panel.getByTestId("dynamic-agent-editor")).toHaveCount(0);
  await expect(panel.getByTestId("agent-create")).toHaveCount(0);
  await panel.getByTestId("agent-open-workflows").click();

  await expect(page.locator(".settings-page")).toHaveClass(/workflow-studio-mode/);
  await expect(page.getByTestId("workflow-studio")).toBeVisible();
});

test("main-Agent dynamic batches show parallel roots and pending dependencies", async ({ page }) => {
  await enterApp(page, "/?mockAgentWorkflow=parallel&mockOtherAgentWorkflow=succeeded");
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator(".rightpane").getByRole("button", { name: "Agents", exact: true }).click();
  const panel = page.getByTestId("agent-workflows");
  const card = panel.locator(".agent-workflow-card.dynamic").first();

  await expect.poll(() => lastInvokeArgs(page, "list_agent_workflows"))
    .toMatchObject({ sessionId: "s-current" });
  await expect(panel.locator(".agent-workflow-card.dynamic")).toHaveCount(1);
  await expect(panel).not.toContainText("Completed dynamic research");
  await expect(card).toContainText("Main Agent parallel research batch");
  const researchA = card.locator('[data-step-id$=":research_a"]');
  await expect(researchA.locator(".agent-attempt-status")).toHaveText("Running");
  await expect(researchA).toContainText("Temporary Agent · native · default");
  await expect(researchA.locator(".agent-chip.capability")).toHaveText("project_read");
  await expect(card.locator('[data-step-id$=":research_b"] .agent-attempt-status')).toHaveText("Running");
  const synthesis = card.locator('[data-step-id$=":synthesize"]');
  await expect(synthesis.locator(".agent-attempt-status")).toHaveText("Pending");
  await expect(synthesis.locator(".agent-chip.dependency")).toHaveText(["research_a", "research_b"]);
  await expect(panel.locator(".agent-workflow-group-head")).toContainText("Conversation");
});

test("nested Agent workflows render under their root without independent controls", async ({ page }) => {
  await enterApp(page, "/?mockAgentWorkflow=nested");
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator(".rightpane").getByRole("button", { name: "Agents", exact: true }).click();
  const panel = page.getByTestId("agent-workflows");
  const groups = panel.locator(".agent-workflow-group");
  await expect(groups).toHaveCount(1);
  const root = panel.locator('.agent-workflow-card.dynamic[data-depth="0"]');
  const nested = panel.locator('.agent-workflow-card.dynamic.nested[data-depth="1"]');
  await expect(root).toContainText("Root delegation batch");
  await expect(nested).toContainText("Nested evidence batch");
  await expect(nested).toContainText("Nested · depth 2");
  await expect(nested.locator('[data-step-id$="parent/leaf"]')).toBeVisible();
  await expect(nested.getByTestId("agent-retry")).toHaveCount(0);
  await expect(nested).not.toContainText("Delegation is off for this workflow");
});

test("failed dynamic tasks and dependency-blocked tasks stay distinct", async ({ page }) => {
  await enterApp(page, "/?mockAgentWorkflow=partial");
  await enableDelegation(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator(".rightpane").getByRole("button", { name: "Agents", exact: true }).click();
  const card = page.getByTestId("agent-workflows").locator(".agent-workflow-card.dynamic").first();

  await expect(card.locator(".agent-workflow-status")).toHaveText("Failed");
  await expect(card.locator('[data-step-id$=":research_a"] .agent-attempt-status')).toHaveText("Failed");
  await expect(card.locator('[data-step-id$=":research_b"] .agent-attempt-status')).toHaveText("Succeeded");
  await expect(card.locator('[data-step-id$=":synthesize"] .agent-attempt-status')).toHaveText("Blocked");
  await expect(card.locator('[data-step-id$=":synthesize"]')).toContainText("Blocked by a failed dependency");

  const retryBudget = card.locator('[data-step-id$=":research_a"]')
    .getByTestId("agent-retry-max-tokens");
  await expect(retryBudget).toHaveValue("8000");
  await retryBudget.fill("16000");
  const workflowId = await card.getAttribute("data-workflow-id");
  await card.getByTestId("agent-retry").click();
  await expect.poll(() => lastInvokeArgs(page, "retry_agent_workflow")).toMatchObject({
    workflowId,
    budgetOverrides: { research_a: { max_tokens: 16000 } },
  });
  await expect(card.locator(".agent-workflow-status")).toHaveText("Approved");
  await expect(card.locator('[data-step-id$=":research_b"] .agent-attempt-status')).toHaveText("Succeeded");
  await card.getByTestId("agent-run").click();
  await expect(card.locator(".agent-workflow-status")).toHaveText("Succeeded", { timeout: 2_000 });
  await expect(card.locator('[data-step-id$=":research_b"] .agent-attempt-status')).toHaveText("Succeeded");
});

test("task results are readable without exposing the child Agent conversation", async ({ page }) => {
  await enterApp(page, "/?mockAgentWorkflow=succeeded");
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator(".rightpane").getByRole("button", { name: "Agents", exact: true }).click();
  const panel = page.getByTestId("agent-workflows");
  const card = panel.locator(".agent-workflow-card.dynamic").first();
  const synthesis = card.locator('[data-step-id$=":synthesize"]');
  await expect(synthesis).toContainText("1140 tokens · 3 tools");

  await synthesis.getByTestId("agent-inspect-result").click();
  const dialog = page.getByRole("dialog", { name: "Task result" });
  await expect(dialog).toHaveClass(/artifact-modal/);
  await expect(dialog.getByTestId("agent-result-summary")).toContainText("Completed synthesize");
  await expect(dialog.getByTestId("agent-result-artifacts")).toContainText("Readable result content for synthesize");
  await expect(dialog.getByTestId("agent-result-evidence")).toContainText("evidence-for-synthesize");
  await expect(dialog.getByTestId("agent-result-tests")).toContainText("Structure check passed");
  await expect(dialog.getByTestId("agent-result-risks")).toContainText("Mock evidence only");
  await expect(dialog).not.toContainText("agent-child-synthesize");
  await expect(dialog.getByTestId("agent-result-json")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("agent-workflows")).toBeVisible();
  await expect(page.locator(".rightpane")).toBeVisible();
  await expect(card.getByRole("button", { name: "Take over" })).toHaveCount(0);
});

test("disabled delegation preserves running history and blocks new starts and retries", async ({ page }) => {
  await enterApp(page, "/?mockAgentWorkflow=parallel");
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator(".rightpane").getByRole("button", { name: "Agents", exact: true }).click();
  const panel = page.getByTestId("agent-workflows");
  const card = panel.locator(".agent-workflow-card.dynamic").first();

  await expect(panel).toContainText("Delegation is off for this conversation");
  await expect(card).toContainText("Main Agent parallel research batch");
  await expect(panel.getByTestId("agent-create")).toHaveCount(0);
  await expect(card.getByTestId("agent-edit")).toHaveCount(0);
  await expect(card.getByTestId("agent-cancel")).toBeEnabled();
  await card.getByTestId("agent-cancel").click();
  await expect(card.locator(".agent-workflow-status")).toHaveText("Cancelled");
  await expect(card.getByTestId("agent-retry")).toBeDisabled();
});

test("code lives in Notebook instead of Artifacts", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });

  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Notebook (2)", exact: true }).click();

  const cells = page.locator(".notebook-cell");
  await expect(cells).toHaveCount(2);
  await expect(cells.nth(0).locator(".notebook-language")).toHaveText("bash");
  await expect(cells.nth(1).locator(".notebook-language")).toHaveText("python");
  await expect(cells.nth(1)).toContainText("import pandas as pd");
  await cells.nth(1).locator(".notebook-output summary").click();
  await expect(cells.nth(1).locator(".notebook-output pre")).toContainText("col_0: ok");

  await page.getByRole("button", { name: "Artifacts", exact: true }).click();
  await expect(page.locator(".rp-badge.code")).toHaveCount(0);
});

test("R tool calls project into a highlighted Notebook cell", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("RNOTEBOOK");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("R summary complete.")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Notebook (1)", exact: true }).click();

  const cell = page.locator(".notebook-cell");
  await expect(cell.locator(".notebook-language")).toHaveText("r");
  await expect(cell.locator("code.language-r")).toContainText("summary(dataset)");
  await expect(cell.locator("code.language-r")).not.toContainText("ssh:gpu-server");
  await cell.locator(".notebook-output summary").click();
  await expect(cell.locator(".notebook-output pre")).toContainText("Length Class Mode");
});

test("an SVG star saves a Notebook cell in the global library", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Notebook (2)", exact: true }).click();

  const cell = page.locator(".notebook-cell").first();
  const star = cell.getByRole("button", { name: "Add to library" });
  await expect(star.locator("svg path")).toHaveCount(1);
  await expect(star).toHaveText("");
  const copy = cell.getByRole("button", { name: "Copy code" });
  await expect.poll(() => copy.evaluate((node) =>
    node.previousElementSibling?.classList.contains("notebook-star") ?? false,
  )).toBe(true);
  await star.click();
  await expect(cell.getByRole("button", { name: "Remove from library" })).toHaveAttribute("aria-pressed", "true");

  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(page.getByTestId("library-screen")).toBeVisible();
  await expect(page.locator('.library-card[data-library-kind="code"]')).toContainText("zcat counts.txt.gz");
  await expect(page.locator('.library-card[data-library-kind="code"]')).toContainText("wisp-science / Current analysis");

  // Library search runs against SQLite so full saved source remains searchable
  // even though the global list only carries bounded previews.
  await page.getByRole("searchbox", { name: "Search library" }).fill("counts.txt.gz");
  await expect.poll(() => lastInvokeArgs(page, "search_library_items")).toMatchObject({
    query: "counts.txt.gz",
  });
  await expect(page.locator('.library-card[data-library-kind="code"]')).toContainText("zcat counts.txt.gz");
});

test("the command palette opens the global library", async ({ page }) => {
  await enterApp(page);
  await page.keyboard.press("Control+p");
  const input = page.locator("#action-palette-input");
  await input.fill("library");
  await expect(page.locator(".action-palette-row").first()).toContainText("Open library");
  await input.press("Enter");
  await expect(page.getByTestId("library-screen")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("library-screen")).toHaveCount(0);
  await expect(newSessionButton(page)).toBeVisible();
});

test("a starred figure keeps its image and generating code", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();

  const modal = page.locator(".artifact-modal");
  const star = modal.getByRole("button", { name: "Add to library" });
  await expect(star.locator("svg path")).toHaveCount(1);
  const openCenter = modal.getByRole("button", { name: "Open in center" });
  await expect.poll(() => openCenter.evaluate((node) =>
    node.previousElementSibling?.getAttribute("aria-label"),
  )).toBe("Add to library");
  await star.click();
  await expect(modal.getByRole("button", { name: "Remove from library" })).toHaveAttribute("aria-pressed", "true");
  await modal.getByRole("button", { name: "Close panel" }).click();

  await page.getByRole("button", { name: "Library", exact: true }).click();
  const figure = page.locator('.library-card[data-library-kind="figure"]');
  await expect(figure).toContainText("volcano.png");
  await figure.locator(".library-card-main").click();
  const detail = page.locator(".library-detail");
  await expect(detail.locator(".library-figure img")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(detail).toHaveCount(0);
  await expect(page.getByTestId("library-screen")).toBeVisible();

  await figure.locator(".library-card-main").click();
  await expect(detail.locator(".library-figure img")).toBeVisible();
  await expect(detail).toContainText("Generating code");
  await expect(detail).toContainText("savefig");

  await detail.getByRole("button", { name: "Remove from library" }).click();
  await expect(page.locator('.library-card[data-library-kind="figure"]')).toHaveCount(0);
});

test("a starred code item edits into a new version and re-runs from the composer (#474)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Notebook (2)", exact: true }).click();
  await page.locator(".notebook-cell").first().getByRole("button", { name: "Add to library" }).click();

  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.locator('.library-card[data-library-kind="code"] .library-card-main').click();
  const detail = page.locator(".library-detail");
  await expect(detail.locator(".library-code-head h3")).toHaveText("v1");

  await detail.getByRole("button", { name: "Edit code" }).click();
  await detail.locator(".library-edit-area").fill("zcat counts.txt.gz | head -20");
  await detail.getByRole("button", { name: "Save as new version" }).click();

  // The edit appends v2; v1 keeps the starred snapshot verbatim.
  await expect(detail.locator(".library-code-head h3")).toHaveText("v2");
  await expect(detail.locator(".rp-code")).toContainText("head -20");
  await detail.locator(".library-versions button", { hasText: "Original" }).click();
  await expect(detail.locator(".library-code-head h3")).toHaveText("v1");
  await expect(detail.locator(".rp-code")).toContainText("zcat counts.txt.gz");
  await expect(detail.locator(".rp-code")).not.toContainText("head -20");

  // "Insert into chat" pre-fills the composer with the chosen version — the
  // request carries the item id + version number and is never auto-sent.
  await detail.locator(".library-versions button", { hasText: "v2" }).click();
  await detail.getByRole("button", { name: "Insert into chat" }).click();
  await expect(page.getByTestId("library-screen")).toHaveCount(0);
  await expect(composer(page)).toHaveValue(/v2/);
  await expect(composer(page)).toHaveValue(/library item library-1/);
  await expect(composer(page)).toHaveValue(/head -20/);
});

test("a starred figure's generating code is editable as a new version (#474)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  const modal = page.locator(".artifact-modal");
  await modal.getByRole("button", { name: "Add to library" }).click();
  await modal.getByRole("button", { name: "Close panel" }).click();

  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.locator('.library-card[data-library-kind="figure"] .library-card-main').click();
  const code = page.locator(".library-generating-code");
  await code.getByRole("button", { name: "Edit code" }).click();
  await code.locator(".library-edit-area").fill("plt.savefig('volcano.svg')");
  await code.getByRole("button", { name: "Save as new version" }).click();

  await expect(code.locator(".library-code-head h3")).toHaveText("v2");
  await expect(code.locator(".rp-code")).toContainText("volcano.svg");
  // The starred image itself is untouched by a code edit.
  await expect(page.locator(".library-detail .library-figure img")).toBeVisible();
  await code.locator(".library-versions button", { hasText: "Original" }).click();
  await expect(code.locator(".rp-code")).toContainText("import matplotlib");
});

test("the artifact modal edits provenance code into a composer re-run (#474)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  const modal = page.locator(".artifact-modal");
  await page.locator(".am-tab", { hasText: "Code" }).click();
  await modal.getByRole("button", { name: "Edit code and re-run" }).click();
  await modal.locator(".am-edit-area").fill("plt.savefig('volcano.png', dpi=300)");
  await modal.getByRole("button", { name: "Insert into chat" }).click();

  await expect(page.locator(".artifact-modal")).toHaveCount(0);
  await expect(composer(page)).toHaveValue(/dpi=300/);
  await expect(composer(page)).toHaveValue(/volcano\.png/);
});

test("the selection popup saves a highlight into the right pane and library", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });

  // Select a text run inside the assistant reply, as a reader would.
  const selected = await page.evaluate(() => {
    const body = document.querySelector(".msg.assistant .body");
    if (!body) return "";
    const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT);
    let node: Text | null = null;
    while (walker.nextNode()) {
      const candidate = walker.currentNode as Text;
      if (candidate.data.trim().length > 20) { node = candidate; break; }
    }
    if (!node) return "";
    const range = document.createRange();
    range.selectNodeContents(node);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
    return node.data;
  });
  expect(selected.trim().length).toBeGreaterThan(20);

  // Releasing the mouse over the transcript raises the selection popup — the
  // same surface that offers "Add to chat" / "Explain", now with the highlight.
  await page.locator(".msg.assistant .body").first().dispatchEvent("mouseup");
  await expect(page.locator(".selection-popup")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".selection-popup")).toHaveCount(0);

  await page.locator(".msg.assistant .body").first().dispatchEvent("mouseup");
  await page.getByRole("button", { name: "Save highlight" }).click();

  // The Highlights tab opens with the excerpt, and the transcript is underlined.
  await expect(page.getByRole("button", { name: "Highlights (1)", exact: true })).toBeVisible();
  await expect(page.locator(".highlight-card .highlight-text")).toContainText(selected.trim().slice(0, 30));
  await expect.poll(() => page.evaluate(() => (CSS as any).highlights?.has("wisp-saved") ?? false)).toBe(true);
  // Let the double-rAF underline pass finish before the next turn starts.
  await page.evaluate(() => new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));

  // Saved-mark application is revision-based: token batches in a later turn
  // must not rebuild the transcript text index once per flush.
  await composer(page).fill("MARKDOWNSTREAM");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("stream line 4", { exact: false })).toBeVisible({ timeout: 10_000 });
  await page.evaluate(() => new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
  await page.evaluate(() => {
    const registry = (CSS as any).highlights;
    const set = registry.set.bind(registry);
    (window as any).__savedMarkSetCalls = 0;
    registry.set = (name: string, value: unknown) => {
      if (name === "wisp-saved") (window as any).__savedMarkSetCalls += 1;
      return set(name, value);
    };
  });
  await expect(page.getByText("stream line 18", { exact: false })).toBeVisible({ timeout: 10_000 });
  expect(await page.evaluate(() => (window as any).__savedMarkSetCalls)).toBe(0);
  await expect(page.getByText("stream line 23", { exact: false })).toBeVisible({ timeout: 10_000 });

  // The global library lists it under the Highlights filter.
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(page.getByTestId("library-screen")).toBeVisible();
  await page.locator(".library-filters button", { hasText: "Highlights" }).click();
  await expect(page.locator('.library-card[data-library-kind="text"]')).toContainText(selected.trim().slice(0, 30));
});

test("a project card can open its project in a new window (#52)", async ({ page }) => {
  await page.goto("/");
  await page.locator(".proj-card:not(.proj-example) .pc-window").first().click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? [])
      .filter((c: any) => c.cmd === "open_project_window")
      .map((c: any) => (c.args instanceof Map ? c.args.get("id") : c.args?.id)),
  )).toContain("default");
});

test("a ?project window opens straight into the project, skipping the landing (#52)", async ({ page }) => {
  // A dedicated project window carries ?project=<id>; it must open that project
  // directly (per-window active) instead of showing the projects landing.
  await page.goto("/?project=default");
  await expect(newSessionButton(page)).toBeVisible({ timeout: 10_000 });
  // The landing (project cards) must NOT be shown in a dedicated project window.
  await expect(page.locator(".proj-card")).toHaveCount(0);
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((c: any) => c.cmd === "open_project"),
  )).toBe(true);
  await expect(page).toHaveTitle("wisp science \u2014 wisp-science");
});

test("specialists page configures the builtin Reader and saves a custom specialist", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Specialists");
  await expect(page.getByText("Reviewer")).toBeVisible();
  await expect(page.getByText("Reader")).toBeVisible();
  await expect(page.getByText("Scientific Illustrator")).toBeVisible();
  // Builtin rows have no remove button.
  await expect(page.locator(".settings-list-remove")).toHaveCount(0);

  await page.getByText("Reader").click();
  await expect(page.getByLabel("Instructions")).toBeDisabled();
  await page.getByTestId("reviewer-backend-select").selectOption("opus");
  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_specialist_cmd")).toMatchObject({
    spec: { id: "reader", model_id: "opus" },
  });

  // builtin row: open it and verify instructions are disabled
  await page.getByText("Reviewer").click();
  await expect(page.getByLabel("Instructions")).toBeDisabled();
  await page.locator(".settings-head-back").click();

  await page.getByText("Add specialist").click();
  const fromScratch = page.getByText("Write from scratch");
  await expect(fromScratch).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(fromScratch).not.toBeVisible();
  await expect(page.locator(".settings-page")).toBeVisible();

  await page.getByText("Add specialist").click();
  await fromScratch.click();
  await page.getByLabel("Name").fill("Paper hunter");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Paper hunter")).toBeVisible();
});

test("specialist skills whitelist uses a searchable picker instead of a full list", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Specialists");
  await page.getByText("Scientific Illustrator").click();

  // Existing whitelist entries show as removable chips, even when the skill is
  // not present in the local skill list.
  const selected = page.getByTestId("specialist-selected-skill");
  await expect(selected).toHaveCount(2);
  await expect(selected.filter({ hasText: "figure-composer" })).toHaveCount(1);

  // No skills render until a search query narrows the list.
  const options = page.getByTestId("specialist-skill-option");
  await expect(options).toHaveCount(0);
  const search = page.getByTestId("specialist-skill-search");
  await expect(page.getByTestId("specialist-skill-results")).toContainText("4 available skills");

  await search.fill("narrative");
  await expect(options).toHaveCount(1);
  await options.first().click();
  await expect(selected).toHaveCount(3);

  // Unchecking through the picker removes the chip again.
  await options.first().click();
  await expect(selected).toHaveCount(2);

  // Chips remove entries directly.
  await selected.filter({ hasText: "figure-style" }).click();
  await expect(selected).toHaveCount(1);

  await search.fill("does-not-exist");
  await expect(options).toHaveCount(0);
  await expect(page.getByTestId("specialist-skill-results")).toContainText("No matching skills.");
});

test("Reviewer settings select, test, and persist an ACP backend", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Specialists");
  await page.getByText("Reviewer").click();

  const backend = page.getByTestId("reviewer-backend-select");
  await expect(backend.locator('option[value="acp:acp-test"]')).toHaveCount(1);
  await backend.selectOption("acp:acp-test");
  await expect(page.getByTestId("reviewer-selected-backend")).toContainText("Test ACP Agent");
  await expect(backend).toHaveValue("acp:acp-test");
  await expect(page.getByTestId("reviewer-selected-backend")).toContainText("ACP");

  await page.getByTestId("test-reviewer-backend").click();
  await expect.poll(() => lastInvokeArgs(page, "test_reviewer_backend")).toMatchObject({
    reviewer: {
      id: "reviewer",
      review_backend: { kind: "acp_agent", profile_id: "acp-test" },
    },
  });
  await expect(page.locator(".settings-status")).toContainText(
    "valid review JSON via ACP / Test ACP Agent",
  );

  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_specialist_cmd")).toMatchObject({
    spec: {
      id: "reviewer",
      review_backend: { kind: "acp_agent", profile_id: "acp-test" },
    },
  });
  await expect(page.locator(".settings-status")).toContainText("Specialist saved");
  await expect(backend).toHaveValue("acp:acp-test");

  await page.locator(".settings-head-back").click();
  await page.getByText("Reviewer").click();
  await expect(page.getByTestId("reviewer-backend-select")).toHaveValue("acp:acp-test");
});

test("a deleted ACP reviewer remains visibly selected as missing", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Specialists");
  await page.getByText("Reviewer").click();
  await page.getByTestId("reviewer-backend-select").selectOption("acp:acp-test");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.locator(".settings-status")).toContainText("Specialist saved");

  const nav = page.locator(".settings-nav");
  await nav.getByRole("button", { name: "Models", exact: true }).click();
  await page.getByTestId("open-acp-agents-from-settings").click();
  const row = page.getByTestId("acp-agent-row").filter({ hasText: "Test ACP Agent" });
  await row.locator(".settings-list-remove").click();
  await page
    .getByTestId("model-delete-confirm")
    .getByRole("button", { name: "Remove model" })
    .click();
  await expect(row).toHaveCount(0);

  await nav.getByRole("button", { name: "Specialists", exact: true }).click();
  await page.getByText("Reviewer").click();
  const backend = page.getByTestId("reviewer-backend-select");
  await expect(backend).toHaveValue("acp:acp-test");
  await expect(page.getByTestId("reviewer-missing-acp-option")).toHaveText(
    "Missing ACP Agent · acp-test",
  );
  await expect(page.getByTestId("reviewer-selected-backend")).toContainText(
    "Missing ACP Agent · acp-test",
  );

  await page.getByTestId("test-reviewer-backend").click();
  await expect(page.locator(".settings-status")).toContainText(
    "Reviewer backend test failed: The Reviewer ACP Agent profile no longer exists.",
  );
});

test("new session can pick a specialist and it locks after the first message", async ({ page }) => {
  await enterApp(page);
  // Create the custom specialist through the settings flow, as above.
  await openSettingsSection(page, "Specialists");
  await page.getByText("Add specialist").click();
  await page.getByText("Write from scratch").click();
  await page.getByLabel("Name").fill("Paper hunter");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Paper hunter")).toBeVisible();
  await page.locator(".settings-head-close").click();

  // Picking a specialist requires an active session (set lazily on first send
  // otherwise), so start one explicitly via "New session".
  await newSessionButton(page).click();
  let agentMenu = await openAgentMenu(page);
  await agentMenu.getByRole("button", { name: /^Specialist/ }).click();
  const specialistMenu = page.getByRole("menu", { name: "Specialist" });
  await expect(specialistMenu.getByRole("button", { name: "Scientific Illustrator" })).toBeVisible();
  await specialistMenu.getByRole("button", { name: "Paper hunter" }).click();
  await expect(page.locator(".session-specialist")).toHaveText("Paper hunter");

  await composer(page).fill("hello there");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  agentMenu = await openAgentMenu(page);
  await expect(agentMenu.getByRole("button", { name: /^Specialist/ })).toBeDisabled();
});

test("chat-with-claude creation opens a new session with the interview prompt", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Specialists");
  await page.getByText("Add specialist").click();
  await page.getByText("Chat with Claude").click();
  // settings closed, a session is active, and send_message was invoked with the template
  await expect(page.locator(".settings-page")).toHaveCount(0);
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "send_message").length,
  )).toBeGreaterThan(0);
});

test("remote access settings: Feishu, WeChat, and StickS3 setup", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Remote Access");

  // List page: routing note plus one row per bot, toggles disabled until bound.
  await expect(page.getByTestId("channel-routing-help")).toBeVisible();
  await expect(page.getByTestId("channel-routing-help").getByText("/project", { exact: true })).toBeVisible();
  await expect(page.getByTestId("channel-routing-help").getByText("/session", { exact: true })).toBeVisible();
  await expect(page.getByTestId("feishu-channel-row")).toBeVisible();
  await expect(page.getByTestId("weixin-channel-row")).toBeVisible();
  await expect(page.getByTestId("sticks3-channel-row")).toBeVisible();
  await expect(page.getByTestId("feishu-enabled")).toBeDisabled();
  await expect(page.getByTestId("weixin-enabled")).toBeDisabled();
  await expect(page.getByTestId("sticks3-enabled")).toBeDisabled();

  // Feishu subpage: existing applications still have a manual, keyring-backed
  // setup path.
  await page.getByTestId("feishu-channel-row").click();
  await expect(page.getByTestId("feishu-channel-card")).toBeVisible();
  await expect(page.getByTestId("feishu-pending-owner")).toBeVisible();
  await expect(page.getByTestId("feishu-pending-owner")).toContainText("ou_pending");
  await page.getByTestId("feishu-pending-reject").click();
  await expect.poll(() => lastInvokeArgs(page, "reject_feishu_pending_owner")).not.toBeNull();
  await expect(page.getByTestId("feishu-pending-owner")).toHaveCount(0);
  await expect(page.getByTestId("feishu-owner-status")).toContainText("No owner bound");
  await page.getByTestId("feishu-international").check();
  await page.getByTestId("feishu-app-id").fill("cli_test123");
  await page.getByTestId("feishu-app-secret").fill("secret-xyz");
  await page.getByTestId("feishu-save").click();
  await expect.poll(() => lastInvokeArgs(page, "set_feishu_channel")).toMatchObject({
    enabled: false,
    international: true,
    appId: "cli_test123",
    appSecret: "secret-xyz",
  });

  // Removing local credentials does not claim to delete the remote app. The
  // one-click path then shows a real QR lifecycle and stores credentials in
  // the backend without exposing the secret to the webview.
  await page.getByTestId("feishu-unbind").click();
  await expect(page.getByTestId("feishu-bind")).toBeVisible();
  await page.getByTestId("feishu-bind").click();
  await expect(page.getByTestId("feishu-qr")).toBeVisible();
  await expect(page.getByTestId("feishu-unbind")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId("feishu-app-id")).toHaveValue("cli_scan_created");
  await expect(page.getByTestId("feishu-owner-status")).toContainText("ou_scan_owner");
  await page.getByTestId("feishu-owner-id").fill("ou_manual");
  await page.getByTestId("feishu-owner-save").click();
  await expect.poll(() => lastInvokeArgs(page, "set_feishu_owner")).toMatchObject({
    openId: "ou_manual",
  });
  await expect(page.getByTestId("feishu-owner-status")).toContainText("ou_manual");

  // Back on the list the bound bot's toggle is now enabled.
  await page.locator(".settings-head-back").click();
  await expect(page.getByTestId("feishu-enabled")).toBeEnabled();

  // WeChat subpage: QR binding. The 2s poll hits the mock's immediate
  // "confirmed": QR goes away and the bind button flips to unbind.
  await page.getByTestId("weixin-channel-row").click();
  await page.getByTestId("weixin-bind").click();
  await expect(page.getByTestId("weixin-qr")).toBeVisible();
  await expect(page.getByTestId("weixin-unbind")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId("weixin-qr")).toHaveCount(0);

  await page.locator(".settings-head-back").click();
  await expect(page.getByTestId("weixin-enabled")).toBeEnabled();

  await page.getByTestId("weixin-channel-row").click();
  await page.getByTestId("weixin-unbind").click();
  await expect(page.getByTestId("weixin-bind")).toBeVisible({ timeout: 10_000 });

  // StickS3 is the third Remote Access peer. LAN is usable now; Relay is an
  // explicit, disabled future transport instead of being conflated with LAN.
  await page.locator(".settings-head-back").click();
  await page.getByTestId("sticks3-channel-row").click();
  await expect(page.getByTestId("sticks3-channel-card")).toBeVisible();
  await expect(page.locator('input[name="device-bridge-mode"][value="lan"]')).toBeChecked();
  await expect(page.locator('input[name="device-bridge-mode"][value="relay"]')).toBeDisabled();

  // UI validation rejects wildcard binding before invoking Tauri.
  await page.getByTestId("sticks3-bind-ipv4").fill("0.0.0.0");
  await page.getByTestId("sticks3-save").click();
  await expect(page.getByText("0.0.0.0 is not allowed. Choose one specific IPv4 address.")).toBeVisible();
  await expect.poll(async () => (await invokeArgsList(page, "set_device_bridge")).length).toBe(0);

  await page.getByTestId("sticks3-bind-ipv4").fill("127.0.0.1");
  await page.getByTestId("sticks3-port").fill("18766");
  await page.getByTestId("sticks3-enabled-detail").check();
  await expect.poll(() => lastInvokeArgs(page, "set_device_bridge")).toMatchObject({
    enabled: true,
    mode: "lan",
    bindIpv4: "127.0.0.1",
    port: 18766,
  });
  await expect(page.getByTestId("sticks3-detail-state")).toHaveText("Listening");
  await expect(page.getByTestId("sticks3-listening-url")).toContainText("http://127.0.0.1:18766");

  await page.getByTestId("sticks3-rotate-token").click();
  await expect(page.getByText(/previous token stopped working immediately/i)).toBeVisible();
  await expect(page.getByTestId("sticks3-copy-token")).toBeEnabled();
  await page.getByTestId("sticks3-copy-token").click();
  await expect.poll(async () => (await invokeArgsList(page, "get_device_bridge_token")).length).toBe(1);
  await page.getByTestId("sticks3-revoke-token").click();
  await expect(page.getByTestId("sticks3-copy-token")).toBeDisabled();
});

test("StickS3 listener errors remain visible without breaking settings", async ({ page }) => {
  await enterApp(page, "/?mockDeviceBridgeError=1");
  await openSettingsSection(page, "Remote Access");
  await page.getByTestId("sticks3-channel-row").click();
  await page.getByTestId("sticks3-bind-ipv4").fill("127.0.0.1");
  await page.getByTestId("sticks3-port").fill("18766");
  await page.getByTestId("sticks3-enabled-detail").check();

  await expect(page.getByTestId("sticks3-detail-state")).toHaveText("Error");
  await expect(page.getByTestId("sticks3-runtime-error")).toContainText("address already in use");
  await expect(page.getByTestId("sticks3-channel-card")).toBeVisible();
});

test("Ctrl+P imports Codex conversations from local, WSL, or SSH without rescanning", async ({ page }) => {
  await enterApp(page);
  await expect(page.locator(".sidebar").getByRole("button", { name: "Import from Codex" })).toHaveCount(0);
  await page.evaluate(() => {
    (window as any).__mockExecutionContexts.push({
      id: "wsl:Ubuntu-24.04",
      kind: "wsl",
      label: "Ubuntu-24.04",
      config_json: "{\"distro\":\"Ubuntu-24.04\"}",
      capabilities_json: "{}",
      last_probe_at: null,
      last_probe_status: null,
      last_probe_error: null,
      created_at: 1783478400,
      updated_at: 1783478400,
    });
  });
  await page.keyboard.press("Control+p");
  const commandInput = page.locator("#action-palette-input");
  await commandInput.fill("import codex");
  await commandInput.press("Enter");

  const modal = page.locator(".codex-import-modal");
  await expect(modal).toBeVisible();
  const source = modal.getByRole("combobox", { name: "Source" });
  await expect(source.locator("option")).toHaveText([
    "Local",
    "SSH · gpu-server",
    "WSL · Ubuntu-24.04",
  ]);
  await expect.poll(() => lastInvokeArgs(page, "list_codex_sessions"))
    .toMatchObject({ contextId: "local", refresh: false });
  await source.selectOption("wsl:Ubuntu-24.04");
  await expect.poll(() => lastInvokeArgs(page, "list_codex_sessions"))
    .toMatchObject({ contextId: "wsl:Ubuntu-24.04", refresh: false });
  await source.selectOption("local");
  await expect.poll(() => lastInvokeArgs(page, "list_codex_sessions"))
    .toMatchObject({ contextId: "local", refresh: false });
  const scansBeforeRefresh = (await invokeArgsList(page, "list_codex_sessions")).length;
  await modal.getByRole("button", { name: "Refresh" }).click();
  await expect.poll(async () => (await invokeArgsList(page, "list_codex_sessions")).length)
    .toBe(scansBeforeRefresh + 1);
  await expect.poll(() => lastInvokeArgs(page, "list_codex_sessions"))
    .toMatchObject({ contextId: "local", refresh: true });

  // The already-imported rollout renders disabled; the new one is actionable.
  await expect(modal.locator(".codex-import-row.imported").getByRole("button", { name: "Imported" })).toBeDisabled();
  const targetRow = modal
    .locator(".codex-import-row")
    .filter({ hasText: "Fix the renderer crash" });
  await targetRow.locator(".codex-import-main").click();
  await expect(targetRow.locator(".codex-import-main")).toHaveAttribute("aria-expanded", "true");
  await expect(targetRow.locator(".codex-import-preview")).toContainText("It fails after opening a second window.");
  await expect.poll(() => lastInvokeArgs(page, "preview_codex_session"))
    .toMatchObject({ contextId: "local" });

  const scansBeforeImport = (await invokeArgsList(page, "list_codex_sessions")).length;
  await page.evaluate(() => (window as any).__delayNextSessionImport(300));
  await targetRow.getByRole("button", { name: "Import", exact: true }).click();

  await expect(modal.locator(".codex-import-progress")).toContainText("Importing 0 of 1 conversations");
  await expect(modal.locator(".codex-import-progress progress")).not.toHaveAttribute("value");
  await expect(page.locator(".copy-toast")).toHaveText("Synced 1 Codex conversations");
  await expect(modal.locator(".codex-import-progress")).toContainText("Synced 1 Codex conversations");
  await expect(modal.locator(".codex-import-progress progress")).toHaveAttribute("value", "1");
  const [toastZ, overlayZ] = await page.evaluate(() => [
    Number(getComputedStyle(document.querySelector(".copy-toast")!).zIndex),
    Number(getComputedStyle(document.querySelector(".overlay")!).zIndex),
  ]);
  expect(toastZ).toBeGreaterThan(overlayZ);
  await expect(modal.locator(".codex-import-row.imported")).toHaveCount(2);
  await expect(page.locator('.side-folder[data-folder-name="codex"]')).toContainText("1");
  expect((await invokeArgsList(page, "list_codex_sessions")).length).toBe(scansBeforeImport);
  // Nothing left to import, so the bulk action is disabled.
  await expect(modal.getByRole("button", { name: "Import all" })).toBeDisabled();
});

test("Ctrl+P imports paged Claude Code conversations into the claude group", async ({ page }) => {
  await enterApp(page);
  await page.keyboard.press("Control+p");
  const commandInput = page.locator("#action-palette-input");
  await commandInput.fill("import claude");
  await commandInput.press("Enter");

  const modal = page.locator('.codex-import-modal[data-provider="claude"]');
  await expect(modal).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "list_claude_sessions"))
    .toMatchObject({ contextId: "local", refresh: false });
  await expect(modal.locator(".codex-import-row")).toHaveCount(25);
  await expect(modal.locator(".codex-import-pagination")).toContainText("Page 1 of 2");

  await modal.getByRole("button", { name: "Next" }).click();
  await expect(modal.locator(".codex-import-row")).toHaveCount(2);
  await expect(modal.locator(".codex-import-pagination")).toContainText("Page 2 of 2");
  await expect(modal).toContainText("Claude task 26");

  const scansBeforeImport = (await invokeArgsList(page, "list_claude_sessions")).length;
  await modal
    .locator(".codex-import-row")
    .filter({ hasText: "Claude task 26" })
    .getByRole("button", { name: "Import", exact: true })
    .click();

  await expect(page.locator(".copy-toast")).toHaveText("Synced 1 Claude Code conversations");
  await expect(modal.locator(".codex-import-row.imported")).toHaveCount(1);
  await expect(page.locator('.side-folder[data-folder-name="claude"]')).toContainText("1");
  expect((await invokeArgsList(page, "list_claude_sessions")).length).toBe(scansBeforeImport);
});

test("Escape after leaving the projects screen does not touch disposed signals", async ({ page }) => {
  const browserErrors: string[] = [];
  page.on("pageerror", (error) => browserErrors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") browserErrors.push(message.text());
  });
  // Opening a project disposes ProjectsScreen; its window-level Escape listener
  // must go with it, or this keypress reads signals that no longer exist.
  await enterApp(page);
  await page.keyboard.press("Escape");
  await expect(newSessionButton(page)).toBeVisible();
  expect(browserErrors.filter((message) => message.includes("disposed"))).toEqual([]);
});
